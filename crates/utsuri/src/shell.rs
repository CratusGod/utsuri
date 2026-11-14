//! Windows shell bits: open, show in Explorer, clipboard, start at login, the right-click
//! "Utsuri…" entry and Send To, and window backdrops.

use anyhow::{Context as _, Result};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

pub const APP_NAME: &str = "Utsuri";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VERB: &str = "Utsuri";

pub fn exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("utsuri.exe"))
}

/// Open with the default app.
pub fn open(p: &Path) {
    if std::env::var_os("UTSURI_NO_LAUNCH").is_some() {
        log::info!("(test) would open {}", p.display());
        return;
    }
    let _ = std::process::Command::new("explorer.exe").arg(p).creation_flags(0x0800_0000).spawn();
}

/// Show in Explorer with the file selected.
pub fn reveal(p: &Path) {
    if std::env::var_os("UTSURI_NO_LAUNCH").is_some() {
        log::info!("(test) would reveal {}", p.display());
        return;
    }
    let mut arg = std::ffi::OsString::from("/select,");
    arg.push(p.as_os_str());
    let _ = std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", p.display())).creation_flags(0x0800_0000).spawn();
    let _ = arg;
}

pub fn open_url(url: &str) {
    if std::env::var_os("UTSURI_NO_LAUNCH").is_some() {
        log::info!("(test) would open {url}");
        return;
    }
    let _ = std::process::Command::new("rundll32.exe").args(["url.dll,FileProtocolHandler", url]).spawn();
}

pub fn set_clipboard(clip: &utsuri_engine::report::Clip) -> Result<()> {
    let mut cb = arboard::Clipboard::new().context("the clipboard is busy")?;
    match clip {
        utsuri_engine::report::Clip::Text(t) => cb.set_text(t.clone())?,
        utsuri_engine::report::Clip::Image(im) => cb.set_image(arboard::ImageData {
            width: im.width() as usize,
            height: im.height() as usize,
            bytes: std::borrow::Cow::Borrowed(im.as_raw()),
        })?,
    }
    Ok(())
}

// ---------------------------------------------------------------- start at login

pub fn start_at_login() -> bool {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(RUN_KEY).and_then(|k| k.get_value::<String, _>(APP_NAME)).is_ok()
}

pub fn set_start_at_login(on: bool) -> Result<()> {
    let (k, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
    if on {
        k.set_value(APP_NAME, &format!("\"{}\" --quiet", exe().display()))?;
    } else {
        let _ = k.delete_value(APP_NAME);
    }
    Ok(())
}

// ---------------------------------------------------------------- right-click + Send To

fn send_to_link(suffix: &str) -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join(r"Microsoft\Windows\SendTo").join(format!("{APP_NAME}{suffix}.lnk")))
}

/// The registry key name of the right-click entry (a test install adds a suffix).
fn verb(suffix: &str) -> String {
    format!("{VERB}{}", suffix.replace(|c: char| !c.is_ascii_alphanumeric(), ""))
}

pub fn shell_menu_installed() -> bool {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(format!(r"Software\Classes\*\shell\{}", verb(""))).is_ok()
}

/// Add "Utsuri…" to every file's and folder's right-click menu, and a Send To entry.
pub fn install_shell_menu(suffix: &str) -> Result<()> {
    let hk = RegKey::predef(HKEY_CURRENT_USER);
    let cmd = format!("\"{}\" \"%1\"", exe().display());
    for base in [r"Software\Classes\*\shell", r"Software\Classes\Directory\shell"] {
        let (k, _) = hk.create_subkey(format!(r"{base}\{}", verb(suffix)))?;
        k.set_value("MUIVerb", &format!("{APP_NAME}{suffix}…"))?;
        k.set_value("Icon", &format!("\"{}\",0", exe().display()))?;
        // one menu for a whole multi-selection: Explorer starts us per file, the pipe gathers them
        k.set_value("MultiSelectModel", &"Player")?;
        let (c, _) = k.create_subkey("command")?;
        c.set_value("", &cmd)?;
    }
    if let Some(lnk) = send_to_link(suffix) {
        create_shortcut(&lnk, &exe())?;
    }
    Ok(())
}

pub fn uninstall_shell_menu(suffix: &str) -> Result<()> {
    let hk = RegKey::predef(HKEY_CURRENT_USER);
    for base in [r"Software\Classes\*\shell", r"Software\Classes\Directory\shell"] {
        let _ = hk.delete_subkey_all(format!(r"{base}\{}", verb(suffix)));
    }
    if let Some(lnk) = send_to_link(suffix) {
        let _ = std::fs::remove_file(lnk);
    }
    Ok(())
}

fn create_shortcut(lnk: &Path, target: &Path) -> Result<()> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{HSTRING, Interface};
    utsuri_engine::winrt::com_init();
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(target.as_os_str()))?;
        link.SetDescription(&HSTRING::from("Convert or edit with Utsuri"))?;
        let pf: IPersistFile = link.cast()?;
        pf.Save(&HSTRING::from(lnk.as_os_str()), true)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- screen

/// The work area (screen minus taskbar) of the monitor at a point, physical pixels.
pub fn work_area_at(x: i32, y: i32) -> (i32, i32, i32, i32) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint};
    unsafe {
        let m = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(m, &mut mi).as_bool() {
            let r = mi.rcWork;
            return (r.left, r.top, r.right - r.left, r.bottom - r.top);
        }
    }
    (0, 0, 1920, 1040)
}

pub fn cursor() -> (i32, i32) {
    let mut p = windows::Win32::Foundation::POINT::default();
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p);
    }
    (p.x, p.y)
}
