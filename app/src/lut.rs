//! LUTs for the Studio camera: a `.cube` colour look-up table applied to the picture.
//!
//! A chosen file is checked and copied into the app's settings folder, so moving or deleting the original
//! does not break it. Strength is applied by blending the table itself towards "no change" (written to a
//! small file in the runtime folder), so it costs nothing extra while the picture runs.
//!
//! The table runs on the graphics chip through ffmpeg's `libplacebo` filter (Vulkan) when that works here,
//! tried first on a small made-up picture; otherwise on the processor with `lut3d`, which is heavy (it has
//! to turn every frame into RGB and back).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A 3D table from a `.cube` file.
#[derive(Clone, Debug, PartialEq)]
pub struct Cube {
    pub size: usize,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    /// size³ entries, red changing fastest, as in the file.
    pub values: Vec<[f32; 3]>,
}

/// Read a `.cube` file (the usual format for colour-grading LUTs). Only 3D tables.
pub fn parse_cube(text: &str) -> Result<Cube, String> {
    let mut size = None;
    let (mut domain_min, mut domain_max) = ([0.0f32; 3], [1.0f32; 3]);
    let mut values = Vec::new();
    let three = |rest: &str| -> Option<[f32; 3]> {
        let v: Vec<f32> = rest.split_whitespace().filter_map(|t| t.parse().ok()).collect();
        (v.len() == 3).then(|| [v[0], v[1], v[2]])
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        match word {
            "TITLE" => {}
            "LUT_1D_SIZE" => return Err("This is a 1D LUT; only 3D LUTs (LUT_3D_SIZE) are supported.".into()),
            "LUT_3D_SIZE" => size = Some(rest.trim().parse::<usize>().map_err(|_| format!("Bad LUT_3D_SIZE: {rest}"))?),
            "DOMAIN_MIN" => domain_min = three(rest).ok_or("Bad DOMAIN_MIN line.")?,
            "DOMAIN_MAX" => domain_max = three(rest).ok_or("Bad DOMAIN_MAX line.")?,
            _ => match three(line) {
                Some(v) => values.push(v),
                // Other keywords some programs write are skipped.
                None if word.chars().all(|c| c.is_ascii_uppercase() || c == '_') => {}
                None => return Err(format!("Cannot read this line: {line}")),
            },
        }
    }
    let size = size.ok_or("No LUT_3D_SIZE line: is this a .cube LUT?")?;
    if !(2..=256).contains(&size) { return Err(format!("LUT size {size} is out of range.")); }
    if values.len() != size * size * size {
        return Err(format!("The table has {} entries; a {size}-point LUT needs {}.", values.len(), size * size * size));
    }
    Ok(Cube { size, domain_min, domain_max, values })
}

/// The table blended towards "no change": 100 is the LUT as it is, 0 leaves the picture alone.
pub fn blend(cube: &Cube, strength: u8) -> Cube {
    let k = f32::from(strength.min(100)) / 100.0;
    let n = cube.size;
    let step = |axis: usize, i: usize| cube.domain_min[axis] + (cube.domain_max[axis] - cube.domain_min[axis]) * i as f32 / (n - 1) as f32;
    let values = cube.values.iter().enumerate().map(|(index, v)| {
        let (r, g, b) = (index % n, (index / n) % n, index / (n * n));
        let same = [step(0, r), step(1, g), step(2, b)];
        [same[0] + (v[0] - same[0]) * k, same[1] + (v[1] - same[1]) * k, same[2] + (v[2] - same[2]) * k]
    }).collect();
    Cube { values, ..cube.clone() }
}

pub fn write_cube(cube: &Cube) -> String {
    let mut text = format!("LUT_3D_SIZE {}\n", cube.size);
    if cube.domain_min != [0.0; 3] || cube.domain_max != [1.0; 3] {
        text.push_str(&format!("DOMAIN_MIN {} {} {}\nDOMAIN_MAX {} {} {}\n", cube.domain_min[0], cube.domain_min[1], cube.domain_min[2], cube.domain_max[0], cube.domain_max[1], cube.domain_max[2]));
    }
    for v in &cube.values { text.push_str(&format!("{:.6} {:.6} {:.6}\n", v[0], v[1], v[2])); }
    text
}

/// A table that changes nothing, for testing the routes.
pub fn identity(size: usize) -> Cube {
    let values = (0..size * size * size).map(|i| {
        let f = |x: usize| x as f32 / (size - 1) as f32;
        [f(i % size), f((i / size) % size), f(i / (size * size))]
    }).collect();
    Cube { size, domain_min: [0.0; 3], domain_max: [1.0; 3], values }
}

// ── Files ─────────────────────────────────────────────────────────────────

/// Where chosen LUTs are kept: next to the settings.
pub fn lut_dir() -> PathBuf {
    crate::config::config_path().parent().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir).join("luts")
}

/// Check a chosen file and copy it in; returns the name it is kept under.
pub fn import(path: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    parse_cube(&text)?;
    let name = path.file_name().and_then(|n| n.to_str()).ok_or("The file has no usable name.")?.to_string();
    let dir = lut_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not make {}: {e}", dir.display()))?;
    std::fs::write(dir.join(&name), text).map_err(|e| format!("Could not keep a copy: {e}"))?;
    Ok(name)
}

