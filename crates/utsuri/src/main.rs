//! Utsuri (Rust; Windows and Linux): start dragging a file, press Shift, drop it on a slice.
//! Ported from the Python app's app.py; the file work lives in utsuri-engine. The UI is Rune:
//! a windowless app (`Hub`, the logic) with two overlays (the radial menu and the
//! notifications) and a window per tool.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
#[cfg(windows)]
mod crash;
#[cfg(not(windows))]
#[path = "linux/crash.rs"]
mod crash;
#[cfg(windows)]
mod droptarget;
#[cfg(not(windows))]
#[path = "linux/droptarget.rs"]
mod droptarget;
mod headless;
#[cfg(windows)]
mod ipc;
#[cfg(not(windows))]
#[path = "linux/ipc.rs"]
mod ipc;
#[cfg(target_os = "linux")]
mod linux;
mod settings;
#[cfg(windows)]
mod shell;
#[cfg(not(windows))]
#[path = "linux/shell.rs"]
mod shell;
mod tools;
mod trigger;
mod ui;

use anyhow::Result;
use rune::{AppHandle, OverlayHandle, OverlayOptions, OverlayPlacement, OverlaySpec, Tx};
use settings::Settings;
use shared::theme::Theme;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use ui::menu::{Hit, MenuS};
use ui::toasts::{Action, Body, ToastS};
use utsuri_engine::actions::{Plan, Slice};
use utsuri_engine::report::Report;

const TICK: Duration = Duration::from_millis(15);
/// After the button goes up, give the drop event this long to land before hiding.
const DROP_GRACE: Duration = Duration::from_millis(120);
/// Linux: how long a drop's files may take to arrive from the source app.
#[cfg(target_os = "linux")]
const DROP_WAIT: Duration = Duration::from_secs(3);
/// A job shows a spinner card only after this long.
const SPINNER_AFTER: Duration = Duration::from_millis(150);
/// Explorer starts us once per selected file: gather them this long, then one menu.
const GATHER: Duration = Duration::from_millis(280);

// ------------------------------------------------------------------ logging

struct FileLog(std::sync::Mutex<Option<std::fs::File>>);

impl log::Log for FileLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.target().starts_with("utsuri") || m.level() <= log::Level::Warn
    }
    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        // one target for the whole app, so the log reads (and the E2E run parses) the same
        let target = if r.target().starts_with("utsuri::") { "utsuri" } else { r.target() };
        let line = format!("{}.{:03} {} {}: {}\n", t.as_secs(), t.subsec_millis(), r.level(), target, r.args());
        eprint!("{line}");
        if let Some(f) = self.0.lock().unwrap().as_mut() {
            use std::io::Write;
            let _ = f.write_all(line.as_bytes());
        }
    }
    fn flush(&self) {}
}

pub fn log_dir() -> PathBuf {
    shared::paths::state_dir("Utsuri-rs")
}

fn init_log() {
    let dir = log_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = std::env::var_os("UTSURI_LOG").map(PathBuf::from).unwrap_or_else(|| dir.join("log.txt"));
    // keep the log small: start over past 2 MB
    if std::fs::metadata(&path).map(|m| m.len() > 2 << 20).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    let file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok();
    let logger = Box::leak(Box::new(FileLog(std::sync::Mutex::new(file))));
    let _ = log::set_logger(logger);
    log::set_max_level(log::LevelFilter::Debug);
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
}

// ------------------------------------------------------------------ the app

struct Job {
    working: String,
    names: String,
    started: Instant,
    toast: Option<u64>,
}

/// The menu as the app tracks it (the overlay gets a copy of what it draws).
#[derive(Default)]
struct Menu {
    visible: bool,
    click_mode: bool,
    origin: (i32, i32),
    scale: f32,
    paths: Vec<PathBuf>,
    rings: Vec<(String, Vec<Slice>)>,
    ring: usize,
    hover: Option<Hit>,
    got_drag: bool,
    hide_at: Option<Instant>,
    /// Shift taps that came before the files were known (applied once they are)
    early_taps: usize,
}

/// A tool window the app keeps an eye on.
struct ToolEntry {
    window: rune::WindowHandle,
    at: Option<((i32, i32), (u32, u32))>,
    /// when the window first showed: it keeps itself in front for a second after
    opened: Option<Instant>,
    /// when it was asked for (the drop), to log how long it took to show
    asked: Instant,
}

