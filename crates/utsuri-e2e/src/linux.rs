//! The Linux driver (Hyprland + ydotool): real drags from a GTK test window (a real
//! Wayland drag source, like a file manager) onto the real Utsuri, then the same checks as
//! on Windows. Every pointer move, button and key is a real input event from ydotool's
//! uinput device. Keys are typed only after checking a test window has the focus.
//!
//! The trigger is Ctrl: the desktop's own Utsuri plugin answers Shift+drag, and two menus
//! must never open at once.

use crate::cases::*;
use anyhow::{Context as _, Result, anyhow, bail};
use std::io::{Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Where menus open: right half of the screen, away from the drag source.
const MENU_AT: (i32, i32) = (1300, 520);
/// The drag source window's rect (layout px).
const SOURCE: (i32, i32, i32, i32) = (60, 140, 360, 360);
const SOURCE_TITLE: &str = "utsuri-e2e-source";
const WORKSPACE: &str = "9";
const KEY_ENTER: u16 = 28;
const KEY_TAB: u16 = 15;
const KEY_CTRL: u16 = 29;

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

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

// ---------------------------------------------------------------- Hyprland

fn hypr(cmd: &str) -> Result<String> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").context("not in a Hyprland session")?;
    let run = std::env::var("XDG_RUNTIME_DIR").context("no XDG_RUNTIME_DIR")?;
    let mut s = UnixStream::connect(format!("{run}/hypr/{sig}/.socket.sock"))?;
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    s.write_all(cmd.as_bytes())?;
    let mut out = String::new();
    s.read_to_string(&mut out)?;
    Ok(out)
}

fn dispatch(lua: &str) -> Result<()> {
    let r = hypr(&format!("dispatch {lua}"))?;
    if r.trim() != "ok" {
        bail!("hyprland refused {lua}: {}", r.trim());
    }
    Ok(())
}

fn clients() -> Vec<serde_json::Value> {
    hypr("j/clients").ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn client(title: &str) -> Option<serde_json::Value> {
    clients().into_iter().find(|c| c["title"] == title)
}

fn active_title() -> String {
    hypr("j/activewindow").ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).and_then(|v| v["title"].as_str().map(String::from)).unwrap_or_default()
}

fn rect(c: &serde_json::Value) -> (i32, i32, i32, i32) {
    let n = |v: &serde_json::Value| v.as_i64().unwrap_or(0) as i32;
    (n(&c["at"][0]), n(&c["at"][1]), n(&c["size"][0]), n(&c["size"][1]))
}

fn focus(addr: &str) -> Result<()> {
    dispatch(&format!("hl.dsp.focus({{ window = \"address:{addr}\" }})"))
}

fn place(addr: &str, (x, y, w, h): (i32, i32, i32, i32)) -> Result<()> {
    let sel = format!("window = \"address:{addr}\"");
    dispatch(&format!("hl.dsp.window.float({{ action = \"enable\", {sel} }})"))?;
    dispatch(&format!("hl.dsp.window.resize({{ x = {w}, y = {h}, exact = true, {sel} }})"))?;
    dispatch(&format!("hl.dsp.window.move({{ x = {x}, y = {y}, exact = true, {sel} }})"))?;
    sleep(150);
    Ok(())
}

fn close_tool_windows() {
    for c in clients() {
        if c["title"].as_str().is_some_and(|t| t.ends_with("— Utsuri")) {
            if let Some(a) = c["address"].as_str() {
                let _ = dispatch(&format!("hl.dsp.window.close({{ window = \"address:{a}\" }})"));
            }
        }
    }
    sleep(300);
}

// ---------------------------------------------------------------- input (ydotool)

fn ydotool(args: &[&str]) -> Result<()> {
    let s = Command::new("ydotool").args(args).stdout(Stdio::null()).stderr(Stdio::null()).status()?;
    if !s.success() {
        bail!("ydotool {args:?} failed");
    }
    Ok(())
}

fn cursor() -> (i32, i32) {
    hypr("cursorpos")
        .ok()
        .and_then(|s| {
            let (x, y) = s.trim().split_once(',')?;
            Some((x.trim().parse::<f64>().ok()?.round() as i32, y.trim().parse::<f64>().ok()?.round() as i32))
        })
        .unwrap_or((0, 0))
}

