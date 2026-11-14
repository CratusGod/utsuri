//! What kind of file is this, and which files of a drop belong together.
//! Ported from util.py (category), media.py (category), archive.py (archive_suffix) and
//! cli.py (kind_of, same_kind, check_files).

use std::path::{Path, PathBuf};

pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "heic", "heif", "tif", "tiff", "avif", "bmp", "gif", "ico"];
pub const PDF_EXTS: &[&str] = &["pdf"];
pub const DOCX_EXTS: &[&str] = &["docx"];
pub const TEXT_EXTS: &[&str] = &["txt", "md", "csv", "log", "json", "xml", "html", "htm"];
pub const VIDEO_EXTS: &[&str] = &["mp4", "mov", "mkv", "webm", "avi", "m4v"];
pub const AUDIO_EXTS: &[&str] = &["mp3", "m4a", "wav", "flac", "ogg", "opus"];
/// Longest first, so "x.tar.gz" is a .tar.gz and not a .gz.
pub const ARCHIVE_SUFFIXES: &[&str] = &[".tar.gz", ".tar.xz", ".tar.bz2", ".tgz", ".txz", ".tbz2", ".zip", ".tar"];
pub const MAX_FILES: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Image,
    Pdf,
    Docx,
    Text,
    Video,
    Audio,
    Archive,
    Folder,
    /// Nothing known: can still be archived or have its path copied.
    Any,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Pdf => "pdf",
            Kind::Docx => "docx",
            Kind::Text => "text",
            Kind::Video => "video",
            Kind::Audio => "audio",
            Kind::Archive => "archive",
            Kind::Folder => "folder",
            Kind::Any => "any",
        }
    }
}

pub fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

pub fn archive_suffix(p: &Path) -> &'static str {
    let name = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    ARCHIVE_SUFFIXES.iter().copied().find(|s| name.ends_with(s) && name.len() > s.len()).unwrap_or("")
}

/// "photo.tar.gz" -> "photo"; anything else -> the usual stem.
pub fn archive_stem(p: &Path) -> String {
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let s = archive_suffix(p);
    if s.is_empty() {
        p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    } else {
        name[..name.len() - s.len()].to_string()
    }
}

/// The kind of a single file (by extension), or None.
pub fn category(p: &Path) -> Option<Kind> {
    let e = ext(p);
    let e = e.as_str();
    if IMAGE_EXTS.contains(&e) {
        Some(Kind::Image)
    } else if PDF_EXTS.contains(&e) {
        Some(Kind::Pdf)
    } else if DOCX_EXTS.contains(&e) {
        Some(Kind::Docx)
    } else if TEXT_EXTS.contains(&e) {
        Some(Kind::Text)
    } else if VIDEO_EXTS.contains(&e) {
        Some(Kind::Video)
    } else if AUDIO_EXTS.contains(&e) {
        Some(Kind::Audio)
    } else {
        None
    }
}

pub fn kind_of(p: &Path) -> Option<Kind> {
    if p.is_dir() {
        return Some(Kind::Folder);
    }
    if !archive_suffix(p).is_empty() {
        return Some(Kind::Archive);
    }
    category(p)
}

/// Windows .lnk shortcuts (desktop icons) stand for the file they point to.
pub fn resolve_shortcuts(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths.iter().map(|p| if ext(p) == "lnk" { crate::winrt::shortcut_target(p).unwrap_or_else(|| p.clone()) } else { p.clone() }).collect()
}

/// A drop, sorted out: the kind, the files of that kind, and everything usable.
#[derive(Clone, Debug)]
pub struct Drop {
    pub kind: Kind,
    pub files: Vec<PathBuf>,
    pub everything: Vec<PathBuf>,
}

/// The first file of a known kind decides; the rest of that kind come along. All
/// folders is Folder; nothing known (unknown files, folders mixed in) is Any.
pub fn sort_drop(paths: &[PathBuf]) -> Drop {
    let items: Vec<PathBuf> = resolve_shortcuts(paths).into_iter().filter(|p| p.is_file() || p.is_dir()).take(MAX_FILES).collect();
    let kinds: Vec<Option<Kind>> = items.iter().map(|p| kind_of(p)).collect();
    let cat = kinds.iter().flatten().copied().find(|k| *k != Kind::Folder);
    match cat {
        Some(cat) => {
            let files = items.iter().zip(&kinds).filter(|(_, k)| **k == Some(cat)).map(|(p, _)| p.clone()).collect();
            Drop { kind: cat, files, everything: items }
        }
        None => {
            let all_folders = !items.is_empty() && kinds.iter().all(|k| *k == Some(Kind::Folder));
            Drop { kind: if all_folders { Kind::Folder } else { Kind::Any }, files: items.clone(), everything: items }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::tmpdir;

    #[test]
    fn categories_and_archives() {
        assert_eq!(category(Path::new("a.HEIC")), Some(Kind::Image));
        assert_eq!(category(Path::new("a.md")), Some(Kind::Text));
        assert_eq!(category(Path::new("a.m4v")), Some(Kind::Video));
        assert_eq!(category(Path::new("a.opus")), Some(Kind::Audio));
        assert_eq!(category(Path::new("a.xyz")), None);
        assert_eq!(archive_suffix(Path::new("x.tar.gz")), ".tar.gz");
        assert_eq!(archive_suffix(Path::new(".zip")), "");
        assert_eq!(archive_stem(Path::new("photos.tar.xz")), "photos");
        assert_eq!(archive_stem(Path::new("a.b.zip")), "a.b");
    }

    #[test]
    fn first_known_kind_decides() {
        let d = tmpdir("kinds");
        for n in ["a.xyz", "b.png", "c.pdf", "d.jpg"] {
            std::fs::write(d.join(n), b"x").unwrap();
        }
        std::fs::create_dir(d.join("sub")).unwrap();
        let p = |n: &str| d.join(n);
        let r = sort_drop(&[p("a.xyz"), p("b.png"), p("c.pdf"), p("d.jpg"), p("sub")]);
        assert_eq!(r.kind, Kind::Image);
        assert_eq!(r.files, vec![p("b.png"), p("d.jpg")]);
        assert_eq!(r.everything.len(), 5);
        assert_eq!(sort_drop(&[p("sub")]).kind, Kind::Folder);
        assert_eq!(sort_drop(&[p("sub"), p("a.xyz")]).kind, Kind::Any);
        assert_eq!(sort_drop(&[p("gone.png")]).everything.len(), 0);
        let _ = std::fs::remove_dir_all(&d);
    }
}
