//! Global key and button state from evdev (/dev/input/event*). Wayland gives apps no global
//! input state, so the trigger reads the devices directly; that needs the `input` group.
//! One reader thread per device; a scanner picks up devices plugged in later (and ydotool's
//! virtual device, which the end-to-end tests drive).

use evdev::{Device, EventSummary, KeyCode};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, Once};
use std::time::Duration;

/// Bits of what's held right now.
const LBTN: u32 = 1;
const SHIFT_L: u32 = 1 << 1;
const SHIFT_R: u32 = 1 << 2;
const CTRL_L: u32 = 1 << 3;
const CTRL_R: u32 = 1 << 4;
const ALT_L: u32 = 1 << 5;
const ALT_R: u32 = 1 << 6;
const ESC: u32 = 1 << 7;

/// Each device's held bits, so one device's release can't clear another's press.
static DEVICES: Mutex<Vec<(PathBuf, u32)>> = Mutex::new(Vec::new());
static HELD: AtomicU32 = AtomicU32::new(0);
/// At least one keyboard or mouse could be opened.
static READABLE: AtomicBool = AtomicBool::new(false);
static START: Once = Once::new();

const RESCAN: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Default)]
pub struct Held {
    pub lbtn: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub esc: bool,
}

/// What's held now (starts the readers on first use).
pub fn held() -> Held {
    START.call_once(start);
    let b = HELD.load(Ordering::Relaxed);
    Held { lbtn: b & LBTN != 0, shift: b & (SHIFT_L | SHIFT_R) != 0, ctrl: b & (CTRL_L | CTRL_R) != 0, alt: b & (ALT_L | ALT_R) != 0, esc: b & ESC != 0 }
}

/// False when no input device could be read (not in the `input` group yet).
pub fn readable() -> bool {
    START.call_once(start);
    READABLE.load(Ordering::Relaxed)
}

fn bit(code: KeyCode) -> u32 {
    match code {
        KeyCode::BTN_LEFT => LBTN,
        KeyCode::KEY_LEFTSHIFT => SHIFT_L,
        KeyCode::KEY_RIGHTSHIFT => SHIFT_R,
        KeyCode::KEY_LEFTCTRL => CTRL_L,
        KeyCode::KEY_RIGHTCTRL => CTRL_R,
        KeyCode::KEY_LEFTALT => ALT_L,
        KeyCode::KEY_RIGHTALT => ALT_R,
        KeyCode::KEY_ESC => ESC,
        _ => 0,
    }
}

/// A keyboard or a mouse: it reports Shift or the left button.
fn wanted(d: &Device) -> bool {
    d.supported_keys().is_some_and(|k| k.contains(KeyCode::KEY_LEFTSHIFT) || k.contains(KeyCode::BTN_LEFT))
}

fn start() {
    let first = scan(&mut HashSet::new());
    if first == 0 {
        log::warn!("no keyboard or mouse is readable: the trigger needs the `input` group (sudo usermod -aG input $USER, then sign in again)");
    }
    let _ = std::thread::Builder::new().name("utsuri-evdev-scan".into()).spawn(|| {
        let mut seen = HashSet::new();
        // `scan` above opened the first set; remember them so they aren't opened twice
        seen.extend(DEVICES.lock().unwrap().iter().map(|(p, _)| p.clone()));
        loop {
            std::thread::sleep(RESCAN);
            scan(&mut seen);
        }
    });
}

/// Opens devices not seen yet; returns how many were opened.
fn scan(seen: &mut HashSet<PathBuf>) -> usize {
    let mut opened = 0;
    for (path, dev) in evdev::enumerate() {
        if seen.contains(&path) || !wanted(&dev) {
            continue;
        }
        seen.insert(path.clone());
        DEVICES.lock().unwrap().push((path.clone(), 0));
        READABLE.store(true, Ordering::Relaxed);
        opened += 1;
        log::debug!("trigger reads {} ({})", path.display(), dev.name().unwrap_or("?"));
        let _ = std::thread::Builder::new().name("utsuri-evdev".into()).spawn(move || read(path, dev));
    }
    opened
}

fn set(path: &PathBuf, f: impl FnOnce(u32) -> u32) {
    let mut devs = DEVICES.lock().unwrap();
    if let Some(d) = devs.iter_mut().find(|(p, _)| p == path) {
        d.1 = f(d.1);
    }
    HELD.store(devs.iter().fold(0, |a, (_, b)| a | b), Ordering::Relaxed);
}

fn read(path: PathBuf, mut dev: Device) {
    loop {
        let events = match dev.fetch_events() {
            Ok(e) => e.collect::<Vec<_>>(),
            Err(e) => {
                // unplugged: forget it; the scanner opens it again if it comes back
                log::debug!("{} closed: {e}", path.display());
                let mut devs = DEVICES.lock().unwrap();
                devs.retain(|(p, _)| *p != path);
                HELD.store(devs.iter().fold(0, |a, (_, b)| a | b), Ordering::Relaxed);
                return;
            }
        };
        for ev in events {
            if let EventSummary::Key(_, code, value) = ev.destructure() {
                let b = bit(code);
                if b == 0 {
                    continue;
                }
                // 1 press, 2 autorepeat, 0 release
                set(&path, |held| if value == 0 { held & !b } else { held | b });
            }
        }
    }
}
