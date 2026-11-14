//! Utsuri's file actions, with no UI: everything here is testable on its own.
//! Ported from the Python app's convert.py, imaging.py, pdfkit.py, cli.py, archive.py,
//! media.py, extras.py and report.py.

pub mod actions;
pub mod annotate;
pub mod archive;
pub mod docx;
pub mod fixtures;
pub mod imaging;
pub mod imgio;
pub mod kinds;
pub mod media;
pub mod ocr;
pub mod oumu;
pub mod out;
pub mod pdf;
pub mod pdfgen;
pub mod report;
pub mod winrt;

use std::sync::RwLock;

/// User settings the engine needs. The app sets them from its settings file.
#[derive(Clone, Debug)]
pub struct Options {
    /// Results go next to the original, or in a "Utsuri" subfolder beside it.
    pub output_folder: bool,
    pub jpg_quality: u8,
    pub pdf_dpi: u32,
    pub ocr_lang: String,
}

impl Default for Options {
    fn default() -> Self {
        Self { output_folder: false, jpg_quality: 90, pdf_dpi: 200, ocr_lang: "eng".into() }
    }
}

static OPTIONS: RwLock<Option<Options>> = RwLock::new(None);

pub fn options() -> Options {
    OPTIONS.read().unwrap().clone().unwrap_or_default()
}

pub fn set_options(o: Options) {
    *OPTIONS.write().unwrap() = Some(o);
}

/// An error whose message is fine to show the user as-is.
#[derive(Debug)]
pub struct UserError(pub String);

impl std::fmt::Display for UserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UserError {}

/// `bail!` for messages meant for the user.
#[macro_export]
macro_rules! refuse {
    ($($t:tt)*) => { return Err($crate::UserError(format!($($t)*)).into()) };
}

/// The message to show for an error: a UserError as-is, anything else short.
pub fn friendly(e: &anyhow::Error) -> String {
    if let Some(u) = e.downcast_ref::<UserError>() {
        return u.0.clone();
    }
    if let Some(io) = e.downcast_ref::<std::io::Error>() {
        match io.kind() {
            std::io::ErrorKind::PermissionDenied => return "Permission denied: the file or folder is read-only or open in another app".into(),
            std::io::ErrorKind::NotFound => return "The file was moved or deleted".into(),
            _ => {}
        }
    }
    let s = format!("{e:#}");
    if s.chars().count() > 160 { format!("{}…", s.chars().take(159).collect::<String>()) } else { s }
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::path::PathBuf;

    /// A fresh, empty temp folder for one test.
    /// Point the ledger and the Recycle Bin at temp folders, once per test run.
    pub fn test_env() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let base = std::env::temp_dir().join(format!("utsuri-engine-state-{}", std::process::id()));
            // SAFETY: set once, before any test reads them.
            unsafe {
                std::env::set_var("UTSURI_STATE_DIR", base.join("state"));
                std::env::set_var("UTSURI_FAKE_TRASH", base.join("bin"));
            }
        });
    }

    pub fn tmpdir(tag: &str) -> PathBuf {
        test_env();
        let d = std::env::temp_dir().join(format!("utsuri-engine-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A colourful test photo with a gradient (compresses like a real one).
    pub fn photo(w: u32, h: u32) -> image::RgbImage {
        image::RgbImage::from_fn(w, h, |x, y| {
            let n = ((x * 7919 + y * 104729) % 23) as u8;
            image::Rgb([(x * 255 / w) as u8 ^ n, (y * 255 / h) as u8, ((x + y) % 256) as u8])
        })
    }
}
