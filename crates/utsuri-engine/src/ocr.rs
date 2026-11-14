//! Text recognition through Tesseract (an external program). Ported from extras.py.

use crate::refuse;
use anyhow::Result;
use crate::proc::Quiet as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The tesseract executable: on PATH, or where the Windows installers put it.
pub fn tesseract() -> Option<PathBuf> {
    if let Some(p) = which(&crate::proc::exe_name("tesseract")) {
        return Some(p);
    }
    let bases = [std::env::var_os("ProgramFiles"), std::env::var_os("ProgramFiles(x86)"), std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Programs").into_os_string())];
    bases.into_iter().flatten().map(|b| PathBuf::from(b).join("Tesseract-OCR").join("tesseract.exe")).find(|p| p.is_file())
}

/// Find a program on PATH.
pub fn which(exe: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(exe)).find(|p| p.is_file()))
}

/// Installed OCR languages (without "osd").
pub fn langs() -> Vec<String> {
    let Some(exe) = tesseract() else { return vec![] };
    let Ok(out) = Command::new(exe).arg("--list-langs").stdin(Stdio::null()).quiet().output() else { return vec![] };
    String::from_utf8_lossy(&out.stdout).lines().skip(1).map(str::trim).filter(|l| !l.is_empty() && *l != "osd").map(String::from).collect()
}

pub fn ready() -> bool {
    use std::sync::OnceLock;
    static R: OnceLock<bool> = OnceLock::new();
    *R.get_or_init(|| langs().iter().any(|l| l == "eng"))
}

fn check_lang(lang: &str) -> Result<PathBuf> {
    // before anything else: the codes reach a command line
    let valid = !lang.is_empty() && lang.split('+').all(|c| !c.is_empty() && c.chars().all(|ch| ch.is_ascii_alphabetic() || ch == '_'));
    if !valid {
        refuse!("'{lang}' isn't a valid OCR language (use codes like eng or eng+vie)");
    }
    let Some(exe) = tesseract() else { refuse!("Tesseract isn't installed. Run: winget install UB-Mannheim.TesseractOCR") };
    let have = langs();
    if let Some(m) = lang.split('+').find(|c| !have.iter().any(|h| h == c)) {
        refuse!("OCR language '{m}' isn't installed. Rerun the Tesseract installer and tick it under Additional language data.");
    }
    Ok(exe)
}

fn run(exe: &Path, args: &[&std::ffi::OsStr]) -> Result<std::process::Output> {
    Ok(Command::new(exe).args(args).stdin(Stdio::null()).quiet().output()?)
}

fn last_line(b: &[u8]) -> String {
    String::from_utf8_lossy(b).lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("no output").to_string()
}

/// The text in an image (no file written).
pub fn ocr_text(src: &Path, lang: &str) -> Result<String> {
    let exe = check_lang(lang)?;
    let tmp = crate::out::temp_part(&std::env::temp_dir(), ".png");
    // HEIC/AVIF/WEBP become something leptonica reads
    let r = (|| -> Result<String> {
        crate::imgio::open_image(src)?.save_with_format(&tmp, image::ImageFormat::Png)?;
        let out = run(&exe, &[tmp.as_os_str(), "stdout".as_ref(), "-l".as_ref(), lang.as_ref()])?;
        if !out.status.success() {
            refuse!("tesseract failed: {}", last_line(&out.stderr));
        }
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if text.is_empty() {
            refuse!("No text found in this image");
        }
        Ok(text)
    })();
    let _ = std::fs::remove_file(&tmp);
    r
}

/// Read the text in an image and save it as "<name> Text.txt". Returns (file, text).
pub fn ocr_to_file(src: &Path, lang: &str) -> Result<(PathBuf, String)> {
    let text = ocr_text(src, lang)?;
    let out = crate::out::sibling(src, "Text", Some("txt"));
    crate::out::atomic_write(&out, |tmp| std::fs::write(tmp, format!("{text}\n")))?;
    Ok((out, text))
}

/// A one-page searchable PDF (image + invisible text) at `base`.pdf.
pub fn tesseract_pdf(png: &Path, base: &Path, lang: &str) -> Result<()> {
    let exe = check_lang(lang)?;
    let out = run(&exe, &[png.as_os_str(), base.as_os_str(), "-l".as_ref(), lang.as_ref(), "pdf".as_ref()])?;
    if !out.status.success() || !base.with_extension("pdf").is_file() {
        refuse!("{}", last_line(&out.stderr));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_language_codes_are_refused_before_running() {
        for bad in ["", "eng;rm", "en g", "eng+", "../x"] {
            let e = ocr_text(Path::new("x.png"), bad).unwrap_err();
            assert!(crate::friendly(&e).contains("isn't a valid OCR language"), "{bad}");
        }
    }

    #[test]
    fn reads_rendered_text() {
        if !ready() {
            eprintln!("tesseract not installed: skipped");
            return;
        }
        let d = crate::testutil::tmpdir("ocr");
        let mark = crate::pdfgen::text_stamp_image("Hello Utsuri 42", 30.0, [0, 0, 0, 255]).unwrap();
        let mut white = image::RgbaImage::from_pixel(mark.width() + 80, mark.height() + 80, image::Rgba([255, 255, 255, 255]));
        image::imageops::overlay(&mut white, &mark, 40, 40);
        let p = d.join("t.png");
        white.save(&p).unwrap();
        let (out, text) = ocr_to_file(&p, "eng").unwrap();
        assert!(text.contains("Hello") && text.contains("42"), "{text}");
        assert_eq!(out.file_name().unwrap(), "t Text.txt");
        let _ = std::fs::remove_dir_all(&d);
    }
}