fn jump(p: (i32, i32)) {
    let _ = dispatch(&format!("hl.dsp.cursor.move({{ x = {}, y = {} }})", p.0, p.1));
    sleep(40);
}

/// Glide with relative moves (real motion events), correcting from the compositor's own
/// pointer each step.
fn glide(to: (i32, i32), steps: i32) {
    for i in (1..=steps).rev() {
        let c = cursor();
        let (dx, dy) = ((to.0 - c.0) / i, (to.1 - c.1) / i);
        if dx == 0 && dy == 0 {
            continue;
        }
        let _ = ydotool(&["mousemove", "-x", &dx.to_string(), "-y", &dy.to_string()]);
        sleep(12);
    }
    for _ in 0..4 {
        let c = cursor();
        if c == to {
            return;
        }
        let _ = ydotool(&["mousemove", "-x", &(to.0 - c.0).to_string(), "-y", &(to.1 - c.1).to_string()]);
        sleep(12);
    }
}

fn left(down: bool) {
    let _ = ydotool(&["click", if down { "0x40" } else { "0x80" }]);
}

fn click(p: (i32, i32)) {
    glide(p, 6);
    sleep(80);
    left(true);
    sleep(60);
    left(false);
}

fn key(code: u16, down: bool) {
    let _ = ydotool(&["key", &format!("{code}:{}", u8::from(down))]);
}

fn tap(code: u16) {
    key(code, true);
    sleep(40);
    key(code, false);
}

fn release_all() {
    for k in [KEY_CTRL, 42, 56, 1] {
        key(k, false);
    }
    left(false);
}

/// Type into the focused window, only when it is the test window `title`.
fn type_into(title: &str, text: &str) -> Result<()> {
    if active_title() != title {
        bail!("refusing to type: focus is on {:?}", active_title());
    }
    ydotool(&["type", "--key-delay", "12", text])
}

fn press_into(title: &str, code: u16) -> Result<()> {
    if active_title() != title {
        bail!("refusing to press keys: focus is on {:?}", active_title());
    }
    tap(code);
    Ok(())
}

// ---------------------------------------------------------------- the app

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
        self.log_text().get(from..).unwrap_or("").to_string()
    }
    /// The centre of the last menu shown since `mark` ("menu at (x, y)").
    fn menu_center(&self, mark: usize) -> Option<(i32, i32)> {
        let log = self.log_since(mark);
        let line = log.lines().rev().find(|l| l.contains("utsuri: menu at ("))?;
        let inner = line.split_once("menu at (")?.1.split_once(')')?.0;
        let (x, y) = inner.split_once(',')?;
        Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
    }
}

/// The trigger reads /dev/input, which needs the `input` group. A session started before
/// the user joined it doesn't have the group yet: `newgrp` gives this one run of Utsuri it.
fn spawn_utsuri(exe: &Path, env: &[(String, String)], args: &[&str]) -> Result<Child> {
    let readable = Command::new("id").arg("-nG").output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().any(|g| g == "input"));
    let mut cmd = if readable {
        let mut c = Command::new(exe);
        c.args(args);
        c
    } else {
        let mut c = Command::new("newgrp");
        c.arg("input").stdin(Stdio::piped());
        c
    };
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = cmd.spawn()?;
    if !readable {
        let quoted: Vec<String> = std::iter::once(exe.display().to_string()).chain(args.iter().map(|a| a.to_string())).map(|a| format!("'{}'", a.replace('\'', "'\\''"))).collect();
        let mut stdin = child.stdin.take().context("newgrp stdin")?;
        writeln!(stdin, "exec {}", quoted.join(" "))?;
    }
    Ok(child)
}

struct Source {
    child: Child,
    control: PathBuf,
    addr: String,
}

impl Source {
    fn open(ws: &Path, control: PathBuf) -> Result<Source> {
        std::fs::write(&control, "")?;
        let script = ws.join("tests/linux/drag_source.py");
        let child = Command::new("python3").arg(&script).arg(&control).arg(SOURCE_TITLE).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        let c = wait_for(Duration::from_secs(15), || client(SOURCE_TITLE)).ok_or_else(|| anyhow!("the drag source window didn't open"))?;
        let addr = c["address"].as_str().unwrap_or_default().to_string();
        place(&addr, SOURCE)?;
        Ok(Source { child, control, addr })
    }