/// The LUTs added so far, by name, in order.
pub fn list() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(lut_dir()).map(|entries| entries.flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.to_lowercase().ends_with(".cube")).collect()).unwrap_or_default();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Take a LUT out of the list (its kept copy; the original file is left alone).
pub fn remove(name: &str) -> Result<(), String> {
    if name.contains('/') { return Err("Not a LUT name.".into()); }
    std::fs::remove_file(lut_dir().join(name)).map_err(|e| format!("Could not remove {name}: {e}"))
}

/// The kept LUT blended to `strength`, written where ffmpeg can read it; the path (with nothing in it that
/// a filter graph would have to escape).
pub fn prepared(name: &str, strength: u8) -> Result<PathBuf, String> {
    let text = std::fs::read_to_string(lut_dir().join(name)).map_err(|e| format!("The LUT {name} is missing: {e}"))?;
    let cube = blend(&parse_cube(&text)?, strength);
    let dir = crate::service::runtime_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // A new name for each strength, so a running ffmpeg is restarted onto the new table.
    let mut hash = 1469598103934665603u64;
    for b in name.bytes().chain([strength]) { hash = (hash ^ u64::from(b)).wrapping_mul(1099511628211); }
    let path = dir.join(format!("studio-lut-{hash:016x}.cube"));
    if !path.exists() { std::fs::write(&path, write_cube(&cube)).map_err(|e| e.to_string())?; }
    Ok(path)
}

// ── Running it ────────────────────────────────────────────────────────────

/// Where the table is applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Route { Gpu, Cpu }

impl Route {
    pub fn name(self) -> &'static str { match self { Route::Gpu => "graphics chip", Route::Cpu => "processor" } }
}

/// Setting up the graphics chip for the GPU route.
pub fn hw_args(route: Route) -> Vec<String> {
    match route {
        Route::Gpu => vec!["-init_hw_device".into(), "vulkan=vk".into(), "-filter_hw_device".into(), "vk".into()],
        Route::Cpu => Vec::new(),
    }
}

/// The filter that applies the table at `path`, ending in the Studio camera's format.
pub fn filter(route: Route, path: &Path, out: &str) -> String {
    let path = path.to_string_lossy();
    match route {
        // libplacebo converts, looks up and converts back on the graphics chip.
        Route::Gpu => format!("libplacebo=lut={path}:lut_type=normalized:format=yuv422p,format={out}"),
        Route::Cpu => format!("lut3d=file={path}:interp=tetrahedral,format={out}"),
    }
}

/// Run ffmpeg on a small made-up picture with `filter`, and give back the average brightness (Y, 0–255) of
/// what comes out, or what ffmpeg said.
fn test_brightness(route: Route, filter: &str) -> Result<f64, String> {
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostdin"].iter().map(|s| s.to_string()).collect();
    args.extend(hw_args(route));
    // A made-up picture as the camera sends it: MJPEG, full range.
    args.extend(["-f".into(), "lavfi".into(), "-i".into(), "testsrc2=s=640x360:r=30,format=yuvj422p".into(), "-frames:v".into(), "4".into(),
        "-vf".into(), filter.into(), "-f".into(), "rawvideo".into(), "-pix_fmt".into(), "yuyv422".into(), "-".into()]);
    let mut child = Command::new("ffmpeg").args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
        .map_err(|e| format!("ffmpeg could not start: {e}"))?;
    // Read the picture on another thread, so a full pipe never stops ffmpeg.
    let mut out = child.stdout.take().ok_or("no output")?;
    let reader = std::thread::spawn(move || { use std::io::Read; let mut data = Vec::new(); let _ = out.read_to_end(&mut data); data });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > Duration::from_secs(15) => { let _ = child.kill(); let _ = child.wait(); return Err("it took too long".into()); }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let data = reader.join().unwrap_or_default();
    if !status.success() {
        let mut said = String::new();
        if let Some(mut err) = child.stderr.take() { use std::io::Read; let _ = err.read_to_string(&mut said); }
        let said: Vec<&str> = said.lines().filter(|l| !l.trim().is_empty()).collect();
        let said = said[said.len().saturating_sub(2)..].join(" ");
        return Err(if said.is_empty() { format!("ffmpeg stopped ({status})") } else { said });
    }
    let luma: Vec<u8> = data.iter().step_by(2).copied().collect();
    if luma.is_empty() { return Err("no picture came out".into()); }
    Ok(luma.iter().map(|v| f64::from(*v)).sum::<f64>() / luma.len() as f64)
}

