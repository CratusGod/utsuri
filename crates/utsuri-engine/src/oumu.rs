//! Translate with Oumu: hand a file's text to Oumu's named pipe. Ported from oumu_link.py.
//! Oumu listens on \\.\pipe\oumu-<user> for "translate\n<text>"; when it's installed but
//! not running it is started and the text is sent once it listens.

use crate::kinds::{Kind, category};
use crate::refuse;
use anyhow::Result;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Oumu reads at most 64 KiB per request; leave room for the command line.
pub const MAX_BYTES: usize = 60_000;

pub fn pipe_name() -> String {
    if let Ok(p) = std::env::var("OUMU_PIPE") {
        if p.starts_with(r"\\.\pipe\") {
            return p;
        }
    }
    let user: String = std::env::var("USERNAME").unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c)).collect();
    format!(r"\\.\pipe\oumu-{}", if user.is_empty() { "user" } else { &user })
}

/// The pipe exists (listed, not opened: opening would use up Oumu's waiting connection).
pub fn running() -> bool {
    let name = pipe_name();
    let short = name.trim_start_matches(r"\\.\pipe\");
    std::fs::read_dir(r"\\.\pipe\").map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(short))).unwrap_or(false)
}

pub fn find_exe() -> Option<PathBuf> {
    if let Some(p) = crate::ocr::which("oumu.exe") {
        return Some(p);
    }
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default();
    let pf = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_default();
    [local.join("Oumu").join("oumu.exe"), local.join("Programs").join("Oumu").join("oumu.exe"), pf.join("Oumu").join("oumu.exe")].into_iter().find(|p| p.is_file())
}

/// Show the Translate slice: Oumu runs, or can be started.
pub fn available() -> bool {
    running() || find_exe().is_some()
}

/// The text in a text file, Word document or PDF; an image through OCR.
pub fn text_of(p: &Path) -> Result<String> {
    match category(p) {
        Some(Kind::Text) => crate::docx::read_any_text(p),
        Some(Kind::Docx) => crate::docx::read_text(p),
        Some(Kind::Pdf) => crate::pdf::extract_text(p),
        Some(Kind::Image) => crate::ocr::ocr_text(p, &crate::options().ocr_lang),
        _ => refuse!("Utsuri can't read text from {}", p.file_name().unwrap_or_default().to_string_lossy()),
    }
}

/// `text` cut to `limit` UTF-8 bytes on a character boundary; (text, was it cut).
pub fn clip(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_string(), false);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), true)
}

fn try_send(text: &str) -> bool {
    // open existing, write only
    let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(pipe_name()) else { return false };
    let mut msg = b"translate\n".to_vec();
    msg.extend_from_slice(text.as_bytes());
    f.write_all(&msg).is_ok()
}

pub fn send(text: &str) -> Result<()> {
    if text.trim().is_empty() {
        refuse!("There is no text in it to translate");
    }
    if try_send(text) {
        return Ok(());
    }
    let Some(exe) = find_exe() else { refuse!("Oumu isn't running. Start Oumu and try again.") };
    use std::os::windows::process::CommandExt;
    std::process::Command::new(exe)
        .creation_flags(0x0000_0008 | 0x0000_0200) // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(300));
        if try_send(text) {
            return Ok(());
        }
    }
    refuse!("Oumu did not start in time")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_on_char_boundary() {
        let s = "ảảảả"; // 3 bytes each
        assert_eq!(clip(s, 7), ("ảả".to_string(), true));
        assert_eq!(clip("abc", 10), ("abc".to_string(), false));
    }

    #[test]
    fn pipe_name_rules() {
        assert!(pipe_name().starts_with(r"\\.\pipe\oumu-"));
    }
}
