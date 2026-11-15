//! Headless runs (Rune's tooling: `--snap`, `--script`, `--tree`, `--list-windows`, video):
//! every surface of the app as a window with demo state, over test files made in a temp
//! folder. No window opens, no input is touched.
//!
//!   utsuri --list-windows
//!   utsuri --window menu --snap menu.png --scale 2
//!   utsuri --window settings --script s.json --tree tree.json

use crate::ui::frame::{Tool, ToolS};
use crate::ui::menu::{Hit, MenuS};
use crate::ui::toasts::{Body, ToastS};
use rune::WindowSpec;
use std::path::PathBuf;
use std::sync::OnceLock;
use utsuri_engine::report::Report;

const FLAGS: &[&str] = &["--snap", "--script", "--tree", "--list-windows", "--list-scenes", "--video", "--out", "--strip", "--first-pixels"];

pub fn wanted(args: &[String]) -> bool {
    args.iter().any(|a| FLAGS.contains(&a.as_str())) && !args.iter().any(|a| a == "--native")
}

/// Each run writes its own copies: runs back to back (or side by side) found the shared
/// folder's PDFs still locked by the run before, and lost the windows that need them.
fn fixtures_dir() -> PathBuf {
    std::env::temp_dir().join(format!("utsuri-headless-{}", std::process::id()))
}

/// Test files (made once per run).
pub fn fixtures() -> &'static Vec<PathBuf> {
    static F: OnceLock<Vec<PathBuf>> = OnceLock::new();
    F.get_or_init(|| {
        let dir = fixtures_dir();
        let _ = std::fs::create_dir_all(&dir);
        utsuri_engine::fixtures::write_all(&dir).unwrap_or_default()
    })
}

pub fn fixture(name: &str) -> PathBuf {
    fixtures().iter().find(|p| p.file_name().is_some_and(|n| n == name)).cloned().unwrap_or_else(|| PathBuf::from(name))
}

fn theme() -> shared::theme::Theme {
    let name = std::env::var("UTSURI_THEME").unwrap_or_else(|_| shared::theme::DEFAULT.to_string());
    shared::theme::by_name(&name)
}

fn menu_state(file: &str, ring: usize, hover: Option<Hit>, click: bool) -> MenuS {
    let p = fixture(file);
    let mut m = MenuS::new(theme());
    m.visible = true;
    m.got_drag = true;
    m.click_mode = click;
    m.key = if click { String::new() } else { "Shift".into() };
    m.rings = utsuri_engine::actions::rings(std::slice::from_ref(&p));
    m.paths = vec![p];
    m.ring = ring.min(m.rings.len().saturating_sub(1));
    m.hover = hover;
    m
}

fn clear() -> rune::WindowOptions {
    rune::WindowOptions { transparent: true, borderless: true, min_size: Some((0.0, 0.0)), ..Default::default() }
}

fn menu_window(key: &str, state: MenuS) -> rune::AnyWindowSpec {
    let mut state = Some(state);
    let title: &'static str = Box::leak(key.to_string().into_boxed_str());
    let app = rune::App::new(rune::AppSpec::new(title, (crate::ui::menu::SIZE, crate::ui::menu::SIZE), || unreachable!(), crate::ui::menu::view)).init(move || state.take().unwrap());
    WindowSpec::from_app(key, app).options(clear()).boxed()
}

fn toasts_window() -> rune::AnyWindowSpec {
    toasts_window_with("toasts", false)
}

/// `update`: the update card alone (its buttons are scripted).
fn toasts_window_with(key: &'static str, update: bool) -> rune::AnyWindowSpec {
    let mut s = ToastS::new(theme());
    if update {
        s.stack.push_id(1, Body::Update { version: "2.0.1".into() });
        let (w, h) = s.stack.window_size();
        let mut s = Some(s);
        let app = rune::App::new(rune::AppSpec::new(key, (w, h), || unreachable!(), crate::ui::toasts::view)).init(move || s.take().unwrap());
        return WindowSpec::from_app(key, app).options(clear()).boxed();
    }
    let photo = fixture("photo.jpg");
    let thumb = utsuri_engine::imgio::open_preview(&photo, 128).ok().map(|im| crate::ui::pixels_of(&im));
    let rep = Report::new("JPG → WEBP", "photo.webp · 1600×1067 · 212 KB", vec![photo.with_extension("webp")], vec![photo.clone()]).good("75% smaller");
    s.stack.push_id(1, Body::Done(Box::new(rep), thumb));
    s.stack.push_id(2, Body::Working { title: "Compressing…".into(), detail: "holiday (12 files)".into() });
    s.stack.push_id(3, Body::Error { title: "Couldn't finish".into(), msg: "report.pdf is protected with a password. Unlock it first (Tools → Unlock).".into() });
    let (w, h) = s.stack.window_size();
    let mut s = Some(s);
    let app = rune::App::new(rune::AppSpec::new("toasts", (w, h), || unreachable!(), crate::ui::toasts::view)).init(move || s.take().unwrap());
    WindowSpec::from_app("toasts", app).options(clear()).boxed()
}

pub fn tool_window<T: Tool>(key: &str, t: T, sub: String) -> rune::AnyWindowSpec {
    let s = ToolS::new(t, theme(), sub);
    crate::ui::frame::spec(key.to_string(), s).boxed()
}

pub fn run() {
    let mut windows = vec![
        menu_window("menu", menu_state("photo.jpg", 0, Some(Hit::Slice(1)), false)),
        menu_window("menu-tools", menu_state("photo.jpg", 1, Some(Hit::Slice(2)), false)),
        menu_window("menu-hub", menu_state("report.pdf", 0, Some(Hit::Hub), false)),
        menu_window("menu-click", menu_state("letter.docx", 0, None, true)),
        toasts_window(),
        toasts_window_with("toast-update", true),
        tool_window("settings", crate::tools::settings::SettingsWindow::new(), String::new()),
        tool_window("welcome", crate::tools::settings::Welcome { key: "Shift".into() }, String::new()),
    ];
    windows.extend(crate::tools::headless_windows());
    rune::run_windows(move |_| windows);
    // a thumbnail worker may still be inside Windows.Data.Pdf: exiting under it crashed
    utsuri_engine::winrt::shutdown(std::time::Duration::from_secs(3));
    let _ = std::fs::remove_dir_all(fixtures_dir());
}
