//! A single worker thread that owns every camera transaction.
//!
//! Why: the camera must be talked to one command at a time, and slow reads
//! (the OBSBOT query protocol polls for a reply) must never block the UI.
//! Jobs run in order. A *keyed* job replaces any not-yet-started job with the
//! same key, so a slider or joystick that fires faster than the camera
//! answers only ever sends the newest value. An *urgent* job jumps the queue
//! (used to stop the gimbal).

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

/// While the gimbal is being driven, whoever drives it renews a lease this often. If the lease runs
/// out (the window froze, the app was suspended) the gimbal is stopped from here.
pub const MOTION_LEASE: Duration = Duration::from_millis(700);

type Run = Box<dyn FnOnce(&str) + Send>;

struct Job {
    key: Option<String>,
    run: Run,
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    closed: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
}

#[derive(Clone)]
pub struct Camera {
    path: Arc<String>,
    shared: Arc<Shared>,
    /// When the current movement command stops being trusted, if one is running.
    lease: Arc<Mutex<Option<Instant>>>,
}

impl Camera {
    pub fn spawn(path: &str) -> Camera {
        let shared = Arc::new(Shared { queue: Mutex::new(Queue::default()), wake: Condvar::new() });
        let path = Arc::new(path.to_string());
        let worker_shared = shared.clone();
        let worker_path = path.clone();
        thread::Builder::new()
            .name("camera-io".into())
            .spawn(move || loop {
                let job = {
                    let mut queue = worker_shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                    loop {
                        if let Some(job) = queue.jobs.pop_front() { break job; }
                        if queue.closed { return; }
                        queue = worker_shared.wake.wait(queue).unwrap_or_else(|e| e.into_inner());
                    }
                };
                (job.run)(&worker_path);
            })
            .expect("camera worker thread can start");
        let lease: Arc<Mutex<Option<Instant>>> = Arc::default();
        if !path.is_empty() { Camera::watch_motion(Arc::downgrade(&shared), lease.clone()); }
        Camera { path, shared, lease }
    }

    /// Stop the gimbal if a movement lease runs out. Ends when the camera is dropped.
    fn watch_motion(shared: Weak<Shared>, lease: Arc<Mutex<Option<Instant>>>) {
        let _ = thread::Builder::new().name("camera-deadman".into()).spawn(move || loop {
            thread::sleep(Duration::from_millis(100));
            let Some(shared) = shared.upgrade() else { return };
            let expired = {
                let mut lease = lease.lock().unwrap_or_else(|e| e.into_inner());
                match *lease {
                    Some(until) if Instant::now() >= until => { *lease = None; true }
                    _ => false,
                }
            };
            if expired {
                eprintln!("The gimbal command was not renewed; stopping the gimbal.");
                let job = Job { key: Some("gimbal-speed".into()), run: Box::new(|device| {
                    for _ in 0..3 {
                        if crate::backend::run_v4l2(device, "--set-ctrl=pan_speed=0,tilt_speed=0").is_ok() { break; }
                        thread::sleep(Duration::from_millis(60));
                    }
                }) };
                let mut queue = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                queue.jobs.retain(|j| j.key.as_deref() != Some("gimbal-speed"));
                queue.jobs.push_front(job);
                shared.wake.notify_one();
            }
        });
    }

