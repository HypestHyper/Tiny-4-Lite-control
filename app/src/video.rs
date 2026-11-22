use crate::run_v4l2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoMode {
    pub width: u32,
    pub height: u32,
    pub pixel_format: String,
    pub frame_rate_milli: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoCapability {
    pub width: u32,
    pub height: u32,
    pub pixel_format: String,
    pub frame_rates_milli: Vec<u32>,
}

fn parse_size(line: &str) -> Option<(u32, u32)> {
    let value = line.split_once("Discrete")?.1.trim();
    let (width, height) = value.split_once('x')?;
    Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
}

fn parse_rate(line: &str) -> Option<u32> {
    let start = line.find('(')? + 1;
    let tail = &line[start..];
    let end = tail.find(" fps")?;
    let rate: f64 = tail[..end].trim().parse().ok()?;
    if !rate.is_finite() || rate <= 0.0 { return None; }
    Some((rate * 1000.0).round() as u32)
}

/// Parse v4l2-ctl --list-formats-ext without inventing modes. Only discrete
/// frame sizes and rates advertised by the selected capture node are returned.
pub fn parse_capabilities(output: &str) -> Vec<VideoCapability> {
    let mut result: Vec<VideoCapability> = Vec::new();
    let mut format: Option<String> = None;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.contains("]:") {
            format = trimmed.split('\'').nth(1).map(str::to_string);
        } else if trimmed.starts_with("Size:") {
            if let (Some(pixel_format), Some((width, height))) = (format.as_ref(), parse_size(trimmed)) {
                result.push(VideoCapability {
                    width, height, pixel_format: pixel_format.clone(), frame_rates_milli: Vec::new(),
                });
            }
        } else if trimmed.starts_with("Interval:") {
            if let (Some(mode), Some(rate)) = (result.last_mut(), parse_rate(trimmed)) {
                if !mode.frame_rates_milli.contains(&rate) {
                    mode.frame_rates_milli.push(rate);
                }
            }
        }
    }
    result.retain(|mode| !mode.frame_rates_milli.is_empty());
    result
}

pub fn capabilities(device: &str) -> Result<Vec<VideoCapability>, String> {
    let output = run_v4l2(device, "--list-formats-ext")?;
    let modes = parse_capabilities(&output);
    if modes.is_empty() {
        Err(format!("{device} did not report any discrete capture modes with frame rates."))
    } else {
        Ok(modes)
    }
}

/// The format alone, for a device that does not report a frame rate (the denoised virtual camera).
pub fn current_format(device: &str) -> Option<VideoMode> {
    let format = run_v4l2(device, "--get-fmt-video").ok()?;
    let mut size = None;
    let mut pixel_format = None;
    for line in format.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Width/Height") {
            let (w, h) = trimmed.split_once(':')?.1.trim().split_once('/')?;
            size = Some((w.trim().parse().ok()?, h.trim().parse().ok()?));
        } else if trimmed.starts_with("Pixel Format") {
            pixel_format = trimmed.split('\'').nth(1).map(str::to_string);
        }
    }
    let (width, height) = size?;
    Some(VideoMode { width, height, pixel_format: pixel_format?, frame_rate_milli: 30_000 })
}

pub fn current_mode(device: &str) -> Option<VideoMode> {
    let format = run_v4l2(device, "--get-fmt-video").ok()?;
    let mut width = None;
    let mut height = None;
    let mut pixel_format = None;
    for line in format.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Width/Height") {
            let value = trimmed.split_once(':')?.1.trim();
            let (w, h) = value.split_once('/')?;
            width = w.trim().parse().ok();
            height = h.trim().parse().ok();
        } else if trimmed.starts_with("Pixel Format") {
            pixel_format = trimmed.split('\'').nth(1).map(str::to_string);
        }
    }
    let parm = run_v4l2(device, "--get-parm").ok()?;
    let fps_line = parm.lines().find(|line| line.contains("Frames per second"))?;
    let fps_text = fps_line.split_once(':')?.1.trim().split_whitespace().next()?;
    let fps: f64 = fps_text.parse().ok()?;
    if !fps.is_finite() || fps <= 0.0 { return None; }
    Some(VideoMode {
        width: width?,
        height: height?,
        pixel_format: pixel_format?,
        frame_rate_milli: (fps * 1000.0).round() as u32,
    })
}

/// Apply a mode, then verify what the driver actually negotiated.
/// V4L2 setters may return success while the driver rounds or falls back to
/// another mode, so callers must not treat the requested tuple as active
/// until GET_FMT and G_PARM confirm it.
pub fn set_mode(device: &str, mode: &VideoMode) -> Result<(), String> {
    if mode.width == 0 || mode.height == 0 || mode.frame_rate_milli == 0
        || mode.pixel_format.len() != 4 || !mode.pixel_format.is_ascii() {
        return Err("Invalid video capture mode.".into());
    }

    run_v4l2(device, &format!(
        "--set-fmt-video=width={},height={},pixelformat={}",
        mode.width, mode.height, mode.pixel_format
    ))?;
    let fps = mode.frame_rate_milli as f64 / 1000.0;
    run_v4l2(device, &format!("--set-parm={fps:.3}"))?;

    let actual = current_mode(device)
        .ok_or_else(|| "The driver accepted the mode requests but did not return a readable active format and frame rate.".to_string())?;
    let same_format = actual.width == mode.width
        && actual.height == mode.height
        && actual.pixel_format == mode.pixel_format;
    // Drivers commonly report fractional rates such as 29.970 for nominal
    // 30 fps. Allow a small reporting tolerance, never a format fallback.
    let rate_matches = actual.frame_rate_milli.abs_diff(mode.frame_rate_milli) <= 500;
    if !same_format || !rate_matches {
        return Err(format!(
            "The camera did not retain the requested mode. Requested {}×{} {} {:.3} fps; driver reports {}×{} {} {:.3} fps. Another application may own the stream, or the driver may have rejected the combination.",
            mode.width, mode.height, mode.pixel_format, fps,
            actual.width, actual.height, actual.pixel_format,
            actual.frame_rate_milli as f64 / 1000.0
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_discrete_formats_sizes_and_frame_rates() {
        let output = r#"
ioctl: VIDIOC_ENUM_FMT
 Type: Video Capture
 [0]: 'MJPG' (Motion-JPEG, compressed)
   Size: Discrete 1920x1080
     Interval: Discrete 0.033s (30.000 fps)
     Interval: Discrete 0.017s (60.000 fps)
   Size: Discrete 1280x720
     Interval: Discrete 0.033s (30.000 fps)
 [1]: 'YUYV' (YUYV 4:2:2)
   Size: Discrete 640x480
     Interval: Discrete 0.033s (30.000 fps)
"#;
        let modes = parse_capabilities(output);
        assert_eq!(modes.len(), 3);
        assert_eq!(modes[0].pixel_format, "MJPG");
        assert_eq!((modes[0].width, modes[0].height), (1920, 1080));
        assert_eq!(modes[0].frame_rates_milli, vec![30000, 60000]);
        assert_eq!(modes[2].pixel_format, "YUYV");
        assert_eq!(modes[2].frame_rates_milli, vec![30000]);
    }

    #[test]
    fn rejects_modes_without_discrete_frame_rate_metadata() {
        let output = "[0]: 'MJPG' (Motion-JPEG)\n Size: Discrete 1920x1080\n";
        assert!(parse_capabilities(output).is_empty());
    }
}