    fn offer(&self, files: &[PathBuf]) -> Result<()> {
        let body: String = files.iter().map(|p| format!("{}\n", p.display())).collect();
        std::fs::write(&self.control, body)?;
        Ok(())
    }

    fn grab_point(&self) -> (i32, i32) {
        let (x, y, w, h) = client(SOURCE_TITLE).map(|c| rect(&c)).unwrap_or(SOURCE);
        (x + w / 2, y + h / 2)
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------- the cases

fn prepare(root: &Path, c: &Case, fx: &Path) -> Result<(PathBuf, Vec<PathBuf>)> {
    let dir = root.join(c.name.replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"));
    std::fs::create_dir_all(&dir)?;
    let mut srcs = vec![];
    for f in &c.files {
        let (from, to) = (fx.join(f), dir.join(f));
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
        srcs.push(to);
    }
    Ok((dir, srcs))
}

fn run_case(app: &App, src: &Source, root: &Path, c: &Case, fx: &Path) -> Result<String> {
    let (dir, srcs) = prepare(root, c, fx)?;
    let (ring, slice, n) = slice_of(&srcs, &c.key)?;
    src.offer(&srcs)?;
    let mark = app.log_len();
    let from = src.grab_point();
    jump(from);
    sleep(120);
    left(true);
    glide((from.0 + 30, from.1 + 10), 6);
    // GTK starts the drag after a little movement: let it before travelling on
    sleep(250);
    glide(MENU_AT, 18);
    sleep(150);
    tap(KEY_CTRL);
    let center = wait_for(Duration::from_secs(3), || app.menu_center(mark)).ok_or_else(|| {
        left(false);
        anyhow!("the menu didn't open")
    })?;
    // nudge so the drag enters the menu's drop surface
    glide((center.0 + 4, center.1 + 3), 3);
    sleep(250);
    for _ in 0..ring {
        tap(KEY_CTRL);
        sleep(120);
    }
    let target = slice_point(center, n, slice, 1.0);
    glide(target, 14);
    sleep(300);
    left(false);
    sleep(200);
    if let Some(title) = c.window {
        drive_tool(title, c.drive)?;
    }
    let r = wait_for(Duration::from_secs(120), || parse_done(&app.log_since(mark))).ok_or_else(|| anyhow!("no result within 2 minutes"))?;
    match (r, c.expect_error) {
        (Err(msg), Some(want)) if msg.contains(want) => Ok(format!("error as expected: {msg}")),
        (Err(msg), _) => bail!("failed: {msg}"),
        (Ok(_), Some(want)) => bail!("expected an error ({want}) but it worked"),
        (Ok((title, detail, outs)), None) => {
            let o = Outcome { outs, title: title.clone(), detail, dir, srcs };
            Ok(format!("{title} · {}", (c.check)(&o)?))
        }
    }
}

fn drive_tool(title: &str, drive: Drive) -> Result<()> {
    let w = wait_for(Duration::from_secs(10), || client(title)).ok_or_else(|| anyhow!("the {title} window didn't open"))?;
    let addr = w["address"].as_str().unwrap_or_default().to_string();
    sleep(600);
    focus(&addr)?;
    wait_for(Duration::from_secs(3), || (active_title() == title).then_some(())).ok_or_else(|| anyhow!("refusing to type: focus is on {:?}", active_title()))?;
    match drive {
        Drive::Enter => press_into(title, KEY_ENTER)?,
        Drive::DrawEnter => {
            let (x, y, ww, wh) = client(title).map(|c| rect(&c)).ok_or_else(|| anyhow!("{title} closed"))?;
            let (cx, cy) = (x + ww / 2, y + wh / 2 + 20);
            jump((cx - 120, cy - 60));
            left(true);
            glide((cx + 120, cy + 60), 12);
            left(false);
            sleep(300);
            press_into(title, KEY_ENTER)?;
        }
        Drive::Password(pw, twice) => {
            type_into(title, pw)?;
            if twice {
                press_into(title, KEY_TAB)?;
                type_into(title, pw)?;
            }
            sleep(150);
            press_into(title, KEY_ENTER)?;
        }
        Drive::None => {}
    }
    Ok(())
}

/// The toast's Undo puts the last result in the (fake) trash.
fn undo_case(app: &App, src: &Source, root: &Path, fx: &Path, bin: &Path) -> Result<String> {
    let c = case("Undo", &["logo.png"], "convert:WEBP", |_| Ok(String::new()));
    let mark = app.log_len();
    run_case(app, src, root, &c, fx)?;
    let (_, _, outs) = parse_done(&app.log_since(mark)).ok_or_else(|| anyhow!("no result"))?.map_err(|e| anyhow!(e))?;
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
    click(undo);
    wait_for(Duration::from_secs(5), || (!out.exists()).then_some(())).ok_or_else(|| anyhow!("{} still there after Undo", out.display()))?;
    let binned = bin.join(out.file_name().unwrap());
    if !binned.exists() {
        bail!("not in the trash: {}", binned.display());
    }
    Ok(format!("{} → trash", out.file_name().unwrap().to_string_lossy()))
}

/// The file manager's "Utsuri…" entry: a second launch hands the file over; click a slice.
fn handoff_case(app: &App, root: &Path, fx: &Path) -> Result<String> {
    let dir = root.join("Handoff");
    std::fs::create_dir_all(&dir)?;
    let src = dir.join("photo.jpg");
    std::fs::copy(fx.join("photo.jpg"), &src)?;
    let (ring, slice, n) = slice_of(std::slice::from_ref(&src), "convert:PNG")?;
    jump(MENU_AT);
    let mark = app.log_len();
    let mut cmd = Command::new(&app.exe);
    cmd.arg(&src).stdout(Stdio::null()).stderr(Stdio::null());
    for (k, v) in &app.env {
        cmd.env(k, v);
    }
    if !cmd.status()?.success() {
        bail!("the second launch failed");
    }
    let center = wait_for(Duration::from_secs(4), || app.menu_center(mark)).ok_or_else(|| anyhow!("no click menu"))?;
    sleep(400);
    for _ in 0..ring {
        click(center);
        sleep(200);
    }
    let target = slice_point(center, n, slice, 1.0);
    glide(target, 8);
    sleep(150);
    click(target);
    let (title, _, outs) = wait_for(Duration::from_secs(30), || parse_done(&app.log_since(mark))).ok_or_else(|| anyhow!("no result"))?.map_err(|e| anyhow!(e))?;
    let p = outs.first().ok_or_else(|| anyhow!("no output"))?;
    if utsuri_engine::kinds::ext(p) != "png" {
        bail!("got {}", p.display());
    }
    Ok(format!("{title} via the socket"))
}

fn record(results: &mut Vec<Result1>, name: &str, t: Instant, r: Result<String>) {
    let (ok, msg) = match r {
        Ok(m) => (true, m),
        Err(e) => (false, format!("{e:#}")),
    };
    println!("{} {msg}", if ok { "ok" } else { "FAIL" });
    results.push(Result1 { name: name.into(), ok, msg, secs: t.elapsed().as_secs_f32() });
}

pub fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let only: Option<Vec<String>> = args.iter().position(|a| a == "--only").and_then(|i| args.get(i + 1)).map(|s| s.split(',').map(|x| x.trim().to_lowercase()).collect());
    let keep = args.iter().any(|a| a == "--keep");
    let ws = std::env::current_dir()?;
    let profile = std::env::var("CRATUS_PROFILE").unwrap_or_else(|_| "debug".into());
    let exe = ws.join("target").join(profile).join("utsuri");
    if !exe.exists() {
        bail!("build it first: {}", exe.display());
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let base = ws.join("target").join("e2e").join(format!("linux-run-{stamp}"));
    let (fx, state, root) = (base.join("fixtures"), base.join("state"), base.join("cases"));
    std::fs::create_dir_all(&state)?;
    std::fs::create_dir_all(&root)?;
    println!("fixtures…");
    utsuri_engine::fixtures::write_all(&fx)?;
    std::fs::write(state.join("settings.json"), r#"{"welcomed": true, "trigger": "ctrl"}"#)?;
    let env: Vec<(String, String)> = vec![
        ("UTSURI_SETTINGS".into(), state.join("settings.json").display().to_string()),
        ("UTSURI_STATE_DIR".into(), state.display().to_string()),
        ("UTSURI_FAKE_TRASH".into(), state.join("bin").display().to_string()),
        ("UTSURI_PIPE".into(), "utsuri-e2e".into()),
        ("UTSURI_LOG".into(), state.join("log.txt").display().to_string()),
        ("UTSURI_NO_LAUNCH".into(), "1".into()),
        ("UTSURI_NO_WELCOME".into(), "1".into()),
        ("UTSURI_E2E".into(), "1".into()),
    ];
    let home = hypr("j/activeworkspace").ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).and_then(|v| v["name"].as_str().map(String::from)).unwrap_or_else(|| "1".into());
    dispatch(&format!("hl.dsp.focus({{ workspace = \"{WORKSPACE}\" }})"))?;
    let child = spawn_utsuri(&exe, &env, &[])?;
    let app = App { child, log: state.join("log.txt"), exe: exe.clone(), env };
    let started = wait_for(Duration::from_secs(20), || app.log_text().contains("ready (pipe").then_some(()));
    let mut results = vec![];
    let outcome = (|| -> Result<()> {
        started.ok_or_else(|| anyhow!("Utsuri didn't start: {}", app.log_text()))?;
        // the menu warms up at start; drops before then are lost
        sleep(1500);
        let src = Source::open(&ws, base.join("control.txt"))?;
        focus(&src.addr)?;
        let mut all = cases();
        if let Some(o) = &only {
            all.retain(|c| o.iter().any(|x| c.name.to_lowercase().contains(x)));
        }
        let total = all.len();
        for (i, c) in all.iter().enumerate() {
            let t = Instant::now();
            print!("[{}/{total}] {} … ", i + 1, c.name);
            let _ = std::io::stdout().flush();
            let r = run_case(&app, &src, &root, c, &fx);
            release_all();
            close_tool_windows();
            record(&mut results, &c.name, t, r);
            sleep(400);
        }
        if only.as_ref().is_none_or(|o| o.iter().any(|x| "undo".contains(x.as_str()))) {
            let t = Instant::now();
            print!("[+] Undo … ");
            let r = undo_case(&app, &src, &root, &fx, &state.join("bin"));
            record(&mut results, "Undo from the notification", t, r);
        }
        if only.as_ref().is_none_or(|o| o.iter().any(|x| "handoff".contains(x.as_str()))) {
            let t = Instant::now();
            print!("[+] Handoff … ");
            let r = handoff_case(&app, &root, &fx);
            record(&mut results, "Second launch hands files over (file manager entry)", t, r);
        }
        Ok(())
    })();
    release_all();
    close_tool_windows();
    let mut app = app;
    let _ = app.child.kill();
    let _ = Command::new("pkill").args(["-f", &format!("{} ", exe.display())]).status();
    let _ = dispatch(&format!("hl.dsp.focus({{ workspace = \"{home}\" }})"));
    outcome?;
    let passed = results.iter().filter(|r| r.ok).count();
    let mut md = format!("# Utsuri E2E (Linux) — {passed}/{} passed\n\nRun `{}`\n\n| Case | Result | Detail | Time |\n|---|---|---|---|\n", results.len(), base.display());
    for r in &results {
        md.push_str(&format!("| {} | {} | {} | {:.1}s |\n", r.name, if r.ok { "✅" } else { "❌" }, r.msg.replace('|', "/"), r.secs));
    }
    std::fs::write(base.join("report.md"), &md)?;
    std::fs::write(ws.join("target").join("e2e").join("linux-latest.md"), &md)?;
    println!("\n{passed}/{} passed · report: {}", results.len(), base.join("report.md").display());
    if !keep && passed == results.len() {
        let _ = std::fs::remove_dir_all(&root);
    }
    if passed != results.len() {
        std::process::exit(1);
    }
    Ok(())
}
