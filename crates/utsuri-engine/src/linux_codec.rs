//! Linux stand-ins for `winrt.rs` (same functions, same signatures): PDF pages through
//! poppler's `pdftoppm`, HEIC/AVIF through libheif's `heif-dec` / `heif-enc`. Every tool is
//! an ordinary system package (`poppler`, `libheif`), found on PATH.

use crate::proc::Quiet as _;
use crate::refuse;
use anyhow::{Context as _, Result, anyhow};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Nothing to set up on Linux (kept so callers stay platform-neutral).
pub fn com_init() {}

static BUSY: AtomicUsize = AtomicUsize::new(0);
static CLOSING: AtomicBool = AtomicBool::new(false);

/// Counts a codec call in flight, so `shutdown` can wait for it (same contract as Windows).
struct Busy;
impl Busy {
    fn enter() -> Result<Busy> {
        BUSY.fetch_add(1, Ordering::SeqCst);
        if CLOSING.load(Ordering::SeqCst) {
            BUSY.fetch_sub(1, Ordering::SeqCst);
            return Err(anyhow!("closing"));
        }
        Ok(Busy)
    }
}
impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Call before the process exits: refuses new codec calls and waits (up to `max`) for
/// the ones running to finish.
pub fn shutdown(max: Duration) {
    CLOSING.store(true, Ordering::SeqCst);
    let t = Instant::now();
    while BUSY.load(Ordering::SeqCst) > 0 && t.elapsed() < max {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn tool(name: &str) -> Option<PathBuf> {
    crate::ocr::which(name)
}

/// A fresh temp path for one tool run (removed by the caller).
fn temp(tag: &str, ext: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!("utsuri-{tag}-{}-{}{ext}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)))
}

fn run(cmd: &mut Command, what: &str) -> Result<()> {
    let out = cmd.stdin(Stdio::null()).quiet().output().with_context(|| format!("couldn't start {what}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("failed");
        return Err(anyhow!("{what}: {line}"));
    }
    Ok(())
}

/// Render page `index` (0-based) of a PDF to PNG bytes `width` pixels wide.
pub fn render_pdf_page_png(path: &Path, password: Option<&str>, index: u32, width: u32) -> Result<Vec<u8>> {
    let _busy = Busy::enter()?;
    let Some(exe) = tool("pdftoppm") else { refuse!("PDF pages need poppler: sudo pacman -S poppler") };
    let root = temp("pdfpage", "");
    let page = (index + 1).to_string();
    let mut c = Command::new(exe);
    c.args(["-png", "-singlefile", "-f", &page, "-l", &page, "-scale-to-x", &width.to_string(), "-scale-to-y", "-1"]);
    if let Some(pw) = password {
        c.arg("-upw").arg(pw);
    }
    c.arg(path).arg(&root);
    let png = root.with_extension("png");
    let r = run(&mut c, "pdftoppm").and_then(|_| std::fs::read(&png).context("pdftoppm wrote no page"));
    let _ = std::fs::remove_file(&png);
    r
}

/// Decode HEIC/HEIF/AVIF (anything libheif reads) to RGBA8.
pub fn wic_decode(path: &Path) -> Result<image::RgbaImage> {
    let _busy = Busy::enter()?;
    let Some(exe) = tool("heif-dec") else { refuse!("HEIC/AVIF need libheif: sudo pacman -S libheif") };
    let png = temp("heifdec", ".png");
    let r = run(Command::new(exe).arg(path).arg(&png), "heif-dec").and_then(|_| {
        // heif-dec names extra images of a sequence "-1", "-2"…; the primary one is `png`
        Ok(image::open(&png).context("heif-dec wrote no image")?.to_rgba8())
    });
    let _ = std::fs::remove_file(&png);
    r
}

/// Write `im` as HEIC (HEVC) at `quality` 0.0-1.0.
pub fn wic_encode_heic(im: &image::RgbaImage, out: &Path, quality: f32) -> Result<()> {
    let _busy = Busy::enter()?;
    let Some(exe) = tool("heif-enc") else { refuse!("HEIC needs libheif: sudo pacman -S libheif") };
    let png = temp("heifenc", ".png");
    im.save(&png)?;
    let q = ((quality * 100.0).round() as i32).clamp(1, 100).to_string();
    let r = run(Command::new(exe).args(["-q", &q, "-o"]).arg(out).arg(&png), "heif-enc");
    let _ = std::fs::remove_file(&png);
    r
}

/// Whether HEIC can be written here (heif-enc with an HEVC encoder).
pub fn heif_encoder_available() -> bool {
    let p = temp("heif-check", ".heic");
    let ok = wic_encode_heic(&image::RgbaImage::from_pixel(16, 16, image::Rgba([200, 100, 50, 255])), &p, 0.8).is_ok() && std::fs::metadata(&p).map(|m| m.len() > 0).unwrap_or(false);
    let _ = std::fs::remove_file(&p);
    ok
}

/// Windows .lnk shortcuts don't exist here: the path is used as-is.
pub fn shortcut_target(_lnk: &Path) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heic_round_trip() {
        if tool("heif-enc").is_none() {
            return;
        }
        let d = crate::testutil::tmpdir("heic-rt");
        let p = d.join("a.heic");
        let im = image::RgbaImage::from_pixel(64, 48, image::Rgba([10, 200, 30, 255]));
        wic_encode_heic(&im, &p, 0.9).unwrap();
        let back = wic_decode(&p).unwrap();
        assert_eq!((back.width(), back.height()), (64, 48));
    }
}