pub struct Hub {
    settings: Settings,
    theme: Theme,
    me: Option<AppHandle<Hub>>,
    menu_ov: Option<OverlayHandle<MenuS>>,
    toast_ov: Option<OverlayHandle<ToastS>>,
    menu: Menu,
    watcher: trigger::Watcher,
    jobs: HashMap<u64, Job>,
    next_job: u64,
    /// Toast ids are given here, so jobs can find their card before the overlay has it.
    next_toast: u64,
    toast_count: usize,
    toast_rects: Vec<(u64, &'static str, [f32; 4])>,
    toast_geom: (f32, f32, f32, f32, f32),
    e2e_last_buttons: String,
    pending: Vec<PathBuf>,
    pending_at: Option<Instant>,
    last_poll: Instant,
    tools: Vec<ToolEntry>,
    tray: Option<rune::tray::Tray>,
    /// the menu is shown once off screen at startup, so its first real show is instant
    warm_until: Option<Instant>,
    last_expire: Instant,
}

fn names_of(srcs: &[PathBuf]) -> String {
    if srcs.len() == 1 { srcs[0].file_name().unwrap_or_default().to_string_lossy().into_owned() } else { format!("{} files", srcs.len()) }
}

/// The app icon (the 移 glyph on the glass tile) as straight RGBA.
pub fn icon_rgba(side: u32) -> Vec<u8> {
    let im = image::load_from_memory(include_bytes!("../assets/utsuri.png")).expect("bundled icon");
    im.resize_exact(side, side, image::imageops::FilterType::Lanczos3).to_rgba8().into_raw()
}

/// Linux: pointer and placement both use the compositor's layout px, which are already
/// points, so nothing is scaled.
#[cfg(not(windows))]
fn scale_at(_x: i32, _y: i32) -> f32 {
    1.0
}

/// Physical px per point on the monitor at a point.
#[cfg(windows)]
fn scale_at(x: i32, y: i32) -> f32 {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    unsafe {
        let m = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (96u32, 96u32);
        if GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_ok() { dx as f32 / 96.0 } else { 1.0 }
    }
}

impl Hub {
    fn new() -> Hub {
        let settings = Settings::load();
        settings.apply();
        let theme = shared::theme::by_name(&settings.theme);
        let mut watcher = trigger::Watcher::new();
        watcher.enabled = true;
        Hub {
            settings,
            theme,
            me: None,
            menu_ov: None,
            toast_ov: None,
            menu: Menu::default(),
            watcher,
            jobs: HashMap::new(),
            next_job: 0,
            next_toast: 0,
            toast_count: 0,
            toast_rects: vec![],
            toast_geom: (0.0, 0.0, 0.0, 0.0, 1.0),
            e2e_last_buttons: String::new(),
            pending: vec![],
            pending_at: None,
            last_poll: Instant::now(),
            tools: vec![],
            tray: None,
            warm_until: None,
            last_expire: Instant::now(),
        }
    }

    fn tooltip(&self) -> String {
        format!("Utsuri — press {} while dragging a file", self.settings.trigger_label())
    }

    // -------------------------------------------------------------- menu

    /// Show the (empty) menu far off screen for a moment: the first show of an overlay makes
    /// its surface, which took most of a second and lost the first drag.
    fn warm_menu(&mut self) {
        if let Some(ov) = &self.menu_ov {
            ov.update(|m, _| m.ghost = true);
            ov.set_placement(Some(OverlayPlacement::Rect { x: 0.0, y: 0.0, width: ui::menu::SIZE, height: ui::menu::SIZE }));
            ov.show();
            self.warm_until = Some(Instant::now() + Duration::from_millis(600));
        }
    }

    fn show_menu(&mut self, pos: (i32, i32), click_mode: bool) {
        let Some(ov) = self.menu_ov.clone() else { return };
        let scale = scale_at(pos.0, pos.1);
        let side = (ui::menu::SIZE * scale).round() as i32;
        // keep the whole menu on the screen in click mode
        let (mut x, mut y) = (pos.0 - side / 2, pos.1 - side / 2);
        if click_mode {
            let (wx, wy, ww, wh) = shell::work_area_at(pos.0, pos.1);
            x = x.clamp(wx, wx + ww - side);
            y = y.clamp(wy, wy + wh - side);
        }
        self.menu = Menu { visible: true, click_mode, origin: (x, y), scale, ..Default::default() };
        let key = if click_mode { String::new() } else { self.settings.trigger_label().to_string() };
        let theme = self.theme.clone();
        let me = self.me.clone();
        ov.update(move |m, _| {
            *m = MenuS { visible: true, click_mode, key, theme, app: me, ..MenuS::new(m.theme.clone()) };
        });
        ov.set_placement(Some(OverlayPlacement::Rect { x: x as f32 / scale, y: y as f32 / scale, width: ui::menu::SIZE, height: ui::menu::SIZE }));
        ov.show();
        // Wayland: an invisible surface over the menu takes the drag (Rune's overlays can't)
        #[cfg(target_os = "linux")]
        if !click_mode {
            linux::dnd::show(x, y, side, side);
        }
        if click_mode {
            ov.request_keys();
        }
        log::debug!("menu at {pos:?} scale {scale} click={click_mode}");
    }

