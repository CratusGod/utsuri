//! Where results go. Next to the original (or a "Utsuri" subfolder, by setting), never
//! over anything. Batch jobs redirect output into a mirrored folder tree.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

pub use shared::paths::{atomic_write, human_size, unique_path};

thread_local! {
    /// Set while a batch job runs: maps a source file to the folder its result goes in.
    static REDIRECT: RefCell<Option<Box<dyn Fn(&Path) -> PathBuf>>> = const { RefCell::new(None) };
}

/// The folder results for `src` go in.
pub fn output_dir(src: &Path) -> PathBuf {
    if let Some(d) = REDIRECT.with(|r| r.borrow().as_ref().map(|f| f(src))) {
        return d;
    }
    let d = src.parent().map(Path::to_path_buf).unwrap_or_default();
    if crate::options().output_folder {
        let d = d.join("Utsuri");
        let _ = std::fs::create_dir_all(&d);
        return d;
    }
    d
}

/// Output path for `src`: `sibling("a.jpg", "Cropped", None)` -> `a Cropped.jpg`.
pub fn sibling(src: &Path, label: &str, ext: Option<&str>) -> PathBuf {
    shared::paths::sibling(src, Some(&output_dir(src)), label, ext)
}

/// Run `f` with every result redirected by `map`.
pub fn with_redirect<T>(map: impl Fn(&Path) -> PathBuf + 'static, f: impl FnOnce() -> T) -> T {
    REDIRECT.with(|r| *r.borrow_mut() = Some(Box::new(map)));
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            REDIRECT.with(|r| *r.borrow_mut() = None);
        }
    }
    let _reset = Reset;
    f()
}

pub fn file_size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// Total bytes of files and folders (links not followed).
pub fn tree_size(paths: &[PathBuf]) -> u64 {
    fn walk(p: &Path) -> u64 {
        match std::fs::symlink_metadata(p) {
            Ok(m) if m.is_file() => m.len(),
            Ok(m) if m.is_dir() => std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| walk(&e.path())).sum()).unwrap_or(0),
            _ => 0,
        }
    }
    paths.iter().map(|p| walk(p)).sum()
}

/// A hidden ".utsuri-XXXX<suffix>" path in `folder` for writing before the final rename.
pub fn temp_part(folder: &Path, suffix: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    loop {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = folder.join(format!(".utsuri-{}-{n}{suffix}", std::process::id()));
        if !p.exists() {
            return p;
        }
    }
}

// ---------------------------------------------------------------- created folders
// Undo may move a *folder* to the Recycle Bin only if Utsuri created it (extract, batch).

const LEDGER_KEEP: usize = 100;

fn ledger() -> PathBuf {
    let base = std::env::var_os("UTSURI_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| shared::paths::state_dir("Utsuri-rs"));
    base.join("created-dirs")
}

pub fn created_dirs() -> Vec<String> {
    std::fs::read_to_string(ledger()).map(|s| s.lines().filter(|l| !l.is_empty()).map(String::from).collect()).unwrap_or_default()
}

pub fn remember_dir(d: &Path) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let led = ledger();
    if let Some(p) = led.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let me = d.to_string_lossy().into_owned();
    let mut keep: Vec<String> = created_dirs().into_iter().filter(|x| *x != me).collect();
    let start = keep.len().saturating_sub(LEDGER_KEEP - 1);
    keep.drain(..start);
    keep.push(me);
    let _ = atomic_write(&led, |tmp| std::fs::write(tmp, keep.join("\n") + "\n"));
}

/// Undo: move results to the Recycle Bin. Folders only if Utsuri made them.
pub fn trash(paths: &[PathBuf]) -> anyhow::Result<usize> {
    let made = created_dirs();
    let ok: Vec<&PathBuf> = paths
        .iter()
        .filter(|p| p.is_file() || (p.is_dir() && made.contains(&p.to_string_lossy().into_owned())))
        .collect();
    if ok.is_empty() {
        crate::refuse!("Nothing to undo: the results were already moved or deleted");
    }
    // Tests and the E2E run set this so they never fill the real Recycle Bin.
    if let Some(bin) = std::env::var_os("UTSURI_FAKE_TRASH").map(PathBuf::from) {
        std::fs::create_dir_all(&bin)?;
        for p in &ok {
            let dest = unique_path(&bin.join(p.file_name().unwrap_or_default()));
            std::fs::rename(p, dest)?;
        }
        return Ok(ok.len());
    }
    trash::delete_all(&ok)?;
    Ok(ok.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::tmpdir;

    #[test]
    fn redirect_applies_only_inside() {
        let d = tmpdir("redirect");
        let src = d.join("a.png");
        let other = d.join("mirror");
        std::fs::create_dir_all(&other).unwrap();
        let o2 = other.clone();
        let inside = with_redirect(move |_| o2.clone(), || sibling(&src, "", Some("jpg")));
        assert_eq!(inside, other.join("a.jpg"));
        assert_eq!(sibling(&src, "", Some("jpg")), d.join("a.jpg"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ledger_and_trash_folder_rule() {
        let d = tmpdir("ledger");
        let mine = d.join("made by utsuri");
        let theirs = d.join("user folder");
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::create_dir_all(&theirs).unwrap();
        remember_dir(&mine);
        assert!(created_dirs().contains(&mine.to_string_lossy().into_owned()));
        // the user's own folder is never trashed
        assert!(trash(std::slice::from_ref(&theirs)).is_err());
        assert!(theirs.exists());
        assert_eq!(trash(std::slice::from_ref(&mine)).unwrap(), 1);
        assert!(!mine.exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
