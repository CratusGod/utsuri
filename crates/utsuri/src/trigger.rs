//! Watches for "left button held + mouse moved + trigger key pressed" anywhere on screen,
//! which is what a Shift+drag of a file looks like. Polling the global input state needs no
//! admin rights and can't stall the system the way a broken hook can.
//! Ported from utsuri/trigger.py.


/// Pixels the mouse must travel with the button held.
pub const DRAG_THRESHOLD: i32 = 10;
/// Ticks (about 0.75 s at 15 ms) to wait for a file drag to arrive before hiding.
pub const NO_DRAG_TIMEOUT: u32 = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Key {
    #[default]
    Shift,
    Ctrl,
    Alt,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub lbtn: bool,
    pub key: bool,
    pub esc: bool,
    /// Cursor in physical screen pixels.
    pub pos: (i32, i32),
}

/// Read the real global input state.
/// Every key the trigger may use, read at one moment. The tick thread reads this itself and
/// hands each reading to the UI thread, so a moment the UI thread is busy can delay the menu
/// but never lose a quick Shift tap.
#[derive(Clone, Copy, Debug, Default)]
pub struct Raw {
    pub lbtn: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub esc: bool,
    pub pos: (i32, i32),
}

#[cfg(windows)]
pub fn sample() -> Raw {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_ESCAPE, VK_LBUTTON, VK_MENU, VK_SHIFT};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let down = |vk: u16| unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 };
    let mut p = POINT::default();
    let _ = unsafe { GetCursorPos(&mut p) };
    Raw { lbtn: down(VK_LBUTTON.0), shift: down(VK_SHIFT.0), ctrl: down(VK_CONTROL.0), alt: down(VK_MENU.0), esc: down(VK_ESCAPE.0), pos: (p.x, p.y) }
}

/// Linux: keys and the button from evdev; the pointer from the compositor, asked only
/// while the button is down or the menu may be up (an IPC round trip every 15 ms otherwise).
#[cfg(not(windows))]
pub fn sample() -> Raw {
    use std::sync::Mutex;
    static LAST: Mutex<(i32, i32)> = Mutex::new((0, 0));
    static WAS_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let h = crate::linux::keys::held();
    let was = WAS_DOWN.swap(h.lbtn, std::sync::atomic::Ordering::Relaxed);
    let pos = if h.lbtn || was {
        let p = crate::shell::cursor();
        *LAST.lock().unwrap() = p;
        p
    } else {
        *LAST.lock().unwrap()
    };
    Raw { lbtn: h.lbtn, shift: h.shift, ctrl: h.ctrl, alt: h.alt, esc: h.esc, pos }
}

impl Raw {
    pub fn input(&self, key: Key) -> Input {
        let k = match key {
            Key::Shift => self.shift,
            Key::Ctrl => self.ctrl,
            Key::Alt => self.alt,
        };
        Input { lbtn: self.lbtn, key: k, esc: self.esc, pos: self.pos }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Open the menu centred here.
    Show((i32, i32)),
    /// The key was tapped again during a drag: flip Convert / Tools.
    ToggleRing,
    /// The button went up or Esc: hide once a drop has had a moment to land.
    HideSoon,
    /// The cursor moved over us but no file drag arrived (a window move, a text selection).
    Hide,
}

#[derive(Default)]
pub struct Watcher {
    pub enabled: bool,
    down_at: Option<(i32, i32)>,
    key_was: bool,
    fired: bool,
    idle_ticks: u32,
    shown_at: (i32, i32),
}

fn manhattan(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs() + (a.1 - b.1).abs()
}

impl Watcher {
    pub fn new() -> Self {
        Self { enabled: true, ..Default::default() }
    }

    pub fn tick(&mut self, i: Input, menu_visible: bool, got_drag: bool) -> Option<Event> {
        let mut ev = None;
        if menu_visible {
            if !i.lbtn || i.esc {
                ev = Some(Event::HideSoon);
            } else if i.key && !self.key_was && got_drag {
                ev = Some(Event::ToggleRing);
            } else if !got_drag && manhattan(i.pos, self.shown_at) > 6 {
                self.idle_ticks += 1;
                if self.idle_ticks > NO_DRAG_TIMEOUT {
                    ev = Some(Event::Hide);
                }
            }
        } else if i.lbtn {
            let at = *self.down_at.get_or_insert_with(|| {
                self.fired = false;
                i.pos
            });
            let moved = manhattan(i.pos, at) >= DRAG_THRESHOLD;
            if self.enabled && moved && i.key && !self.fired {
                self.fired = true;
                self.idle_ticks = 0;
                self.shown_at = i.pos;
                ev = Some(Event::Show(i.pos));
            }
        } else {
            self.down_at = None;
            self.fired = false;
        }
        self.key_was = i.key;
        ev
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inp(lbtn: bool, key: bool, x: i32) -> Input {
        Input { lbtn, key, esc: false, pos: (x, 0) }
    }

    #[test]
    fn shift_drag_opens_once() {
        let mut w = Watcher::new();
        assert_eq!(w.tick(inp(true, false, 0), false, false), None); // press
        assert_eq!(w.tick(inp(true, true, 5), false, false), None); // not far enough
        assert_eq!(w.tick(inp(true, true, 12), false, false), Some(Event::Show((12, 0))));
        assert_eq!(w.tick(inp(true, true, 20), false, false), None); // already fired
    }

    #[test]
    fn plain_drag_or_shift_click_does_nothing() {
        let mut w = Watcher::new();
        for x in 0..40 {
            assert_eq!(w.tick(inp(true, false, x), false, false), None);
        }
        let mut w = Watcher::new();
        assert_eq!(w.tick(inp(true, true, 0), false, false), None);
        assert_eq!(w.tick(inp(false, true, 0), false, false), None);
    }

    #[test]
    fn release_hides_and_tap_toggles() {
        let mut w = Watcher::new();
        w.tick(inp(true, false, 0), false, false);
        w.tick(inp(true, true, 12), false, false);
        assert_eq!(w.tick(inp(true, false, 12), true, true), None); // key up
        assert_eq!(w.tick(inp(true, true, 12), true, true), Some(Event::ToggleRing));
        assert_eq!(w.tick(inp(true, true, 12), true, true), None); // held, not a new tap
        assert_eq!(w.tick(inp(false, false, 12), true, true), Some(Event::HideSoon));
    }

    #[test]
    fn no_file_drag_gets_out_of_the_way() {
        let mut w = Watcher::new();
        w.tick(inp(true, false, 0), false, false);
        w.tick(inp(true, true, 12), false, false);
        let mut hid = false;
        for _ in 0..=NO_DRAG_TIMEOUT {
            hid |= w.tick(inp(true, true, 100), true, false) == Some(Event::Hide);
        }
        assert!(hid);
    }

    #[test]
    fn disabled_never_opens() {
        let mut w = Watcher::new();
        w.enabled = false;
        w.tick(inp(true, false, 0), false, false);
        assert_eq!(w.tick(inp(true, true, 50), false, false), None);
    }
}