    pub fn hide_menu(&mut self) {
        if self.menu.visible {
            log::debug!("menu hidden (drag={})", self.menu.got_drag);
        }
        if let Some(ov) = &self.menu_ov {
            ov.hide();
            ov.update(|m, _| m.visible = false);
        }
        #[cfg(target_os = "linux")]
        linux::dnd::hide();
        self.menu.visible = false;
        self.menu.hide_at = None;
    }

    /// Copy what the menu draws into its overlay.
    fn push_menu(&self) {
        let Some(ov) = &self.menu_ov else { return };
        let (paths, rings, ring, hover, got_drag) = (self.menu.paths.clone(), self.menu.rings.clone(), self.menu.ring, self.menu.hover, self.menu.got_drag);
        ov.update(move |m, _| {
            m.paths = paths;
            m.rings = rings;
            m.ring = ring;
            if !m.click_mode {
                m.hover = hover;
            }
            m.got_drag = got_drag;
        });
    }

    fn set_paths(&mut self, paths: Vec<PathBuf>) {
        self.menu.rings = utsuri_engine::actions::rings(&paths);
        self.menu.paths = paths;
        self.menu.ring = 0;
    }

    fn items(&self) -> &[Slice] {
        self.menu.rings.get(self.menu.ring).map_or(&[], |r| r.1.as_slice())
    }

    fn hover_from_cursor(&mut self, cursor: (i32, i32)) -> bool {
        let m = &self.menu;
        let s = m.scale.max(0.1);
        let (lx, ly) = ((cursor.0 - m.origin.0) as f32 / s, (cursor.1 - m.origin.1) as f32 / s);
        let h = Some(ui::menu::hit(lx - ui::menu::SIZE / 2.0, ly - ui::menu::SIZE / 2.0, self.items().len()));
        let changed = h != self.menu.hover;
        self.menu.hover = h;
        changed
    }

    fn next_ring(&mut self) {
        if self.menu.rings.len() > 1 {
            self.menu.ring = (self.menu.ring + 1) % self.menu.rings.len();
            self.menu.hover = None;
            self.push_menu();
        } else if self.menu.paths.is_empty() {
            self.menu.early_taps += 1;
        }
    }

    /// The drag reached the menu (a file is being dragged, not just Shift held).
    pub fn menu_got_drag(&mut self) {
        if !self.menu.got_drag {
            self.menu.got_drag = true;
            self.push_menu();
        }
    }

    /// A file drag entered the menu (from its own drop target): show the rings for the files.
    pub fn menu_drag_entered(&mut self, files: Vec<PathBuf>) {
        if self.menu.paths != files {
            self.set_paths(files);
            let taps = std::mem::take(&mut self.menu.early_taps);
            if taps > 0 && self.menu.rings.len() > 1 {
                self.menu.ring = (self.menu.ring + taps) % self.menu.rings.len();
            }
        }
        self.menu.got_drag = true;
        self.push_menu();
    }

    /// A drop landed on the menu and its files are on the way (Wayland hands them over
    /// after the button is up): keep the menu until they arrive.
    #[cfg(target_os = "linux")]
    pub fn menu_drop_pending(&mut self) {
        if self.menu.visible {
            self.menu.hide_at = Some(Instant::now() + DROP_WAIT);
        }
    }

    /// Files dropped on the menu: run the slice under the pointer.
    pub fn menu_dropped(&mut self, files: Vec<PathBuf>, tx: &mut Tx) {
        // where the pointer is now decides, as in the drag the user just made
        let c = shell::cursor();
        self.hover_from_cursor(c);
        if self.menu.paths.is_empty() {
            self.set_paths(files.clone());
        }
        self.choose(files, tx);
    }

    /// A click in click mode (tray, right-click, Send To).
    pub fn menu_clicked(&mut self, hover: Option<Hit>, tx: &mut Tx) {
        match hover {
            Some(Hit::Hub) => self.next_ring(),
            Some(Hit::Slice(_)) => {
                self.menu.hover = hover;
                let paths = self.menu.paths.clone();
                self.choose(paths, tx);
            }
            _ => self.hide_menu(),
        }
    }

