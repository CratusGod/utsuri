//! A tool window: frameless and solid (Windows 11 rounds it) with its own title bar and ✕, a size
//! grip, Esc to close (window.py). Each tool is a `Tool`; the window's state is `ToolS<T>`.

use super::*;
use rune::{AppHandle, Cx, Tx};
use std::path::PathBuf;

/// Height of the window's own title bar (logical px).
pub const TITLE_BAR: f32 = 46.0;

pub trait Tool: Sized + 'static {
    fn title(&self) -> String;
    /// Initial size in logical pixels.
    fn size(&self) -> (f32, f32) {
        (820.0, 680.0)
    }
    /// The window's content under the title bar.
    fn body(s: &ToolS<Self>, cx: &mut Cx<ToolS<Self>>) -> El;
    /// The primary action (Enter, or the primary button).
    fn enter(_s: &mut ToolS<Self>, _tx: &mut Tx) {}
    /// Background work landed: take it. True if anything changed.
    fn poll(_s: &mut ToolS<Self>, _tx: &mut Tx) -> bool {
        false
    }
    /// A key the window didn't use (Esc and Enter are the frame's). True if taken.
    fn key(_s: &mut ToolS<Self>, _k: &rune::KeyEvent, _tx: &mut Tx) -> bool {
        false
    }
    /// Start background work once the window exists.
    fn setup(_s: &mut ToolS<Self>, _waker: rune::Waker) {}
}

pub struct ToolS<T> {
    pub t: T,
    pub theme: Theme,
    pub kit: rune_kit::Kit,
    pub app: Option<AppHandle<crate::Hub>>,
    /// What the window works on, for its subtitle.
    pub sub: String,
}

impl<T> ToolS<T> {
    pub fn new(t: T, theme: Theme, sub: String) -> ToolS<T> {
        ToolS { t, kit: kit(&theme), theme, app: None, sub }
    }

    /// Run in the background with a spinner card, show the result, and close the window.
    pub fn submit(&mut self, tx: &mut Tx, action: &str, srcs: &[PathBuf], working: &str, extra: String, run: impl FnOnce() -> anyhow::Result<Vec<PathBuf>> + Send + 'static) {
        if let Some(app) = self.app.clone() {
            let (action, srcs, working) = (action.to_string(), srcs.to_vec(), working.to_string());
            app.update(move |h, _| h.submit(action, srcs, working, extra, Box::new(run)));
        }
        tx.close_window();
    }

    pub fn error(&self, title: &str, msg: String) {
        if let Some(app) = self.app.clone() {
            let title = title.to_string();
            app.update(move |h, _| h.error(&title, msg));
        }
    }

    pub fn settings_changed(&self) {
        if let Some(app) = self.app.clone() {
            app.update(|h, _| h.settings_changed());
        }
    }
}

/// Start resizing from the bottom-right corner, as a native frame would (the window is
/// frameless): the active window is ours, since its grip was just pressed.
fn start_resize() {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetActiveWindow, ReleaseCapture};
    use windows::Win32::UI::WindowsAndMessaging::{HTBOTTOMRIGHT, PostMessageW, WM_NCLBUTTONDOWN};
    unsafe {
        let h = GetActiveWindow();
        if !h.is_invalid() {
            let _ = ReleaseCapture();
            let _ = PostMessageW(Some(h), WM_NCLBUTTONDOWN, WPARAM(HTBOTTOMRIGHT as usize), LPARAM(0));
        }
    }
}

