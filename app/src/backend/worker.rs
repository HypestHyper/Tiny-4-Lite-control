//! A single worker thread that owns every camera transaction.
//!
//! Why: the camera must be talked to one command at a time, and slow reads
//! (the OBSBOT query protocol polls for a reply) must never block the UI.
//! Jobs run in order. A *keyed* job replaces any not-yet-started job with the
//! same key, so a slider or joystick that fires faster than the camera
//! answers only ever sends the newest value. An *urgent* job jumps the queue
//! (used to stop the gimbal).

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

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
        Camera { path, shared }
    }

    pub fn path(&self) -> &str { &self.path }

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
    use std::time::Duration;

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