    /// Run the hovered slice on `paths`.
    fn choose(&mut self, paths: Vec<PathBuf>, tx: &mut Tx) {
        let Some(Hit::Slice(i)) = self.menu.hover else {
            log::info!("dropped outside a slice: nothing to do");
            self.hide_menu();
            return;
        };
        let Some(item) = self.items().get(i).cloned() else { return };
        if paths != self.menu.paths {
            let rings = utsuri_engine::actions::rings(&paths);
            if rings.get(self.menu.ring).and_then(|r| r.1.get(i)).map(|s| &s.key) != Some(&item.key) {
                log::warn!("the dropped files' menu differs from the one shown; using the shown slice {}", item.key);
            }
        }
        self.hide_menu();
        self.run_slice(&item.key, &paths, tx);
    }

    // -------------------------------------------------------------- jobs

    fn run_slice(&mut self, key: &str, paths: &[PathBuf], tx: &mut Tx) {
        log::info!("{key} on {} file(s)", paths.len());
        match utsuri_engine::actions::plan(key, paths) {
            Ok(Plan::Job { action, working, run }) => self.spawn(action, paths.to_vec(), working, run),
            Ok(Plan::Window { tool, files }) => self.open_tool(tool, files, tx),
            Err(e) => self.error("Couldn't start that", utsuri_engine::friendly(&e)),
        }
    }

    pub fn submit(&mut self, action: String, srcs: Vec<PathBuf>, working: String, extra: String, run: Box<dyn FnOnce() -> anyhow::Result<Vec<PathBuf>> + Send>) {
        let (a2, s2) = (action.clone(), srcs.clone());
        self.spawn(action, srcs, working, Box::new(move || {
            let outs = run()?;
            Ok(utsuri_engine::report::summarize(&a2, &s2, &outs, &extra))
        }));
    }