    /// Say the gimbal is being driven and renew the lease. Call this on every tick while it moves.
    pub fn hold_motion(&self) {
        *self.lease.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now() + MOTION_LEASE);
    }

    /// The gimbal was stopped on purpose: nothing to watch any more.
    pub fn release_motion(&self) {
        *self.lease.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub fn path(&self) -> &str { &self.path }

    /// This connection is being replaced: the worker finishes what is already queued (a stop for the gimbal,
    /// say) and then ends, instead of waiting for work forever.
    pub fn close(&self) {
        self.shared.queue.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        self.shared.wake.notify_all();
    }

    /// Queue a job behind everything already waiting.
    pub fn submit(&self, run: impl FnOnce(&str) + Send + 'static) {
        self.push(None, false, Box::new(run));
    }

    /// Queue a job, replacing any waiting job with the same key.
    pub fn submit_keyed(&self, key: &str, run: impl FnOnce(&str) + Send + 'static) {
        self.push(Some(key.to_string()), false, Box::new(run));
    }

    /// Run next, and discard waiting jobs that share the key.
    pub fn submit_urgent(&self, key: &str, run: impl FnOnce(&str) + Send + 'static) {
        self.push(Some(key.to_string()), true, Box::new(run));
    }

    fn push(&self, key: Option<String>, urgent: bool, run: Run) {
        let mut queue = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(key) = key.as_deref() {
            queue.jobs.retain(|job| job.key.as_deref() != Some(key));
        }
        let job = Job { key, run };
        if urgent { queue.jobs.push_front(job); } else { queue.jobs.push_back(job); }
        self.shared.wake.notify_one();
    }

    /// Number of waiting jobs, for tests.
    #[cfg(test)]
    pub fn pending(&self) -> usize {
        self.shared.queue.lock().unwrap_or_else(|e| e.into_inner()).jobs.len()
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        if let Ok(mut queue) = self.queue.lock() { queue.closed = true; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_movement_lease_that_runs_out_is_cleared_by_the_watchdog() {
        let camera = Camera::spawn("/dev/null");
        camera.hold_motion();
        assert!(camera.lease.lock().unwrap().is_some());
        std::thread::sleep(MOTION_LEASE + Duration::from_millis(500));
        assert!(camera.lease.lock().unwrap().is_none(), "the lease should have expired and been handled");
        // Renewing keeps it alive, and releasing ends it at once.
        camera.hold_motion();
        std::thread::sleep(Duration::from_millis(300));
        camera.hold_motion();
        assert!(camera.lease.lock().unwrap().is_some());
        camera.release_motion();
        assert!(camera.lease.lock().unwrap().is_none());
    }

    #[test]
    fn a_closed_worker_finishes_its_queue_and_ends() {
        let camera = Camera::spawn("/dev/null");
        let (out_tx, out_rx) = mpsc::channel::<i32>();
        camera.submit(move |_| { let _ = out_tx.send(1); });
        camera.close();
        assert_eq!(out_rx.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
        // The worker has gone: the queue's only other owner is this handle.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while Arc::strong_count(&camera.shared) > 1 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(Arc::strong_count(&camera.shared), 1);
    }

    #[test]
    fn keyed_jobs_keep_only_the_newest_waiting_value() {
        let camera = Camera::spawn("/dev/null");
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let (out_tx, out_rx) = mpsc::channel::<i32>();
        // Block the worker so the following jobs queue up.
        let (started_tx, started_rx) = mpsc::channel::<()>();
        camera.submit(move |_| { let _ = started_tx.send(()); let _ = gate_rx.recv(); });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        for value in 1..=5 {
            let out = out_tx.clone();
            camera.submit_keyed("speed", move |_| { let _ = out.send(value); });
        }
        assert_eq!(camera.pending(), 1);
        gate_tx.send(()).unwrap();
        assert_eq!(out_rx.recv_timeout(Duration::from_secs(2)).unwrap(), 5);
        assert!(out_rx.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[test]
    fn urgent_job_runs_before_waiting_work_and_drops_stale_keyed_work() {
        let camera = Camera::spawn("/dev/null");
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let (out_tx, out_rx) = mpsc::channel::<&'static str>();
        let (started_tx, started_rx) = mpsc::channel::<()>();
        camera.submit(move |_| { let _ = started_tx.send(()); let _ = gate_rx.recv(); });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let out = out_tx.clone();
        camera.submit(move |_| { let _ = out.send("other"); });
        let out = out_tx.clone();
        camera.submit_keyed("speed", move |_| { let _ = out.send("stale-speed"); });
        let out = out_tx.clone();
        camera.submit_urgent("speed", move |_| { let _ = out.send("stop"); });
        gate_tx.send(()).unwrap();
        assert_eq!(out_rx.recv_timeout(Duration::from_secs(2)).unwrap(), "stop");
        assert_eq!(out_rx.recv_timeout(Duration::from_secs(2)).unwrap(), "other");
        assert!(out_rx.recv_timeout(Duration::from_millis(100)).is_err());
    }
}
