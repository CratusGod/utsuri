//! Desktop bits on Linux: open, show in the file manager, clipboard, start at login (XDG
//! autostart), the right-click "Utsuri…" entry (Dolphin service menu, plus an "Open with"
//! application entry every file manager lists), and the screen under the pointer.

use anyhow::{Context as _, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const APP_NAME: &str = "Utsuri";
const DESKTOP_ID: &str = "dev.cratusgod.utsuri";

pub fn exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("utsuri"))
}

fn spawn(cmd: &mut Command) {
    // detached and silent; a missing tool only loses this one action
    let r = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    if let Err(e) = r {
        log::warn!("couldn't start {:?}: {e}", cmd.get_program());
    }
}

/// Open with the default app.
pub fn open(p: &Path) {
    if std::env::var_os("UTSURI_NO_LAUNCH").is_some() {
        log::info!("(test) would open {}", p.display());
        return;
    }
    spawn(Command::new("xdg-open").arg(p));
}

/// Show in the file manager with the file selected (FileManager1 over D-Bus), or open its
/// folder when no file manager answers.
pub fn reveal(p: &Path) {
    if std::env::var_os("UTSURI_NO_LAUNCH").is_some() {
        log::info!("(test) would reveal {}", p.display());
        return;
    }
    let uri = file_uri(p);
    let ok = Command::new("dbus-send")
        .args(["--session", "--print-reply", "--dest=org.freedesktop.FileManager1", "--type=method_call", "/org/freedesktop/FileManager1", "org.freedesktop.FileManager1.ShowItems"])
        .arg(format!("array:string:{uri}"))
        .arg("string:")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        spawn(Command::new("xdg-open").arg(p.parent().unwrap_or(p)));
    }
}

/// `file://` URI with every byte outside the unreserved set percent-encoded.
pub fn file_uri(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut s = String::from("file://");
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

pub fn open_url(url: &str) {
    if std::env::var_os("UTSURI_NO_LAUNCH").is_some() {
        log::info!("(test) would open {url}");
        return;
    }
    spawn(Command::new("xdg-open").arg(url));
}

pub fn set_clipboard(clip: &utsuri_engine::report::Clip) -> Result<()> {
    let mut cb = arboard::Clipboard::new().context("the clipboard is busy")?;
    match clip {
        utsuri_engine::report::Clip::Text(t) => cb.set_text(t.clone())?,
        utsuri_engine::report::Clip::Image(im) => cb.set_image(arboard::ImageData { width: im.width() as usize, height: im.height() as usize, bytes: std::borrow::Cow::Borrowed(im.as_raw()) })?,
    }
    Ok(())
}

// ---------------------------------------------------------------- start at login

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config"))
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"))
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn autostart_file() -> PathBuf {
    config_home().join("autostart").join(format!("{DESKTOP_ID}.desktop"))
}

/// Quote one argument for a desktop entry's Exec line.
fn exec_quote(p: &Path) -> String {
    let s = p.display().to_string();
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "/-_.+".contains(c)) {
        return s;
    }
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('`', "\\`").replace('$', "\\$"))
}

pub fn start_at_login() -> bool {
    autostart_file().is_file()
}

pub fn set_start_at_login(on: bool) -> Result<()> {
    let f = autostart_file();
    if on {
        std::fs::create_dir_all(f.parent().unwrap())?;
        let body = format!("[Desktop Entry]\nType=Application\nName={APP_NAME}\nComment=Shift+drag a file for a menu of conversions and tools\nExec={} --quiet\nIcon={DESKTOP_ID}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n", exec_quote(&exe()));
        std::fs::write(&f, body)?;
    } else {
        let _ = std::fs::remove_file(&f);
    }
    Ok(())
}

// ---------------------------------------------------------------- right-click + Open with

fn suffix_id(suffix: &str) -> String {
    suffix.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

fn app_entry(suffix: &str) -> PathBuf {
    let s = suffix_id(suffix);
    data_home().join("applications").join(if s.is_empty() { format!("{DESKTOP_ID}.desktop") } else { format!("{DESKTOP_ID}-{s}.desktop") })
}

fn service_menu(suffix: &str) -> PathBuf {
    let s = suffix_id(suffix);
    data_home().join("kio/servicemenus").join(if s.is_empty() { "utsuri.desktop".to_string() } else { format!("utsuri-{s}.desktop") })
}

pub fn shell_menu_installed() -> bool {
    service_menu("").is_file() || app_entry("").is_file()
}

/// "Utsuri…" in Dolphin's right-click menu for files and folders, and Utsuri in every file
/// manager's "Open with" list. A multi-selection starts us once with all files.
pub fn install_shell_menu(suffix: &str) -> Result<()> {
    let exe = exec_quote(&exe());
    let app = app_entry(suffix);
    std::fs::create_dir_all(app.parent().unwrap())?;
    std::fs::write(&app, format!("[Desktop Entry]\nType=Application\nName={APP_NAME}{suffix}\nGenericName=Convert or edit a file\nComment=Shift+drag a file for a menu of conversions and tools\nExec={exe} %F\nIcon={DESKTOP_ID}\nTerminal=false\nCategories=Utility;\nMimeType=application/octet-stream;inode/directory;image/png;image/jpeg;image/webp;image/gif;image/heic;image/avif;image/tiff;image/bmp;application/pdf;text/plain;video/mp4;video/webm;audio/mpeg;audio/flac;application/zip;\nNoDisplay=false\n"))?;
    let menu = service_menu(suffix);
    std::fs::create_dir_all(menu.parent().unwrap())?;
    std::fs::write(&menu, format!("[Desktop Entry]\nType=Service\nMimeType=all/all;\nActions=utsuri;\nX-KDE-Priority=TopLevel\n\n[Desktop Action utsuri]\nName={APP_NAME}{suffix}…\nIcon={DESKTOP_ID}\nExec={exe} %F\n"))?;
    // KDE runs only service menus marked executable
    set_exec(&menu)?;
    let _ = Command::new("update-desktop-database").arg(app.parent().unwrap()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    Ok(())
}

fn set_exec(p: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = std::fs::metadata(p)?.permissions();
    perm.set_mode(perm.mode() | 0o100);
    std::fs::set_permissions(p, perm)?;
    Ok(())
}

pub fn uninstall_shell_menu(suffix: &str) -> Result<()> {
    let _ = std::fs::remove_file(app_entry(suffix));
    let _ = std::fs::remove_file(service_menu(suffix));
    Ok(())
}

// ---------------------------------------------------------------- screen

/// The work area (screen minus bars) of the monitor at a point, layout px.
pub fn work_area_at(x: i32, y: i32) -> (i32, i32, i32, i32) {
    crate::linux::hypr::monitor_at((x, y)).map(|m| m.work_area()).unwrap_or((0, 0, 1920, 1080))
}

/// The pointer, layout px (the last known one when the compositor doesn't answer).
pub fn cursor() -> (i32, i32) {
    use std::sync::Mutex;
    static LAST: Mutex<(i32, i32)> = Mutex::new((960, 540));
    match crate::linux::hypr::cursor() {
        Some(p) => {
            *LAST.lock().unwrap() = p;
            p
        }
        None => *LAST.lock().unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_and_exec_quoting() {
        assert_eq!(file_uri(Path::new("/tmp/a b/ả.png")), "file:///tmp/a%20b/%E1%BA%A3.png");
        assert_eq!(exec_quote(Path::new("/usr/bin/utsuri")), "/usr/bin/utsuri");
        assert_eq!(exec_quote(Path::new("/home/x/my apps/u$")), "\"/home/x/my apps/u\\$\"");
    }
}