    pub fn spawn(&mut self, action: String, srcs: Vec<PathBuf>, working: String, run: utsuri_engine::actions::JobFn) {
        self.next_job += 1;
        let id = self.next_job;
        self.jobs.insert(id, Job { working, names: names_of(&srcs), started: Instant::now(), toast: None });
        let me = self.me.clone();
        std::thread::Builder::new()
            .name(format!("job-{id}"))
            .spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
                let r = match r {
                    Ok(Ok(rep)) => Ok(rep),
                    Ok(Err(e)) => {
                        log::warn!("{action} failed: {e:#}");
                        Err(utsuri_engine::friendly(&e))
                    }
                    Err(_) => Err("Something went wrong (see the log)".into()),
                };
                // the card's thumbnail is made here, off the UI thread
                let thumb = r.as_ref().ok().and_then(|rep: &Report| rep.thumb().cloned()).and_then(|p| utsuri_engine::imgio::open_preview(&p, 128).ok()).map(|im| ui::pixels_of(&im));
                if let Some(me) = me {
                    me.update(move |h, _| h.job_done(id, r, thumb));
                }
            })
            .expect("spawn job");
    }

    fn job_done(&mut self, id: u64, r: Result<Report, String>, thumb: Option<rune::Pixels>) {
        let job = self.jobs.remove(&id);
        let body = match r {
            Ok(rep) => {
                log::info!("done: {} · {} · {}", rep.title, rep.detail, rep.outs.iter().map(|o| o.display().to_string()).collect::<Vec<_>>().join(" | "));
                if let Some(c) = &rep.clip {
                    if let Err(e) = shell::set_clipboard(c) {
                        log::warn!("clipboard: {e:#}");
                    }
                }
                Body::Done(Box::new(rep), thumb)
            }
            Err(msg) => {
                log::warn!("Couldn't finish: {msg}");
                Body::Error { title: "Couldn't finish".into(), msg }
            }
        };
        match job.and_then(|j| j.toast) {
            Some(t) => self.toasts(move |s| s.stack.set(t, body)),
            None => {
                self.push_toast(body);
            }
        }
    }

    pub fn error(&mut self, title: &str, msg: String) {
        log::warn!("{title}: {msg}");
        self.push_toast(Body::Error { title: title.into(), msg });
    }

    // -------------------------------------------------------------- notifications

    fn push_toast(&mut self, body: Body) -> u64 {
        self.next_toast += 1;
        let id = self.next_toast;
        self.toasts(move |s| s.stack.push_id(id, body));
        id
    }

    /// Change the cards, then size and place their window (bottom-right; hidden when empty).
    fn toasts(&mut self, f: impl FnOnce(&mut ToastS) + Send + 'static) {
        let Some(ov) = self.toast_ov.clone() else { return };
        let me = self.me.clone();
        let ov2 = ov.clone();
        ov.update(move |s, _| {
            f(s);
            let (lw, lh) = s.stack.window_size();
            let count = s.stack.toasts.len();
            let rects = ui::toasts::button_rects(&s.stack);
            if count == 0 {
                if s.placed.take().is_some() {
                    ov2.hide();
                }
                if let Some(me) = me {
                    me.update(|h, _| {
                        h.toast_count = 0;
                        h.toast_rects.clear();
                    });
                }
            } else {
                let (cx, cy) = shell::cursor();
                let (wx, wy, ww, wh) = shell::work_area_at(cx, cy);
                let scale = scale_at(cx, cy);
                let (x, y) = ((wx + ww) as f32 / scale - lw - 4.0, (wy + wh) as f32 / scale - lh - 4.0);
                // same place and size: the capture and the window stay as they are (this runs on
                // every expiry check, and redoing them each time backed the UI thread up)
                let cards = s.stack.total_height();
                if s.placed == Some((x, y, lw, cards)) {
                    return;
                }
                let fresh = s.placed.is_none_or(|p| (p.0, p.1) != (x, y));
                s.placed = Some((x, y, lw, cards));
                if fresh {
                    ov2.set_placement(Some(OverlayPlacement::Rect { x, y, width: lw, height: lh }));
                }
                ov2.show();
                if let Some(me) = me {
                    me.update(move |h, _| {
                        h.toast_count = count;
                        h.toast_rects = rects;
                        h.toast_geom = (x, y, lw, lh, scale);
                        h.log_toast_buttons();
                    });
                }
            }
        });
    }

    /// For the E2E run: where each card's buttons are on screen (physical px).
    fn log_toast_buttons(&mut self) {
        if std::env::var_os("UTSURI_E2E").is_none() || self.toast_rects.is_empty() {
            return;
        }
        let (gx, gy, _, _, s) = self.toast_geom;
        let list: Vec<String> = self.toast_rects.iter().map(|(id, l, r)| format!("{id}:{l}:{}:{}:{}:{}", ((gx + r[0]) * s) as i32, ((gy + r[1]) * s) as i32, (r[2] * s) as i32, (r[3] * s) as i32)).collect();
        let line = list.join(";");
        if line != self.e2e_last_buttons {
            log::info!("toast-buttons {line}");
            self.e2e_last_buttons = line;
        }
    }

    pub fn toast_action(&mut self, a: Action, _tx: &mut Tx) {
        log::debug!("toast action: {}", match &a {
            Action::Open(_) => "open",
            Action::Reveal(_) => "reveal",
            Action::Close(_) => "close",
            Action::Undo(..) => "undo",
        });
        match a {
            Action::Open(p) => shell::open(&p),
            Action::Reveal(p) => shell::reveal(&p),
            Action::Close(id) => self.toasts(move |s| s.stack.remove(id)),
            Action::Undo(id, outs) => match utsuri_engine::out::trash(&outs) {
                Ok(_) => self.toasts(move |s| {
                    if let Some(t) = s.stack.toasts.iter_mut().find(|t| t.id == id) {
                        t.undone = true;
                    }
                }),
                Err(e) => self.error("Couldn't undo", utsuri_engine::friendly(&e)),
            },
        }
    }

    // -------------------------------------------------------------- tool windows

    fn open_tool(&mut self, tool: utsuri_engine::actions::Tool, files: Vec<PathBuf>, _tx: &mut Tx) {
        let at = self.window_place_hint();
        let t = Instant::now();
        let r = tools::open(tool, files, &self.theme, self.me.clone(), at);
        log::debug!("tool set up in {} ms", t.elapsed().as_millis());
        match r {
            Ok(opened) => self.tools.push(opened),
            Err(e) => self.error("Couldn't open that", utsuri_engine::friendly(&e)),
        }
    }

    /// The work area of the monitor under the cursor (physical px), for placing windows.
    fn window_place_hint(&self) -> (i32, i32, i32, i32) {
        let (cx, cy) = shell::cursor();
        shell::work_area_at(cx, cy)
    }

    fn open_settings(&mut self) {
        let at = self.window_place_hint();
        if let Ok(t) = tools::settings::open(&self.theme, self.me.clone(), at) {
            self.tools.push(t);
        }
    }

    fn welcome(&mut self) {
        let at = self.window_place_hint();
        if let Ok(t) = tools::settings::open_welcome(self.settings.trigger_label(), &self.theme, self.me.clone(), at) {
            self.tools.push(t);
        }
    }

    pub fn settings_changed(&mut self) {
        self.settings = Settings::load();
        self.settings.apply();
        self.theme = shared::theme::by_name(&self.settings.theme);
        if let Some(t) = &self.tray {
            t.set_tooltip(self.tooltip());
        }
        let th = self.theme.clone();
        if let Some(ov) = &self.toast_ov {
            let th = th.clone();
            ov.update(move |s, _| {
                s.kit = ui::kit(&th);
                s.theme = th;
            });
        }
    }

    fn open_dialog(&mut self) {
        let me = self.me.clone();
        std::thread::spawn(move || {
            let start = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
            if let Some(files) = rfd::FileDialog::new().set_title("Choose file(s) for Utsuri").set_directory(start).pick_files() {
                if let Some(me) = me {
                    me.update(move |h, _| h.show_click_menu(files));
                }
            }
        });
    }

    fn show_click_menu(&mut self, files: Vec<PathBuf>) {
        let files = utsuri_engine::kinds::resolve_shortcuts(&files);
        let (cx, cy) = shell::cursor();
        let (wx, wy, ww, wh) = shell::work_area_at(cx, cy);
        let inside = cx >= wx && cy >= wy && cx < wx + ww && cy < wy + wh;
        let pos = if inside { (cx, cy) } else { (wx + ww / 2, wy + wh / 2) };
        self.show_menu(pos, true);
        self.menu.got_drag = true;
        self.set_paths(files);
        self.push_menu();
    }

    fn tray_event(&mut self, e: rune::tray::TrayEvent, tx: &mut Tx) {
        match e {
            rune::tray::TrayEvent::Activate { .. } => self.open_dialog(),
            rune::tray::TrayEvent::Menu(id) => match id.as_str() {
                "quit" => tx.quit(),
                "pause" => {
                    self.watcher.enabled = !self.watcher.enabled;
                    if let Some(t) = &self.tray {
                        t.set_menu(tray_menu(self.watcher.enabled));
                    }
                }
                "open" => self.open_dialog(),
                "settings" => self.open_settings(),
                "help" => self.welcome(),
                _ => {}
            },
            _ => {}
        }
    }

    // -------------------------------------------------------------- the tick (every 15 ms)

    fn tick(&mut self, raw: trigger::Raw, _tx: &mut Tx) {
        let started = Instant::now();
        self.tick_inner(raw);
        if started.elapsed() > Duration::from_millis(30) {
            log::debug!("tick took {} ms", started.elapsed().as_millis());
        }
    }

    fn tick_inner(&mut self, raw: trigger::Raw) {
        if self.warm_until.is_some_and(|t| Instant::now() >= t) {
            self.warm_until = None;
            if let Some(ov) = &self.menu_ov {
                ov.update(|m, _| m.ghost = false);
                if !self.menu.visible {
                    ov.hide();
                }
            }
        }
        // files handed over by the pipe, gathered
        if self.pending_at.is_some_and(|t| t.elapsed() >= GATHER) {
            self.pending_at = None;
            let files = std::mem::take(&mut self.pending);
            if !files.is_empty() {
                self.show_click_menu(files);
            }
        }
        // spinners for slow jobs
        let slow: Vec<u64> = self.jobs.iter().filter(|(_, j)| j.toast.is_none() && j.started.elapsed() >= SPINNER_AFTER).map(|(id, _)| *id).collect();
        for id in slow {
            let (title, detail) = {
                let j = &self.jobs[&id];
                (j.working.clone(), j.names.clone())
            };
            let t = self.push_toast(Body::Working { title, detail });
            self.jobs.get_mut(&id).unwrap().toast = Some(t);
        }
        // old cards leave; checked 4 times a second, not every tick
        if self.toast_count > 0 && self.last_expire.elapsed() >= Duration::from_millis(250) {
            self.last_expire = Instant::now();
            self.toasts(|s| {
                s.stack.expire();
            });
        }
        // closed tool windows leave the list (their handles kept what they had alive)
        self.tools.retain(|t| t.at.is_none() || t.window.placement().is_some());
        // a new tool window keeps itself in front for its first second
        for t in &mut self.tools {
            if let Some(p) = t.window.placement() {
                let at = (p.position, p.content_size);
                // the app a file was dragged from takes the front back once its drag ends
                let opened = *t.opened.get_or_insert_with(|| {
                    log::info!("tool window showed {} ms after the drop", t.asked.elapsed().as_millis());
                    Instant::now()
                });
                if opened.elapsed() < Duration::from_millis(1000) && !tools::is_front(&t.window) {
                    tools::bring_to_front(&t.window);
                }
                t.at = Some(at);
            }
        }

        if self.menu.click_mode && self.menu.visible {
            return;
        }
        self.last_poll = Instant::now();
        let input = raw.input(self.settings.key());
        match self.watcher.tick(input, self.menu.visible, self.menu.got_drag) {
            Some(trigger::Event::Show(pos)) => self.show_menu(pos, false),
            Some(trigger::Event::ToggleRing) => self.next_ring(),
            Some(trigger::Event::HideSoon) => {
                if self.menu.hide_at.is_none() {
                    log::debug!("hide soon: lbtn={} esc={} drag={}", input.lbtn, input.esc, self.menu.got_drag);
                }
                self.menu.hide_at.get_or_insert(Instant::now() + DROP_GRACE);
            }
            Some(trigger::Event::Hide) => {
                log::debug!("hide: no file drag arrived");
                self.hide_menu()
            }
            None => {}
        }
        // the menu's window takes file drops (from startup on, and again if it is remade)
        if let Some(me) = &self.me {
            droptarget::ensure("Utsuri", me);
        }
        if self.menu.visible {
            if self.hover_from_cursor(input.pos) {
                self.push_menu();
            }
            if self.menu.hide_at.is_some_and(|t| Instant::now() >= t) {
                self.hide_menu();
            }
        }
    }
}

