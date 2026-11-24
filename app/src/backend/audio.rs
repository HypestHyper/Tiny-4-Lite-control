//! The camera's microphone as the computer sees it: whether its sound card is there (the camera removes it when
//! the microphone is switched off), and a level meter while it is on.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};

/// OBSBOT's USB vendor id.
const VENDOR: &str = "3564:";

/// The camera's ALSA card: its number and id, if the computer has it.
pub fn camera_card() -> Option<(usize, String)> {
    if super::ui_test_mode() {
        return std::env::var("TINY4LITE_UI_TEST_NO_MIC").is_err().then(|| (1, "Lite".to_string()));
    }
    let mut cards: Vec<(usize, String)> = std::fs::read_dir("/proc/asound").ok()?.flatten().filter_map(|entry| {
        let number = entry.file_name().to_string_lossy().strip_prefix("card")?.parse::<usize>().ok()?;
        let usbid = std::fs::read_to_string(entry.path().join("usbid")).ok()?;
        if !usbid.trim().to_lowercase().starts_with(VENDOR) { return None; }
        let id = std::fs::read_to_string(entry.path().join("id")).unwrap_or_default().trim().to_string();
        Some((number, id))
    }).collect();
    cards.sort();
    cards.into_iter().next()
}

/// The camera's microphone source in a `pactl list short sources` listing (PipeWire or PulseAudio).
pub fn source_from_pactl(listing: &str) -> Option<String> {
    listing.lines().filter_map(|line| line.split('\t').nth(1))
        .find(|name| name.starts_with("alsa_input.") && { let n = name.to_lowercase(); n.contains("obsbot") || n.contains("remo_tech") })
        .map(str::to_string)
}

/// How loud a block of 16-bit samples is, from 0 (silence, −60 dB or quieter) to 1 (full scale), by its peak.
pub fn level(samples: &[u8]) -> f64 {
    let peak = samples.chunks_exact(2).map(|s| i32::from(i16::from_le_bytes([s[0], s[1]])).unsigned_abs()).max().unwrap_or(0);
    if peak == 0 { return 0.0; }
    let db = 20.0 * (f64::from(peak) / 32768.0).log10();
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

/// A running meter: the recorder (if a real one) and the levels it measures, about 25 times a second.
pub struct Meter { child: Option<Child>, pub levels: Receiver<f64> }

impl Drop for Meter {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() { let _ = child.kill(); let _ = child.wait(); }
    }
}

/// Start measuring the camera's microphone. The sound is read and measured here; nothing is kept.
pub fn start_meter() -> Option<Meter> {
    let (sender, levels) = channel();
    if super::ui_test_mode() {
        std::thread::spawn(move || {
            for i in 0u32.. {
                std::thread::sleep(std::time::Duration::from_millis(40));
                if sender.send(0.45 + 0.35 * (f64::from(i) / 6.0).sin().abs()).is_err() { break; }
            }
        });
        return Some(Meter { child: None, levels });
    }
    let (_, card) = camera_card()?;
    // Through PipeWire or PulseAudio when there is one (it shares the microphone with other apps), else ALSA.
    let source = Command::new("pactl").args(["list", "short", "sources"]).output().ok()
        .filter(|o| o.status.success()).and_then(|o| source_from_pactl(&String::from_utf8_lossy(&o.stdout)));
    let mut command = match source {
        Some(name) => { let mut c = Command::new("parec"); c.args(["-d", &name, "--raw", "--format=s16le", "--rate=16000", "--channels=1", "--latency-msec=40"]); c }
        None => { let mut c = Command::new("arecord"); c.args(["-q", "-D", &format!("plughw:CARD={card}"), "-f", "S16_LE", "-r", "16000", "-c", "1", "-t", "raw"]); c }
    };
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut out = child.stdout.take()?;
    std::thread::spawn(move || {
        let mut block = [0u8; 1280]; // 40 ms at 16 kHz
        while out.read_exact(&mut block).is_ok() {
            if sender.send(level(&block)).is_err() { break; }
        }
    });
    Some(Meter { child: Some(child), levels })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_source_is_found_in_a_pactl_listing() {
        let listing = "52\talsa_output.pci-0000_00_1f.3.analog-stereo.monitor\tPipeWire\ts32le 2ch 48000Hz\tSUSPENDED\n\
                       61\talsa_input.usb-Remo_Tech_Co.__Ltd._OBSBOT_Tiny_2_Lite_StreamCamera_ABC-02.mono-fallback\tPipeWire\ts16le 1ch 48000Hz\tSUSPENDED\n";
        assert_eq!(source_from_pactl(listing).as_deref(), Some("alsa_input.usb-Remo_Tech_Co.__Ltd._OBSBOT_Tiny_2_Lite_StreamCamera_ABC-02.mono-fallback"));
        assert_eq!(source_from_pactl("1\talsa_input.pci-0000.analog\tPipeWire\n"), None);
    }

    #[test]
    fn levels_run_from_silence_to_full_scale() {
        assert_eq!(level(&[0, 0, 0, 0]), 0.0);
        assert!(level(&i16::MAX.to_le_bytes()) > 0.999);
        let quiet = (33i16).to_le_bytes(); // about −60 dB
        assert!(level(&quiet) < 0.02);
        let half = (3277i16).to_le_bytes(); // −20 dB
        assert!((level(&half) - 0.667).abs() < 0.01);
    }
}