pub fn view<T: Tool>(s: &ToolS<T>, cx: &mut Cx<ToolS<T>>) -> El {
    let th = &s.theme;
    let title = s.t.title();
    let bar = row()
        .key("title-bar")
        .w_full()
        .h(TITLE_BAR)
        .pl(20.0)
        .pr(10.0)
        .items_center()
        .window_drag()
        .child(text(title).key("title").font(TITLE).color(col_of(th.text)))
        .child(rune::spacer())
        .child(
            el().key("close")
                .size(36.0, 28.0)
                .radius(7.0)
                .center()
                .hover(col_of(Rgba(255, 80, 100, 153)))
                .cursor(rune::Cursor::Pointer)
                .role("button")
                .label("Close")
                .help("Close (Esc)")
                .child(text("✕").font(font(14.0, 400.0)).color(col_of(th.text_dim)))
                .on_press_tx(|_s: &mut ToolS<T>, tx: &mut Tx| tx.close_window()),
        );
    let mut body = col().key("body").w_full().grow(1.0).min_h(0.0).px(22.0).pb(18.0).gap(10.0);
    if !s.sub.is_empty() {
        body = body.child(dim(&s.sub, th));
    }
    body = body.child(T::body(s, cx));
    let grip = {
        let mut g = el().key("grip").absolute(0.0, 0.0).size(18.0, 18.0).cursor(rune::Cursor::Move);
        for (dx, dy) in [(12.0, 12.0), (7.0, 12.0), (12.0, 7.0), (2.0, 12.0), (7.0, 7.0), (12.0, 2.0)] {
            g = g.child(el().absolute(dx + 1.5, dy + 1.5).size(2.6, 2.6).radius(1.3).fill(white(0.35)));
        }
        g.on_drag(|_s: &mut ToolS<T>, d, _tx| {
            if matches!(d.phase, rune::Phase::Down) {
                start_resize();
            }
        })
    };
    col()
        .key("window")
        .w_full()
        .h_full()
        // solid (2026-11-14): the see-through glass flickered, cost CPU (a screen capture per
        // move) and its corners didn't match; Windows 11 rounds the window and draws its border
        .fill(col_of(th.surface))
        .child(bar)
        .child(body)
        .child(el().pin(None, None, Some(4.0), Some(4.0)).size(18.0, 18.0).child(grip))
}

/// Keys no control used, whether or not anything has focus: Esc closes, Enter runs the tool.
fn tool_key<T: Tool>(s: &mut ToolS<T>, k: &rune::KeyEvent, tx: &mut Tx) -> bool {
    log::debug!("{}: key {:?}", s.t.title(), k.key);
    T::key(s, k, tx)
        || match k.key {
            rune::Key::Escape => {
                tx.close_window();
                true
            }
            rune::Key::Enter => {
                T::enter(s, tx);
                true
            }
            _ => false,
        }
}

/// The bottom row: `extra` on the left, then Cancel and the primary button (Enter).
pub fn footer<T: Tool>(s: &ToolS<T>, primary_label: &str, enabled: bool, extra: Option<El>) -> El {
    let th = &s.theme;
    let mut r = row().key("footer").w_full().items_center().gap(8.0).pt(6.0);
    if let Some(e) = extra {
        r = r.child(e);
    }
    let p = super::primary("primary", primary_label, enabled, th);
    let p = if enabled { p.on_press_tx(|s: &mut ToolS<T>, tx: &mut Tx| T::enter(s, tx)) } else { p };
    r.child(rune::spacer()).child(super::pill("cancel", "Cancel", th).on_press_tx(|_s: &mut ToolS<T>, tx: &mut Tx| tx.close_window())).child(p)
}

fn tool_poll<T: Tool>(s: &mut ToolS<T>, _t: f64, tx: &mut Tx) -> bool {
    T::poll(s, tx)
}

fn tool_setup<T: Tool>(s: &mut ToolS<T>, waker: rune::Waker) {
    T::setup(s, waker)
}

/// The window spec of a tool, ready for `rune::open_window` (or a headless run).
pub fn spec<T: Tool>(key: String, state: ToolS<T>) -> rune::WindowSpec<ToolS<T>> {
    let (w, h) = state.t.size();
    let title: &'static str = Box::leak(format!("{} — Utsuri", state.t.title()).into_boxed_str());
    let spec = rune::AppSpec { setup: Some(tool_setup::<T>), poll: Some(tool_poll::<T>), on_key: Some(tool_key::<T>), min_size: Some((420.0, 260.0)), ..rune::AppSpec::new(title, (w, h), || unreachable!("tool state comes from init"), view::<T>) };
    let mut state = Some(state);
    // a closed tool window gives back its GPU device; kept, every one ever opened stayed alive
    // and each new window took longer to open (3.4 s, rising to 6 s over a dozen)
    let app = rune::App::new(spec).init(move || state.take().expect("one init")).release_gpu_when_closed();
    rune::WindowSpec::from_app(key, app).options(rune::WindowOptions { transparent: false, borderless: true, min_size: Some((420.0, 260.0)), ..Default::default() })
}