fn tray_menu(enabled: bool) -> Vec<rune::tray::TrayItem> {
    use rune::tray::TrayItem;
    vec![
        TrayItem::action("pause", if enabled { "Pause Utsuri" } else { "Resume Utsuri" }),
        TrayItem::action("open", "Open a file…"),
        TrayItem::Separator,
        TrayItem::action("settings", "Settings…"),
        TrayItem::action("help", "How to use"),
        TrayItem::Separator,
        TrayItem::action("quit", "Quit Utsuri"),
    ]
}

fn hub_view(_: &Hub, _: &mut rune::Cx<Hub>) -> rune::El {
    rune::col()
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--cli") {
        return cli::main(&args[2..]);
    }
    // used by Utsuri-Setup.exe and Settings → Apps → Uninstall
    let suffix_after = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).filter(|a| !a.starts_with("--")).cloned().unwrap_or_default();
    if args.iter().any(|a| a == "--register-shell") {
        return shell::install_shell_menu(&suffix_after("--register-shell"));
    }
    if args.iter().any(|a| a == "--unregister-shell") {
        return shell::uninstall_shell_menu(&suffix_after("--unregister-shell"));
    }
    #[cfg(windows)]
    if args.iter().any(|a| a == "--uninstall") {
        let o = install::Opts::from_args(&args[1..]);
        let suffix = o.suffix.clone();
        let r = install::uninstall(&install::UTSURI, &o, || shell::uninstall_shell_menu(&suffix));
        if let Err(e) = &r {
            if !o.silent {
                install::tell("Uninstall Utsuri", &format!("Couldn't remove Utsuri:\n\n{e:#}"), true);
            }
        }
        return r;
    }
    // headless renders, scripts and trees (Rune's tooling): every surface as a window
    if headless::wanted(&args) {
        crash::install(true);
        headless::run();
        crash::self_test();
        return Ok(());
    }
    // the real app keeps Windows' crash box, but also writes where it crashed (temp folder)
    crash::install(false);
    // a second launch hands its files to the running copy and leaves at once
    if ipc::try_handoff(&args) {
        return Ok(());
    }
    init_log();
    log::info!("Utsuri {} starting: {:?}", env!("CARGO_PKG_VERSION"), &args[1..]);

    let mut hub = Hub::new();
    let files: Vec<PathBuf> = args.iter().skip(1).filter(|a| !a.starts_with("--")).map(PathBuf::from).filter(|p| p.exists()).collect();
    if !files.is_empty() {
        hub.pending = files;
        hub.pending_at = Some(Instant::now());
    }
    let first_run = !hub.settings.welcomed && !args.iter().any(|a| a == "--quiet") && std::env::var_os("UTSURI_NO_WELCOME").is_none();
    let theme = hub.theme.clone();

    let mut app = rune::App::new(rune::AppSpec::new("Utsuri", (10.0, 10.0), || unreachable!("state comes from init"), hub_view)).args(Vec::<String>::new()).windowless().keep_running();
    let me = app.handle();

    // the radial menu: never takes focus
    let mut menu = rune::ConfiguredOverlay::from(OverlaySpec { name: "Utsuri", size: (ui::menu::SIZE, ui::menu::SIZE), edge: rune::ScreenEdge::Top, margin: 0.0, align: rune::Align::Start, inset: 0.0, interactive: true, view: ui::menu::view });
    menu.options = OverlayOptions { keys: rune::KeyPolicy::OnDemand, prewarm: true, level: rune::OverlayLevel::AboveOverlays, ..Default::default() };
    let mut ms = MenuS::new(theme.clone());
    ms.app = Some(me.clone());
    let menu_ov = app.overlay(menu, ms);

    let mut toasts = rune::ConfiguredOverlay::from(OverlaySpec { name: "Utsuri notifications", size: (ui::toasts::WIDTH + 2.0 * ui::toasts::PAD, ui::toasts::HEIGHT), edge: rune::ScreenEdge::Bottom, margin: 0.0, align: rune::Align::End, inset: 0.0, interactive: true, view: ui::toasts::view });
    toasts.options = OverlayOptions { prewarm: true, input: rune::OverlayInput::Regions, ..Default::default() };
    let mut ts = ToastS::new(theme);
    ts.app = Some(me.clone());
    let toast_ov = app.overlay(toasts, ts);

    hub.me = Some(me.clone());
    hub.menu_ov = Some(menu_ov);
    hub.toast_ov = Some(toast_ov);

    // the tray (Shell_NotifyIcon on its own thread)
    let me2 = me.clone();
    let tray = rune::tray::Tray::new(rune::tray::TraySpec::new("utsuri", "Utsuri", rune::tray::TrayIcon::rgba(32, 32, icon_rgba(32))).tooltip(hub.tooltip()).menu(tray_menu(true)), move |e| {
        me2.update(move |h, tx| h.tray_event(e, tx));
    });
    match tray {
        Ok(t) => hub.tray = Some(t),
        Err(e) => log::warn!("no tray icon: {e:?}"),
    }

    // a second launch's files arrive here
    let me3 = me.clone();
    ipc::serve(move |m| {
        me3.update(move |h, _| {
            log::info!("pipe: {} file(s), show={}", m.files.len(), m.show);
            if m.files.is_empty() {
                if m.show {
                    let key = h.settings.trigger_label();
                    h.push_toast(Body::Info { title: "Utsuri is already running".into(), msg: format!("Drag any file and press {key}. It lives in the tray.") });
                }
            } else {
                for f in m.files {
                    if !h.pending.contains(&f) {
                        h.pending.push(f);
                    }
                }
                h.pending_at = Some(Instant::now());
            }
        });
    });
    #[cfg(target_os = "linux")]
    {
        let me5 = me.clone();
        linux::dnd::start(move |d| {
            me5.update(move |h, tx| match d {
                linux::dnd::Drag::Entered(files) => h.menu_drag_entered(files),
                linux::dnd::Drag::Dropping => h.menu_drop_pending(),
                linux::dnd::Drag::Dropped(files) if files.is_empty() => {
                    log::debug!("drop: the source sent no files");
                    h.hide_menu()
                }
                linux::dnd::Drag::Dropped(files) => {
                    for f in &files {
                        log::debug!("dropped: {}", f.display());
                    }
                    h.menu_dropped(files, tx)
                }
            });
        });
        if !linux::keys::readable() {
            me.update(|h, _| {
                h.push_toast(Body::Info { title: "Utsuri can't see the keyboard".into(), msg: "Add yourself to the input group (sudo usermod -aG input $USER) and sign in again.".into() });
            });
        }
    }
    log::info!("ready (pipe {})", ipc::pipe_name());
    // finding ffmpeg and Tesseract starts programs (half a second): do it now, off the UI
    // thread, not during the first drag
    std::thread::spawn(|| {
        let (ff, ocr) = (utsuri_engine::media::available(), utsuri_engine::ocr::ready());
        log::info!("tools: ffmpeg {ff}, tesseract {ocr}");
    });

    // the trigger and timers
    let me4 = me.clone();
    std::thread::Builder::new()
        .name("utsuri-tick".into())
        .spawn(move || loop {
            std::thread::sleep(TICK);
            let (raw, at) = (trigger::sample(), Instant::now());
            me4.update(move |h, tx| {
                // a stall queues many ticks: say it once, with the longest wait (the first tick)
                static LOGGED: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
                let late = at.elapsed();
                if late > Duration::from_millis(100) {
                    let mut l = LOGGED.lock().unwrap();
                    if l.is_none_or(|t| t.elapsed() > Duration::from_secs(1)) {
                        *l = Some(Instant::now());
                        log::debug!("ui thread was busy for {} ms", late.as_millis());
                    }
                }
                h.tick(raw, tx)
            });
        })
        .expect("tick thread");

    me.update(|h, _| h.warm_menu());
    if first_run {
        me.update(|h, _| h.welcome());
    }
    let mut hub = Some(hub);
    app.init(move || hub.take().expect("one hub")).run();
    Ok(())
}