/// Try the graphics-chip route on a small made-up picture, and check it is faithful: a table that changes
/// nothing must leave the brightness as it is (a route that gets the video range or the gamma wrong makes
/// everything darker or lighter). Works, or why not.
pub fn probe_gpu() -> Result<(), String> {
    let dir = crate::service::runtime_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let test = dir.join("studio-lut-test.cube");
    std::fs::write(&test, write_cube(&identity(17))).map_err(|e| e.to_string())?;
    let plain = test_brightness(Route::Cpu, "format=yuyv422")?;
    let through = test_brightness(Route::Gpu, &filter(Route::Gpu, &test, "yuyv422"))?;
    let change = (through - plain) / plain.max(1.0) * 100.0;
    if change.abs() > 3.0 {
        return Err(format!("it changes the brightness by itself (a no-change LUT made a test picture {:.0}% {})", change.abs(), if change < 0.0 { "darker" } else { "lighter" }));
    }
    Ok(())
}

/// Whether what ffmpeg said when it stopped is about the graphics chip (so the LUT should move to the
/// processor), rather than something else that would stop either route.
pub fn blames_gpu(said: &str) -> bool {
    let said = said.to_lowercase();
    ["vulkan", "libplacebo", "placebo", "hwupload", "hwdownload", "hw_device", "hwdevice", "hardware device", "vk_", "gpu"].iter().any(|k| said.contains(k))
}

/// Whether a LUT looks made for flat log footage (from a cinema camera) rather than a normal picture, by
/// its name and title. Such a LUT makes a webcam picture darker and more contrasty.
pub fn looks_like_log(name: &str) -> bool {
    let title = std::fs::read_to_string(lut_dir().join(name)).ok()
        .and_then(|t| t.lines().find(|l| l.trim_start().starts_with("TITLE")).map(str::to_string)).unwrap_or_default();
    let text = format!("{name} {title}").to_lowercase().replace(['-', '_', ' ', '.'], "");
    ["slog", "vlog", "clog", "flog", "nlog", "dlog", "logc", "log3g10", "redlog", "bmdfilm", "cinelike", "logto709", "log2rec709", "logtorec709", "log709", "hlg"].iter().any(|k| text.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cube_file_is_read_and_checked() {
        let text = "TITLE \"warm\"\n# a comment\nLUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let cube = parse_cube(text).unwrap();
        assert_eq!((cube.size, cube.values.len()), (2, 8));
        assert_eq!(cube, identity(2));
        assert!(parse_cube("LUT_3D_SIZE 2\n0 0 0\n").unwrap_err().contains("needs 8"));
        assert!(parse_cube("LUT_1D_SIZE 4\n").unwrap_err().contains("1D"));
        assert!(parse_cube("0 0 0\n").is_err());
    }

    #[test]
    fn strength_blends_towards_no_change() {
        let mut warm = identity(3);
        for v in &mut warm.values { v[0] = (v[0] + 0.2).min(1.0); }
        assert_eq!(blend(&warm, 0), identity(3));
        assert_eq!(blend(&warm, 100), warm);
        let half = blend(&warm, 50);
        assert!((half.values[0][0] - 0.1).abs() < 1e-6, "{:?}", half.values[0]);
        // Written and read back the same.
        assert_eq!(parse_cube(&write_cube(&half)).unwrap().size, 3);
    }

    #[test]
    fn the_processor_route_keeps_the_brightness_with_a_no_change_table() {
        // ffmpeg is there on the machines the tests run on; skip quietly where it is not.
        if !crate::setup::program_found("ffmpeg") { return; }
        let dir = std::env::temp_dir().join(format!("t4-lut-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let test = dir.join("ident.cube");
        std::fs::write(&test, write_cube(&identity(17))).unwrap();
        let plain = test_brightness(Route::Cpu, "format=yuyv422").unwrap();
        let through = test_brightness(Route::Cpu, &filter(Route::Cpu, &test, "yuyv422")).unwrap();
        assert!((through - plain).abs() / plain < 0.02, "{plain} -> {through}");
    }

    #[test]
    fn only_a_graphics_chip_problem_moves_the_lut_to_the_processor() {
        assert!(blames_gpu("[libplacebo @ 0x55] Failed initializing vulkan device"));
        assert!(!blames_gpu("Option thread_queue_size (set the maximum number of queued packets per stream on the muxer) cannot be applied to input url /dev/video0"));
        assert!(!blames_gpu("/dev/video0: Device or resource busy"));
    }

    #[test]
    fn log_luts_are_recognised_by_name() {
        assert!(looks_like_log("Sony S-Log3 to Rec709.cube"));
        assert!(looks_like_log("ARRI_LogC_to_709.cube"));
        assert!(!looks_like_log("Teal and orange.cube"));
        assert!(!looks_like_log("Warm film.cube"));
    }

    #[test]
    fn the_filters_apply_the_table_and_end_in_the_studio_format() {
        let path = Path::new("/run/user/1000/tiny4lite-control/studio-lut-1.cube");
        assert_eq!(filter(Route::Cpu, path, "yuyv422"), "lut3d=file=/run/user/1000/tiny4lite-control/studio-lut-1.cube:interp=tetrahedral,format=yuyv422");
        assert!(filter(Route::Gpu, path, "yuyv422").starts_with("libplacebo=lut=/run/user/1000/tiny4lite-control/studio-lut-1.cube:lut_type=normalized"));
        assert_eq!(hw_args(Route::Cpu), Vec::<String>::new());
    }
}
