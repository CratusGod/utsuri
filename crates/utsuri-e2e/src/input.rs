//! Real input (SendInput) and window helpers for the E2E run.

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, HSTRING};

pub const VK_CONTROL_: u16 = 0x11;
pub const VK_SHIFT_: u16 = 0x10;
pub const VK_MENU_: u16 = 0x12;
pub const VK_RETURN_: u16 = 0x0D;
pub const VK_TAB_: u16 = 0x09;

pub fn sleep(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

fn send(inputs: &[INPUT]) {
    unsafe {
        SendInput(inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

fn mouse(flags: MOUSE_EVENT_FLAGS, x: i32, y: i32) -> INPUT {
    INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: x, dy: y, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

/// Absolute coordinates for SendInput (0..65535 over the virtual screen).
fn abs(p: (i32, i32)) -> (i32, i32) {
    unsafe {
        let (vx, vy) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
        let (vw, vh) = (GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1), GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1));
        (((p.0 - vx) * 65535 + vw / 2) / (vw - 1).max(1), ((p.1 - vy) * 65535 + vh / 2) / (vh - 1).max(1))
    }
}

pub fn move_to(p: (i32, i32)) {
    let (x, y) = abs(p);
    send(&[mouse(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK, x, y)]);
}

/// Move in `steps` small moves (drag sources and drop targets need real motion).
pub fn glide(a: (i32, i32), b: (i32, i32), steps: i32) {
    for i in 1..=steps.max(1) {
        let t = i as f32 / steps.max(1) as f32;
        move_to(((a.0 as f32 + (b.0 - a.0) as f32 * t) as i32, (a.1 as f32 + (b.1 - a.1) as f32 * t) as i32));
        sleep(16);
    }
}

pub fn left_down() {
    send(&[mouse(MOUSEEVENTF_LEFTDOWN, 0, 0)]);
}

pub fn left_up() {
    send(&[mouse(MOUSEEVENTF_LEFTUP, 0, 0)]);
}

pub fn click(p: (i32, i32)) {
    move_to(p);
    sleep(60);
    left_down();
    sleep(60);
    left_up();
}

fn key(vk: u16, up: bool) -> INPUT {
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 } } }
}

pub fn key_down(vk: u16) {
    send(&[key(vk, false)]);
}

pub fn key_up(vk: u16) {
    send(&[key(vk, true)]);
}

pub fn press(vk: u16) {
    key_down(vk);
    sleep(40);
    key_up(vk);
    sleep(40);
}

/// Shift down, held long enough for a 15 ms poll to see it, then up.
/// The trigger key under test (Shift, or Ctrl with --ctrl-folder).
pub static TRIGGER: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(VK_SHIFT_);

pub fn tap_shift() {
    let k = TRIGGER.load(std::sync::atomic::Ordering::Relaxed);
    key_down(k);
    sleep(140);
    key_up(k);
    sleep(100);
}

pub fn key_combo(mods: &[u16], k: u16) {
    for m in mods {
        key_down(*m);
    }
    sleep(40);
    press(k);
    for m in mods.iter().rev() {
        key_up(*m);
    }
    sleep(60);
}

/// Type text as Unicode key events (works for any language).
pub fn type_text(s: &str) {
    for u in s.encode_utf16() {
        let k = |up: bool| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: u, dwFlags: KEYEVENTF_UNICODE | if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 } } };
        send(&[k(false), k(true)]);
        sleep(8);
    }
    sleep(60);
}

// ---------------------------------------------------------------- windows

pub fn find_window(title: &str) -> Option<HWND> {
    let h = unsafe { FindWindowW(None, &HSTRING::from(title)) }.ok()?;
    (!h.is_invalid()).then_some(h)
}

pub fn window_title(h: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(h, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn all_windows() -> Vec<HWND> {
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        let v = unsafe { &mut *(l.0 as *mut Vec<HWND>) };
        v.push(h);
        BOOL(1)
    }
    let mut v: Vec<HWND> = vec![];
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

pub fn find_explorer(title: &str) -> Option<HWND> {
    all_windows().into_iter().find(|h| class_name(*h) == "CabinetWClass" && title_is(&window_title(*h), title) && is_visible(*h))
}

pub fn is_visible(h: HWND) -> bool {
    unsafe { IsWindowVisible(h).as_bool() }
}

pub fn window_rect(h: HWND) -> (i32, i32, i32, i32) {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(h, &mut r);
    }
    (r.left, r.top, r.right - r.left, r.bottom - r.top)
}

pub fn place(h: HWND, x: i32, y: i32, w: i32, hh: i32) {
    unsafe {
        let _ = ShowWindow(h, SW_RESTORE);
        let _ = SetWindowPos(h, None, x, y, w, hh, SWP_NOZORDER);
    }
}

pub fn foreground() -> HWND {
    unsafe { GetForegroundWindow() }
}

pub fn foreground_title() -> String {
    window_title(foreground())
}

/// Bring a window to the front. Windows only lets the process with the last input do
/// that, so a harmless Alt tap comes first.
pub fn focus(h: HWND) {
    // already in front: an Alt tap would only put that window into menu mode
    if foreground() == h {
        return;
    }
    unsafe {
        press(VK_MENU_);
        let _ = ShowWindow(h, SW_SHOW);
        let _ = SetForegroundWindow(h);
    }
}

pub fn close_window(h: HWND) {
    unsafe {
        let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

/// Close any Utsuri tool window left open by a failed case.
pub fn close_tool_windows() {
    for h in all_windows() {
        if window_title(h).ends_with(" — Utsuri") && is_visible(h) {
            close_window(h);
        }
    }
}

/// Explorer titles are "<folder>" or "<folder> - File Explorer".
pub fn title_is(title: &str, folder: &str) -> bool {
    title == folder || title.strip_prefix(folder).is_some_and(|r| r.starts_with(" - "))
}

/// Save a screen region as PNG (debugging: only ever the test Explorer window or menu).
pub fn shot(x: i32, y: i32, w: i32, h: i32, out: &std::path::Path) {
    use windows::Win32::Graphics::Gdi::*;
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp.into());
        let _ = BitBlt(mem, 0, 0, w, h, Some(screen), x, y, SRCCOPY);
        let mut info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        GetDIBits(mem, bmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        for p in buf.chunks_exact_mut(4) {
            p.swap(0, 2);
            p[3] = 255;
        }
        if let Some(im) = image::RgbaImage::from_raw(w as u32, h as u32, buf) {
            let _ = im.save(out);
        }
    }
}

pub fn foreground_class() -> String {
    let h = foreground();
    let mut pid = 0u32;
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(h, Some(&mut pid));
    }
    format!("class {} pid {pid} hwnd {:?}", class_name(h), h.0)
}

/// The top-level window under the pointer: its title, class and frame (debugging a missed click).
pub fn under_pointer() -> String {
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, GetCursorPos, WindowFromPoint};
    unsafe {
        let mut p = windows::Win32::Foundation::POINT::default();
        let _ = GetCursorPos(&mut p);
        let h = GetAncestor(WindowFromPoint(p), GA_ROOT);
        format!("{:?} class {} frame {:?} at {:?}", window_title(h), class_name(h), window_rect(h), (p.x, p.y))
    }
}

/// The top-level window a click at `p` would reach.
pub fn root_at(p: (i32, i32)) -> HWND {
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, WindowFromPoint};
    unsafe { GetAncestor(WindowFromPoint(windows::Win32::Foundation::POINT { x: p.0, y: p.1 }), GA_ROOT) }
}
