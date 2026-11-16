//! Visual regression (`assertScreenshot`): compare a screenshot with a stored baseline,
//! pixel by pixel. The first run (or `update: true`, or `FASTPILOT_UPDATE_BASELINES=1`) stores
//! the baseline; later runs fail when more than `maxDiff` of the pixels differ by more than
//! `threshold`, and write a diff image (changed pixels in red over a faded actual) next to it.
//! Files: `.fastpilot/baselines/<name>.png` (or `FASTPILOT_DIR`), `<name>.actual.png`,
//! `<name>.diff.png`.

use std::path::PathBuf;

use image::{ImageFormat, Rgba, RgbaImage};

/// Defaults: a pixel counts as changed when a channel moves by more than 10%; the step fails
/// when more than 0.2% of the pixels changed (anti-aliasing, caret).
pub const THRESHOLD: f64 = 0.1;
pub const MAX_DIFF: f64 = 0.002;

pub fn dir() -> PathBuf {
    std::env::var_os("FASTPILOT_DIR").map(PathBuf::from).unwrap_or_else(|| ".fastpilot".into()).join("baselines")
}

/// A region to leave out, in image pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mask {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Baseline written (first run or update).
    Stored(PathBuf),
    /// Within the limit: fraction of changed pixels.
    Same(f64),
    /// Over the limit: fraction, limit, diff image.
    Changed { fraction: f64, limit: f64, diff: PathBuf },
}

fn masked(masks: &[Mask], x: u32, y: u32) -> bool {
    masks.iter().any(|m| x >= m.x && x < m.x + m.w && y >= m.y && y < m.y + m.h)
}

