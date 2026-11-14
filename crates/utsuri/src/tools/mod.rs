//! Tool windows (Compress, Resize, Crop, Edit, Annotate, Redact, Add BG, Metadata, and the PDF
//! ones), Settings and the welcome. Ported from tools.py, pdfwin.py and settings.py. Each is a
//! `ui::frame::Tool` in its own Rune window.

pub mod addbg;
pub mod annotate;
pub mod compress;
pub mod crop;
pub mod edit;
pub mod metadata;
pub mod organize;
pub mod password;
pub mod redact;
pub mod resize;
pub mod settings;
pub mod stamp;

use crate::ui::frame::{Tool, ToolS};
use rune::AppHandle;
use shared::theme::Theme;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use utsuri_engine::actions::Tool as Kind;

pub fn subtitle(paths: &[PathBuf]) -> String {
    if paths.len() == 1 {
        format!("{}  ·  {}", name(&paths[0]), utsuri_engine::out::human_size(utsuri_engine::out::file_size(&paths[0])))
    } else {
        let total: u64 = paths.iter().map(|p| utsuri_engine::out::file_size(p)).sum();
        format!("{} files  ·  {}", paths.len(), utsuri_engine::out::human_size(total))
    }
}

pub fn name(p: &Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

/// Windows 11 rounds the frameless window and draws its thin border (a popup window gets
/// square corners unless it asks).
fn round_corners(h: &rune::WindowHandle) {
    let _ = h.with_raw_handle(|raw| {
        use raw_window_handle::RawWindowHandle;
        if let RawWindowHandle::Win32(w) = raw.as_raw() {
            use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
            let hwnd = windows::Win32::Foundation::HWND(w.hwnd.get() as *mut _);
            unsafe {
                let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_ROUND as *const _ as *const _, std::mem::size_of_val(&DWMWCP_ROUND) as u32);
            }
        }
    });
}

/// Open a tool's window, centred in `area` (the work area under the cursor, physical px),
/// solid, with rounded corners.
pub(crate) fn launch<T: Tool>(t: T, sub: String, theme: &Theme, app: Option<AppHandle<crate::Hub>>, area: (i32, i32, i32, i32)) -> anyhow::Result<crate::ToolEntry> {
    static N: AtomicU32 = AtomicU32::new(0);
    let key = format!("tool-{}", N.fetch_add(1, Ordering::Relaxed));
    let (w, h) = t.size();
    let (ax, ay, aw, ah) = area;
    let scale = crate::scale_at(ax + aw / 2, ay + ah / 2);
    let (pw, ph) = ((w * scale) as i32, (h * scale) as i32);
    let (x, y) = (ax + (aw - pw) / 2, ay + (ah - ph) / 2);
    let mut state = ToolS::new(t, theme.clone(), sub);
    state.app = app;
    let mut spec = crate::ui::frame::spec(key, state);
    spec.options.position = Some((x, y));
    let spec = spec.on_created(|h| round_corners(&h));
    let window = rune::open_window(spec);
    Ok(crate::ToolEntry { window, at: None, opened: None, asked: std::time::Instant::now() })
}

pub(crate) fn open(tool: Kind, files: Vec<PathBuf>, theme: &Theme, app: Option<AppHandle<crate::Hub>>, area: (i32, i32, i32, i32)) -> anyhow::Result<crate::ToolEntry> {
    macro_rules! go {
        ($e:expr) => {{
            let (t, sub) = $e;
            launch(t, sub, theme, app, area)
        }};
    }
    match tool {
        Kind::Compress => go!(compress::Compress::new(files)?),
        Kind::Edit => go!(edit::Edit::new(files)?),
        Kind::Crop => go!(crop::Crop::new(files)?),
        Kind::Redact => go!(redact::Redact::new(files)?),
        Kind::Annotate => go!(annotate::Annotate::new(files)?),
        Kind::AddBg => go!(addbg::AddBg::new(files)?),
        Kind::Stamp => go!(stamp::Stamp::new(files)?),
        Kind::Organize => go!(organize::Organize::new(files)?),
        Kind::Resize => go!(resize::Resize::new(files)?),
        Kind::Metadata => go!(metadata::Metadata::new(files, false)?),
        Kind::PdfMetadata => go!(metadata::Metadata::new(files, true)?),
        Kind::Protect => go!(password::Protect::new(files)),
        Kind::Unlock => go!(password::Unlock::new(files)?),
    }
}

