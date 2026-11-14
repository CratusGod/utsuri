//! Utsuri E2E: drives the real app with real mouse drags from File Explorer, the way a
//! person does (drag a file, press Shift, tap Shift to pick a ring, drop on a slice),
//! drives the tool windows, and checks every result file.
//!
//! It never touches the user's own files or settings: fixtures live in
//! target\e2e\<run>\, and the app runs with its own settings, log, pipe and a fake
//! Recycle Bin. It types only into Utsuri's own windows and the Explorer window it
//! opened, after checking which window has focus.
//!
//!   cargo run -p utsuri-e2e -- [--only a,b] [--release] [--keep]

mod cases;

#[cfg(windows)]
mod input;

#[cfg(windows)]
mod app {

use anyhow::{Context as _, Result, anyhow, bail};
use super::input::*;
use crate::cases::*;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// Where menus open: right half of the screen, away from Explorer.
const MENU_AT: (i32, i32) = (1380, 520);

struct App {
    child: Child,
    log: PathBuf,
    exe: PathBuf,
    env: Vec<(String, String)>,
}

impl App {
    fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
    fn log_len(&self) -> usize {
        self.log_text().len()
    }
    fn log_since(&self, from: usize) -> String {
        let t = self.log_text();
        t.get(from..).unwrap_or("").to_string()
    }
}

// ---------------------------------------------------------------- the run

fn wait_for<T>(timeout: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let end = Instant::now() + timeout;
    loop {
        if let Some(v) = f() {
            return Some(v);
        }
        if Instant::now() > end {
            return None;
        }
        sleep(80);
    }
}

fn run_case(app: &App, ex: &mut Explorer, root: &Path, c: &Case, fx: &Path) -> Result<String> {
    // a fresh folder with just this case's inputs
    let dir = root.join(c.name.replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"));
    std::fs::create_dir_all(&dir)?;
    let mut srcs = vec![];
    for f in &c.files {
        let from = fx.join(f);
        let to = dir.join(f);
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
        srcs.push(to);
    }
    let (ring, slice, n) = slice_of(&srcs, &c.key)?;
    ex.navigate(&dir)?;
    let item_names: Vec<String> = c.files.iter().map(|f| f.to_string()).collect();
    let pts = ex.item_points(&item_names)?;
    let mark = app.log_len();
    // several files: the case folder holds only the inputs, so Ctrl+A selects them
    // (clicking them one by one risks a double-click that opens the file)
    if pts.len() > 1 {
        if foreground() != ex.hwnd {
            ex.focus_checked()?;
        }
        // focus the file list (an empty spot), then select everything
        let (x, y, w, h) = window_rect(ex.hwnd);
        click((x + w - 60, y + h - 60));
        sleep(250);
        if foreground() != ex.hwnd {
            bail!("focus left the test Explorer window");
        }
        key_combo(&[VK_CONTROL_], 0x41);
        sleep(400);
    }
    let from = *pts.last().unwrap();
    move_to(from);
    sleep(120);
    left_down();
    glide(from, (from.0 + 30, from.1 + 10), 6);
    // Explorer needs a moment to start the drag before the cursor travels on
    sleep(250);
    if std::env::var_os("E2E_SHOTS").is_some() {
        println!("grab at {from:?}");
        let (x, y, w, h) = window_rect(ex.hwnd);
        shot(x.max(0), y.max(0), w, h, &root.join(format!("{}-grab.png", c.name.replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"))));
    }
    glide((from.0 + 30, from.1 + 10), MENU_AT, 18);
    sleep(150);
    tap_shift();
    // the menu opens under the cursor; nudge so the drag enters it
    let menu = wait_for(Duration::from_secs(3), || find_window("Utsuri").filter(|h| is_visible(*h))).ok_or_else(|| {
        left_up();
        anyhow!("the menu didn't open")
    })?;
    let (mx, my, mw, _) = window_rect(menu);
    let center = (mx + mw / 2, my + mw / 2);
    let scale = mw as f32 / 400.0;
    glide(MENU_AT, (MENU_AT.0 + 4, MENU_AT.1 + 3), 3);
    sleep(250);
    for _ in 0..ring {
        tap_shift();
        sleep(120);
    }
    let target = slice_point(center, n, slice, scale);
    glide((MENU_AT.0 + 4, MENU_AT.1 + 3), target, 14);
    sleep(300);
    left_up();
    sleep(200);
    // tool windows
    if let Some(title) = c.window {
        let w = wait_for(Duration::from_secs(10), || find_window(title)).ok_or_else(|| anyhow!("the {title} window didn't open"))?;
        // it exists before its first frame shows: wait until it is what a click there would hit
        wait_for(Duration::from_secs(8), || {
            let (x, y, ww, wh) = window_rect(w);
            (root_at((x + ww / 2, y + wh / 2)) == w).then_some(())
        })
        .ok_or_else(|| anyhow!("the {title} window never showed"))?;
        sleep(600);
        focus(w);
        sleep(250);
        if foreground_title() != title {
            bail!("refusing to type: focus is on {:?} ({})", foreground_title(), foreground_class());
        }
        match c.drive {
            Drive::Enter => press(VK_RETURN_),
            Drive::DrawEnter => {
                let (x, y, ww, wh) = window_rect(w);
                let (cx, cy) = (x + ww / 2, y + wh / 2 + 20);
                move_to((cx - 120, cy - 60));
                println!("  under the pointer: {}", under_pointer());
                left_down();
                glide((cx - 120, cy - 60), (cx + 120, cy + 60), 12);
                left_up();
                sleep(300);
                if foreground_title() != title {
                    bail!("refusing to type: focus moved to {:?}", foreground_title());
                }
                press(VK_RETURN_);
            }
            Drive::Password(pw, twice) => {
                type_text(pw);
                if twice {
                    press(VK_TAB_);
                    type_text(pw);
                }
                sleep(150);
                press(VK_RETURN_);
            }
            Drive::None => {}
        }
    }
    // the result
    let r = wait_for(Duration::from_secs(120), || parse_done(&app.log_since(mark))).ok_or_else(|| anyhow!("no result within 2 minutes"))?;
    match (r, c.expect_error) {
        (Err(msg), Some(want)) if msg.contains(want) => Ok(format!("error as expected: {msg}")),
        (Err(msg), _) => bail!("failed: {msg}"),
        (Ok(_), Some(want)) => bail!("expected an error ({want}) but it worked"),
        (Ok((title, detail, outs)), None) => {
            if std::env::var_os("E2E_EXPECT_SUBFOLDER").is_some() {
                for p in &outs {
                    if !p.components().any(|c| c.as_os_str() == "Utsuri") {
                        bail!("expected the result in a Utsuri subfolder: {}", p.display());
                    }
                }
            }
            let o = Outcome { outs, title: title.clone(), detail, dir, srcs };
            let msg = (c.check)(&o)?;
            Ok(format!("{title} · {msg}"))
        }
    }
}

/// Undo from the notification: the last result goes to the (fake) Recycle Bin.
fn undo_case(app: &App, ex: &mut Explorer, root: &Path, fx: &Path, bin: &Path) -> Result<String> {
    let c = case("Undo", &["logo.png"], "convert:WEBP", |_| Ok(String::new()));
    let mark = app.log_len();
    run_case(app, ex, root, &c, fx)?;
    let log = app.log_since(mark);
    let (_, _, outs) = parse_done(&log).ok_or_else(|| anyhow!("no result"))?.map_err(|e| anyhow!(e))?;
    let out = outs.first().cloned().ok_or_else(|| anyhow!("no output"))?;
    let undo = wait_for(Duration::from_secs(5), || {
        app.log_since(mark).lines().filter(|l| l.contains("toast-buttons ")).last().and_then(|l| {
            l.split_once("toast-buttons ")?.1.split(';').filter(|b| b.contains(":Undo:")).last().map(|b| {
                let v: Vec<i32> = b.split(':').skip(2).filter_map(|n| n.parse().ok()).collect();
                (v[0] + v[2] / 2, v[1] + v[3] / 2)
            })
        })
    })
    .ok_or_else(|| anyhow!("no Undo button logged"))?;
    move_to(undo);
    sleep(150);
    println!("  under the pointer: {}", under_pointer());
    click(undo);
    wait_for(Duration::from_secs(5), || (!out.exists()).then_some(())).ok_or_else(|| anyhow!("{} still there after Undo", out.display()))?;
    let binned = bin.join(out.file_name().unwrap());
    if !binned.exists() {
        bail!("not in the recycle bin: {}", binned.display());
    }
    Ok(format!("{} → Recycle Bin", out.file_name().unwrap().to_string_lossy()))
}

/// Right-click → Utsuri… style: a second launch hands the file over; click a slice.
fn handoff_case(app: &App, root: &Path, fx: &Path) -> Result<String> {
    let dir = root.join("Handoff");
    std::fs::create_dir_all(&dir)?;
    let src = dir.join("photo.jpg");
    std::fs::copy(fx.join("photo.jpg"), &src)?;
    let (ring, slice, n) = slice_of(std::slice::from_ref(&src), "convert:PNG")?;
    move_to(MENU_AT);
    let mark = app.log_len();
    let mut cmd = Command::new(&app.exe);
    cmd.arg(&src);
    for (k, v) in &app.env {
        cmd.env(k, v);
    }
    let status = cmd.status()?;
    if !status.success() {
        bail!("the second launch failed");
    }
    let menu = wait_for(Duration::from_secs(4), || find_window("Utsuri").filter(|h| is_visible(*h))).ok_or_else(|| anyhow!("no click menu"))?;
    sleep(400);
    let (mx, my, mw, _) = window_rect(menu);
    let center = (mx + mw / 2, my + mw / 2);
    for _ in 0..ring {
        click(center);
        sleep(200);
    }
    let target = slice_point(center, n, slice, mw as f32 / 400.0);
    glide(center, target, 8);
    sleep(150);
    click(target);
    let r = wait_for(Duration::from_secs(30), || parse_done(&app.log_since(mark))).ok_or_else(|| anyhow!("no result"))?;
    let (title, _, outs) = r.map_err(|e| anyhow!(e))?;
    let p = outs.first().ok_or_else(|| anyhow!("no output"))?;
    if utsuri_engine::kinds::ext(p) != "png" {
        bail!("got {}", p.display());
    }
    Ok(format!("{title} via the pipe"))
}

pub fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let only: Option<Vec<String>> = args.iter().position(|a| a == "--only").and_then(|i| args.get(i + 1)).map(|s| s.split(',').map(|x| x.trim().to_lowercase()).collect());
    let release = args.iter().any(|a| a == "--release");
    let keep = args.iter().any(|a| a == "--keep");
    let ws = std::env::current_dir()?;
    let profile = std::env::var("CRATUS_PROFILE").unwrap_or_else(|_| if release { "release" } else { "debug" }.to_string());
    let exe = ws.join("target").join(profile).join("utsuri.exe");
    if !exe.exists() {
        bail!("build it first: {}", exe.display());
    }
    if other_utsuri_running(&exe) {
        bail!("another Utsuri is running (the Python one?): quit it first so only one menu opens");
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let base = ws.join("target").join("e2e").join(format!("run-{stamp}"));
    let fx = base.join("fixtures");
    let state = base.join("state");
    let root = base.join("cases");
    std::fs::create_dir_all(&state)?;
    std::fs::create_dir_all(&root)?;
    println!("fixtures…");
    // the fixtures use the same engine options as the app
    utsuri_engine::fixtures::write_all(&fx)?;
    // --ctrl-folder: Ctrl as the trigger, results in a "Utsuri" subfolder, another theme
    let alt = args.iter().any(|a| a == "--ctrl-folder");
    if alt {
        // SAFETY: set before any other thread starts
        unsafe { std::env::set_var("E2E_EXPECT_SUBFOLDER", "1") };
        TRIGGER.store(VK_CONTROL_, std::sync::atomic::Ordering::Relaxed);
        std::fs::write(state.join("settings.json"), r#"{"welcomed": true, "trigger": "ctrl", "output": "folder", "theme": "Tokyo Night Storm"}"#)?;
    } else {
        std::fs::write(state.join("settings.json"), r#"{"welcomed": true}"#)?;
    }
    let env: Vec<(String, String)> = vec![
        ("UTSURI_SETTINGS".into(), state.join("settings.json").display().to_string()),
        ("UTSURI_STATE_DIR".into(), state.display().to_string()),
        ("UTSURI_FAKE_TRASH".into(), state.join("bin").display().to_string()),
        ("UTSURI_PIPE".into(), "Utsuri-e2e".into()),
        ("UTSURI_LOG".into(), state.join("log.txt").display().to_string()),
        ("UTSURI_NO_LAUNCH".into(), "1".into()),
        ("UTSURI_NO_WELCOME".into(), "1".into()),
        ("UTSURI_E2E".into(), "1".into()),
    ];
    let mut cmd = Command::new(&exe);
    for (k, v) in &env {
        cmd.env(k, v);
    }
    cmd.stderr(std::process::Stdio::null()).stdout(std::process::Stdio::null());
    let child = cmd.spawn()?;
    let app = App { child, log: state.join("log.txt"), exe: exe.clone(), env: env.clone() };
    wait_for(Duration::from_secs(20), || app.log_text().contains("ready (pipe").then_some(())).ok_or_else(|| anyhow!("Utsuri didn't start: {}", app.log_text()))?;
    let mut ex = Explorer::open(&root)?;
    let mut results = vec![];
    let mut all = cases();
    if let Some(o) = &only {
        all.retain(|c| o.iter().any(|x| c.name.to_lowercase().contains(x)));
    }
    let total = all.len();
    for (i, c) in all.iter().enumerate() {
        let t = Instant::now();
        print!("[{}/{total}] {} … ", i + 1, c.name);
        let _ = std::io::stdout().flush();
        let r = run_case(&app, &mut ex, &root, c, &fx);
        close_tool_windows();
        let (ok, msg) = match r {
            Ok(m) => (true, m),
            Err(e) => (false, format!("{e:#}")),
        };
        println!("{} {msg}", if ok { "ok" } else { "FAIL" });
        results.push(Result1 { name: c.name.clone(), ok, msg, secs: t.elapsed().as_secs_f32() });
        sleep(400);
    }
    if only.as_ref().is_none_or(|o| o.iter().any(|x| "undo".contains(x.as_str()))) {
        let t = Instant::now();
        print!("[+] Undo … ");
        let r = undo_case(&app, &mut ex, &root, &fx, &state.join("bin"));
        let (ok, msg) = match r {
            Ok(m) => (true, m),
            Err(e) => (false, format!("{e:#}")),
        };
        println!("{} {msg}", if ok { "ok" } else { "FAIL" });
        results.push(Result1 { name: "Undo from the notification".into(), ok, msg, secs: t.elapsed().as_secs_f32() });
    }
    if only.as_ref().is_none_or(|o| o.iter().any(|x| "handoff".contains(x.as_str()))) {
        let t = Instant::now();
        print!("[+] Handoff … ");
        let r = handoff_case(&app, &root, &fx);
        let (ok, msg) = match r {
            Ok(m) => (true, m),
            Err(e) => (false, format!("{e:#}")),
        };
        println!("{} {msg}", if ok { "ok" } else { "FAIL" });
        results.push(Result1 { name: "Second launch hands files over (right-click / Send To)".into(), ok, msg, secs: t.elapsed().as_secs_f32() });
    }
    ex.close();
    let mut app = app;
    let _ = app.child.kill();
    // report
    let passed = results.iter().filter(|r| r.ok).count();
    let mut md = format!("# Utsuri E2E — {passed}/{} passed\n\nRun `{}`\n\n| Case | Result | Detail | Time |\n|---|---|---|---|\n", results.len(), base.display());
    for r in &results {
        md.push_str(&format!("| {} | {} | {} | {:.1}s |\n", r.name, if r.ok { "✅" } else { "❌" }, r.msg.replace('|', "/"), r.secs));
    }
    std::fs::write(base.join("report.md"), &md)?;
    std::fs::write(ws.join("target").join("e2e").join("latest.md"), &md)?;
    println!("\n{passed}/{} passed · report: {}", results.len(), base.join("report.md").display());
    if !keep && passed == results.len() {
        let _ = std::fs::remove_dir_all(&root);
    }
    if passed != results.len() {
        std::process::exit(1);
    }
    Ok(())
}

fn other_utsuri_running(ours: &Path) -> bool {
    let out = Command::new("tasklist").args(["/FO", "CSV", "/NH"]).output().map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase()).unwrap_or_default();
    let _ = ours;
    out.lines().any(|l| l.starts_with("\"utsuri.exe\""))
}

// ---------------------------------------------------------------- File Explorer

struct Explorer {
    hwnd: windows::Win32::Foundation::HWND,
    ui: uiautomation::UIAutomation,
}

impl Explorer {
    fn open(dir: &Path) -> Result<Explorer> {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        Command::new("explorer.exe").arg(dir).spawn()?;
        let hwnd = wait_for(Duration::from_secs(15), || find_explorer(&name)).ok_or_else(|| anyhow!("Explorer didn't open {name}"))?;
        place(hwnd, 0, 0, 1000, 1000);
        sleep(800);
        let ui = uiautomation::UIAutomation::new().map_err(|e| anyhow!("{e}"))?;
        let mut ex = Explorer { hwnd, ui };
        ex.focus_checked()?;
        // details view: one row per file, names fully visible
        key_combo(&[VK_CONTROL_, VK_SHIFT_], 0x36);
        sleep(400);
        Ok(ex)
    }

    fn focus_checked(&mut self) -> Result<()> {
        focus(self.hwnd);
        sleep(200);
        if foreground() != self.hwnd {
            sleep(400);
            focus(self.hwnd);
            sleep(300);
            if foreground() != self.hwnd {
                bail!("couldn't bring the test Explorer window to the front (focus is on {:?})", foreground_title());
            }
        }
        Ok(())
    }

    /// Go to `dir` through the address bar (only after checking focus is ours).
    fn navigate(&mut self, dir: &Path) -> Result<()> {
        self.focus_checked()?;
        key_combo(&[VK_MENU_], 0x44); // Alt+D: address bar
        sleep(250);
        if foreground() != self.hwnd {
            bail!("focus left the test Explorer window");
        }
        type_text(&dir.display().to_string());
        press(VK_RETURN_);
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        wait_for(Duration::from_secs(8), || title_is(&window_title(self.hwnd), &name).then_some(())).ok_or_else(|| anyhow!("Explorer didn't go to {name} (title {:?})", window_title(self.hwnd)))?;
        sleep(700);
        // click an empty spot of the list so nothing is selected
        let (x, y, w, h) = window_rect(self.hwnd);
        click((x + w - 60, y + h - 60));
        sleep(200);
        Ok(())
    }

    /// Screen points to grab each file by.
    fn item_points(&self, names: &[String]) -> Result<Vec<(i32, i32)>> {
        use uiautomation::types::ControlType;
        let win = self.ui.element_from_handle(self.hwnd.into()).map_err(|e| anyhow!("{e}"))?;
        let mut pts = vec![];
        for n in names {
            let stem = Path::new(n).file_stem().unwrap().to_string_lossy().into_owned();
            let found = wait_for(Duration::from_secs(6), || {
                let items = self.ui.create_matcher().from_ref(&win).control_type(ControlType::ListItem).depth(12).timeout(800).find_all().ok()?;
                items.into_iter().find(|it| it.get_name().map(|nm| nm == *n || nm == stem).unwrap_or(false))
            })
            .ok_or_else(|| anyhow!("{n} isn't listed in Explorer"))?;
            let r = found.get_bounding_rectangle().map_err(|e| anyhow!("{e}"))?;
            let (l, t, w, h) = (r.get_left(), r.get_top(), r.get_width(), r.get_height());
            // the file icon: in details view, blank row space starts a selection box instead of a drag
            let x = if w > 300 { l + 22 } else { l + w / 2 };
            pts.push((x, t + h / 2));
        }
        Ok(pts)
    }

    fn close(&self) {
        close_window(self.hwnd);
    }
}

}

#[cfg(windows)]
fn main() {
    app::main()
}

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    linux::main()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn main() {
    eprintln!("This tool is Windows-only; see tests/linux for the Linux harness.");
    std::process::exit(2);
}