/// Compare `shot` (any image format) with baseline `name`.
pub fn compare(name: &str, shot: &[u8], masks: &[Mask], threshold: f64, max_diff: f64, update: bool) -> Result<Outcome, String> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) || name.contains("..") {
        return Err(format!("baseline name `{name}`: use letters, digits, '-', '_' and '.'"));
    }
    let actual = image::load_from_memory(shot).map_err(|e| format!("screenshot: {e}"))?.to_rgba8();
    let d = dir();
    std::fs::create_dir_all(&d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    let base_path = d.join(format!("{name}.png"));
    let update = update || std::env::var("FASTPILOT_UPDATE_BASELINES").is_ok_and(|v| v == "1");
    let save = |img: &RgbaImage, p: &PathBuf| img.save_with_format(p, ImageFormat::Png).map_err(|e| format!("cannot write {}: {e}", p.display()));
    if update || !base_path.exists() {
        save(&actual, &base_path)?;
        return Ok(Outcome::Stored(base_path));
    }
    let base = image::open(&base_path).map_err(|e| format!("baseline {}: {e}", base_path.display()))?.to_rgba8();
    let diff_path = d.join(format!("{name}.diff.png"));
    if base.dimensions() != actual.dimensions() {
        save(&actual, &d.join(format!("{name}.actual.png")))?;
        let (bw, bh) = base.dimensions();
        let (aw, ah) = actual.dimensions();
        return Err(format!(
            "screenshot is {aw}x{ah}, baseline {name} is {bw}x{bh} (another viewport or device?); \
             rerun with update: true to replace it"
        ));
    }
    let (w, h) = actual.dimensions();
    let limit = (threshold.clamp(0.0, 1.0) * 255.0) as i32;
    let mut diff = RgbaImage::new(w, h);
    let (mut changed, mut counted) = (0u64, 0u64);
    for (x, y, a) in actual.enumerate_pixels() {
        if masked(masks, x, y) {
            diff.put_pixel(x, y, Rgba([0, 0, 255, 60]));
            continue;
        }
        counted += 1;
        let b = base.get_pixel(x, y);
        let moved = (0..3).map(|c| (i32::from(a[c]) - i32::from(b[c])).abs()).max().unwrap_or(0);
        if moved > limit {
            changed += 1;
            diff.put_pixel(x, y, Rgba([255, 0, 0, 255]));
        } else {
            let gray = ((u32::from(a[0]) + u32::from(a[1]) + u32::from(a[2])) / 3) as u8;
            diff.put_pixel(x, y, Rgba([gray, gray, gray, 64]));
        }
    }
    let fraction = if counted == 0 { 0.0 } else { changed as f64 / counted as f64 };
    if fraction <= max_diff {
        let _ = std::fs::remove_file(&diff_path);
        let _ = std::fs::remove_file(d.join(format!("{name}.actual.png")));
        return Ok(Outcome::Same(fraction));
    }
    save(&diff, &diff_path)?;
    save(&actual, &d.join(format!("{name}.actual.png")))?;
    Ok(Outcome::Changed { fraction, limit: max_diff, diff: diff_path })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(f: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let img = RgbaImage::from_fn(100, 50, |x, y| {
            let [r, g, b] = f(x, y);
            Rgba([r, g, b, 255])
        });
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).expect("png");
        out.into_inner()
    }

    #[test]
    fn stores_compares_masks_and_updates() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let d = std::env::temp_dir().join(format!("fastpilot-visual-{}", std::process::id()));
        // SAFETY: test-only env mutation; this is the only test in the crate that reads it.
        unsafe { std::env::set_var("FASTPILOT_DIR", &d) };
        let white = png(|_, _| [255, 255, 255]);
        assert!(matches!(compare("home", &white, &[], THRESHOLD, MAX_DIFF, false), Ok(Outcome::Stored(_))));
        assert_eq!(compare("home", &white, &[], THRESHOLD, MAX_DIFF, false), Ok(Outcome::Same(0.0)));
        // A faint change (below the per-pixel threshold) is the same.
        assert_eq!(compare("home", &png(|_, _| [250, 250, 250]), &[], THRESHOLD, MAX_DIFF, false), Ok(Outcome::Same(0.0)));
        // A red box of 10x10 = 2% of the pixels.
        let boxed = png(|x, y| if x < 10 && y < 10 { [255, 0, 0] } else { [255, 255, 255] });
        match compare("home", &boxed, &[], THRESHOLD, MAX_DIFF, false) {
            Ok(Outcome::Changed { fraction, diff, .. }) => {
                assert!((fraction - 0.02).abs() < 1e-9, "{fraction}");
                assert!(diff.exists() && d.join("baselines/home.actual.png").exists());
            }
            other => panic!("{other:?}"),
        }
        // Masked away, or a generous limit: same; the diff files are cleaned up.
        assert!(matches!(compare("home", &boxed, &[Mask { x: 0, y: 0, w: 10, h: 10 }], THRESHOLD, MAX_DIFF, false), Ok(Outcome::Same(_))));
        assert!(!d.join("baselines/home.diff.png").exists());
        assert!(matches!(compare("home", &boxed, &[], THRESHOLD, 0.05, false), Ok(Outcome::Same(_))));
        // Update replaces the baseline.
        assert!(matches!(compare("home", &boxed, &[], THRESHOLD, MAX_DIFF, true), Ok(Outcome::Stored(_))));
        assert_eq!(compare("home", &boxed, &[], THRESHOLD, MAX_DIFF, false), Ok(Outcome::Same(0.0)));
        // Another size.
        let small = {
            let mut out = std::io::Cursor::new(Vec::new());
            RgbaImage::new(10, 10).write_to(&mut out, ImageFormat::Png).expect("png");
            out.into_inner()
        };
        assert!(compare("home", &small, &[], THRESHOLD, MAX_DIFF, false).expect_err("size").contains("10x10"));
        assert!(compare("../x", &white, &[], THRESHOLD, MAX_DIFF, false).is_err());
        unsafe { std::env::remove_var("FASTPILOT_DIR") };
        let _ = std::fs::remove_dir_all(d);
    }
}