/// Every tool window with test files, for headless renders and scripts.
pub fn headless_windows() -> Vec<rune::AnyWindowSpec> {
    use crate::headless::{fixture, tool_window};
    let mut v = vec![];
    let photo = vec![fixture("photo.jpg")];
    let pdf = vec![fixture("report.pdf")];
    if let Ok((t, sub)) = compress::Compress::new(photo.clone()) {
        v.push(tool_window("compress", t, sub));
    }
    if let Ok((t, sub)) = edit::Edit::new(photo.clone()) {
        v.push(tool_window("edit", t, sub));
    }
    if let Ok((t, sub)) = crop::Crop::new(photo.clone()) {
        v.push(tool_window("crop", t, sub));
    }
    if let Ok((t, sub)) = redact::Redact::new(photo.clone()) {
        v.push(tool_window("redact", t, sub));
    }
    if let Ok((t, sub)) = annotate::Annotate::new(photo.clone()) {
        v.push(tool_window("annotate", t, sub));
    }
    if let Ok((t, sub)) = addbg::AddBg::new(photo.clone()) {
        v.push(tool_window("addbg", t, sub));
    }
    if let Ok((t, sub)) = stamp::Stamp::new(pdf.clone()) {
        v.push(tool_window("stamp", t, sub));
    }
    if let Ok((t, sub)) = organize::Organize::new(vec![fixture("report.pdf"), fixture("appendix.pdf")]) {
        v.push(tool_window("organize", t, sub));
    }
    if let Ok((t, sub)) = resize::Resize::new(photo.clone()) {
        v.push(tool_window("resize", t, sub));
    }
    if let Ok((t, sub)) = metadata::Metadata::new(photo.clone(), false) {
        v.push(tool_window("metadata", t, sub));
    }
    if let Ok((t, sub)) = metadata::Metadata::new(pdf.clone(), true) {
        v.push(tool_window("pdf-metadata", t, sub));
    }
    let (t, sub) = password::Protect::new(pdf.clone());
    v.push(tool_window("protect", t, sub));
    if let Ok((t, sub)) = password::Unlock::new(vec![fixture("locked.pdf")]) {
        v.push(tool_window("unlock", t, sub));
    }
    v
}

/// A tool window opens in front with the keyboard: it is opened from a drop or a tray click,
/// which Windows doesn't count as the user choosing that window.
pub(crate) fn bring_to_front(h: &rune::WindowHandle) {
    let _ = h.with_raw_handle(|raw| {
        use raw_window_handle::RawWindowHandle;
        if let RawWindowHandle::Win32(w) = raw.as_raw() {
            use windows::Win32::UI::WindowsAndMessaging::{BringWindowToTop, SetForegroundWindow};
            let hwnd = windows::Win32::Foundation::HWND(w.hwnd.get() as *mut _);
            unsafe {
                let _ = BringWindowToTop(hwnd);
                let _ = SetForegroundWindow(hwnd);
            }
        }
    });
    h.focus();
}

/// The window is the one the user is typing into.
pub(crate) fn is_front(h: &rune::WindowHandle) -> bool {
    h.with_raw_handle(|raw| {
        use raw_window_handle::RawWindowHandle;
        match raw.as_raw() {
            RawWindowHandle::Win32(w) => unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow().0 as isize == w.hwnd.get() },
            _ => true,
        }
    })
    .unwrap_or(true)
}
