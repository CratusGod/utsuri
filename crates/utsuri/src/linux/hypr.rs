//! The pointer position and the monitors, from the compositor. Hyprland answers over its IPC
//! socket; anything else falls back to the primary screen's centre (the trigger then still
//! opens, just not under the cursor).

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_millis(200);

fn socket() -> Option<PathBuf> {
    let sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let run = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
    Some(run.join("hypr").join(sig).join(".socket.sock"))
}

/// One request to Hyprland's IPC socket; the whole reply.
pub fn request(cmd: &str) -> Option<String> {
    let mut s = UnixStream::connect(socket()?).ok()?;
    s.set_read_timeout(Some(TIMEOUT)).ok()?;
    s.set_write_timeout(Some(TIMEOUT)).ok()?;
    s.write_all(cmd.as_bytes()).ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    Some(out)
}

/// The pointer in global layout coordinates (logical px).
pub fn cursor() -> Option<(i32, i32)> {
    parse_cursor(&request("cursorpos")?)
}

fn parse_cursor(s: &str) -> Option<(i32, i32)> {
    let (x, y) = s.trim().split_once(',')?;
    Some((x.trim().parse::<f64>().ok()?.round() as i32, y.trim().parse::<f64>().ok()?.round() as i32))
}

/// A monitor: its rect in layout px, the part windows may use, and its scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// Reserved edges (bars): left, top, right, bottom.
    pub reserved: (i32, i32, i32, i32),
    pub scale: f32,
}

impl Monitor {
    pub fn contains(&self, p: (i32, i32)) -> bool {
        p.0 >= self.x && p.1 >= self.y && p.0 < self.x + self.w && p.1 < self.y + self.h
    }

    pub fn work_area(&self) -> (i32, i32, i32, i32) {
        let (l, t, r, b) = self.reserved;
        (self.x + l, self.y + t, self.w - l - r, self.h - t - b)
    }
}

pub fn monitors() -> Vec<Monitor> {
    request("j/monitors").and_then(|s| parse_monitors(&s)).unwrap_or_default()
}

fn parse_monitors(s: &str) -> Option<Vec<Monitor>> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    let out = v
        .as_array()?
        .iter()
        .filter_map(|m| {
            let scale = m["scale"].as_f64().unwrap_or(1.0).max(0.1);
            // width/height are mode pixels; the layout uses them divided by the scale
            let w = (m["width"].as_f64()? / scale).round() as i32;
            let h = (m["height"].as_f64()? / scale).round() as i32;
            let (w, h) = if m["transform"].as_i64().unwrap_or(0) % 2 == 1 { (h, w) } else { (w, h) };
            let r = m["reserved"].as_array();
            let res = |i: usize| r.and_then(|r| r.get(i)).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            Some(Monitor { x: m["x"].as_i64()? as i32, y: m["y"].as_i64()? as i32, w, h, reserved: (res(0), res(1), res(2), res(3)), scale: scale as f32 })
        })
        .collect();
    Some(out)
}

/// The monitor at a point, or the first one.
pub fn monitor_at(p: (i32, i32)) -> Option<Monitor> {
    let all = monitors();
    all.iter().copied().find(|m| m.contains(p)).or_else(|| all.first().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cursor_and_monitors() {
        assert_eq!(parse_cursor("960, 540\n"), Some((960, 540)));
        assert_eq!(parse_cursor("bad"), None);
        let j = r#"[{"x":0,"y":0,"width":2560,"height":1440,"scale":1.25,"transform":0,"reserved":[0,40,0,0]}]"#;
        let m = parse_monitors(j).unwrap()[0];
        assert_eq!((m.w, m.h), (2048, 1152));
        assert_eq!(m.work_area(), (0, 40, 2048, 1112));
        assert!(m.contains((100, 100)) && !m.contains((3000, 0)));
    }
}
