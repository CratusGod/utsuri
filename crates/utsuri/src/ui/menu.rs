//! The radial drop menu: geometry (from radial.py) and its Rune view: solid petals with
//! rims, labels and the hub (the see-through glass was removed on 2026-11-14).

use super::*;
use rune::shape::Shape as Vector;
use rune::{AppHandle, Cx, canvas};
use std::path::PathBuf;
use utsuri_engine::actions::Slice;

/// Window size in logical pixels (room for the hover pop).
pub const SIZE: f32 = 400.0;
pub const R_OUT: f32 = 158.0;
pub const R_IN: f32 = 60.0;
pub const R_HUB: f32 = 50.0;
pub const GAP_DEG: f32 = 3.2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Slice(usize),
    Hub,
    Outside,
}

/// Angles of petal `i` of `n` in degrees (0 = 3 o'clock, counter-clockwise). The first petal
/// is centred at 12 o'clock and they run clockwise.
pub fn angles(n: usize, i: usize) -> (f32, f32) {
    let span = 360.0 / n as f32;
    let start = 90.0 + span / 2.0 - (i as f32 + 1.0) * span;
    (start + GAP_DEG / 2.0, start + span - GAP_DEG / 2.0)
}

/// Point at `deg` and radius `r` around `c`, y down.
pub fn at(c: (f32, f32), r: f32, deg: f32) -> [f32; 2] {
    let a = deg.to_radians();
    [c.0 + r * a.cos(), c.1 - r * a.sin()]
}

/// What's under the offset (dx, dy) from the menu centre, in logical pixels.
pub fn hit(dx: f32, dy: f32, n: usize) -> Hit {
    let r = (dx * dx + dy * dy).sqrt();
    if r <= R_HUB {
        return Hit::Hub;
    }
    if n == 0 || !(R_IN - 6.0..=R_OUT + 14.0).contains(&r) {
        return Hit::Outside;
    }
    let ang = (-dy).atan2(dx).to_degrees();
    (0..n)
        .find(|&i| {
            let (a0, a1) = angles(n, i);
            // count the gaps too: a drop between two petals goes to the nearer one
            let a0 = a0 - GAP_DEG / 2.0;
            (ang - a0).rem_euclid(360.0) <= (a1 + GAP_DEG / 2.0) - a0
        })
        .map_or(Hit::Outside, Hit::Slice)
}

/// An annular sector as a polygon: outer arc, then the inner arc back.
fn sector(c: (f32, f32), r0: f32, r1: f32, a0: f32, a1: f32) -> Vec<[f32; 2]> {
    let steps = (((a1 - a0).abs() / 3.0).ceil() as usize).max(2);
    let mut pts: Vec<[f32; 2]> = (0..=steps).map(|k| at(c, r1, a0 + (a1 - a0) * k as f32 / steps as f32)).collect();
    pts.extend((0..=steps).rev().map(|k| at(c, r0, a0 + (a1 - a0) * k as f32 / steps as f32)));
    pts
}

/// What the menu overlay shows. The app (main.rs) owns the logic and pushes this in.
pub struct MenuS {
    pub visible: bool,
    /// Opened from the tray / right-click / Send To: clicks choose, no drag.
    pub click_mode: bool,
    pub paths: Vec<PathBuf>,
    pub rings: Vec<(String, Vec<Slice>)>,
    pub ring: usize,
    pub hover: Option<Hit>,
    pub got_drag: bool,
    pub theme: Theme,
    /// The key that switches rings ("Shift"), or "" in click mode.
    pub key: String,
    pub app: Option<AppHandle<crate::Hub>>,
    /// drawn at 1% opacity once at startup, so the first real show has nothing left to prepare
    pub ghost: bool,
}

impl MenuS {
    pub fn new(theme: Theme) -> MenuS {
        MenuS { visible: false, click_mode: false, paths: vec![], rings: vec![], ring: 0, hover: None, got_drag: false, theme, key: String::new(), app: None, ghost: false }
    }

    pub fn items(&self) -> &[Slice] {
        self.rings.get(self.ring).map_or(&[], |r| r.1.as_slice())
    }

    /// What's being dragged: a name line and a size line.
    fn hub_text(&self) -> (String, String) {
        match self.paths.as_slice() {
            [] => ("Drop a file".into(), String::new()),
            [p] => (p.file_name().unwrap_or_default().to_string_lossy().into_owned(), utsuri_engine::out::human_size(utsuri_engine::out::file_size(p))),
            ps => (format!("{} files", ps.len()), utsuri_engine::out::human_size(ps.iter().map(|p| utsuri_engine::out::file_size(p)).sum())),
        }
    }
}

fn elide(s: &str, n: usize) -> String {
    if s.chars().count() > n { format!("{}…", s.chars().take(n - 1).collect::<String>()) } else { s.to_string() }
}

pub fn view(s: &MenuS, _cx: &mut Cx<MenuS>) -> El {
    let th = &s.theme;
    let c = (SIZE / 2.0, SIZE / 2.0);
    let (lab, dimc) = (col_of(th.label()), col_of(th.text_dim));
    let solid = col_of(th.tint(Rgba(22, 18, 34, 255)));
    let rim = col_of(th.accent_a).alpha(0.55);
    let items = s.items();
    let n = items.len();
    let hover = s.hover.unwrap_or(Hit::Outside);

    let mut shapes = Vector::new();
    let mut labels: Vec<El> = vec![];
    for (i, it) in items.iter().enumerate() {
        let (a0, a1) = angles(n, i);
        let hovered = hover == Hit::Slice(i);
        let r1 = if hovered { R_OUT + 8.0 } else { R_OUT };
        let poly = sector(c, R_IN, r1, a0, a1);
        let fill = if hovered { col_of(th.accent_b.mix(th.tint(Rgba(22, 18, 34, 230)), 0.35)) } else { solid };
        shapes = shapes.fill(poly.clone(), fill);
        let mut closed = poly;
        closed.push(closed[0]);
        shapes = shapes.stroke(closed, if hovered { 2.0 } else { 1.2 }, if hovered { col_of(th.accent_a) } else { rim });

        let mid_r = (R_IN + R_OUT) / 2.0 + if hovered { 4.0 } else { 0.0 };
        let m = at(c, mid_r, (a0 + a1) / 2.0);
        let big = if n > 8 { 13.5 } else { 15.0 };
        let mut col_el = col().items_center().gap(2.0);
        if !it.glyph.is_empty() {
            col_el = col_el.child(text(&it.glyph).font(font(19.0, 500.0)).color(lab));
        }
        col_el = col_el.child(text(&it.label).font(font(big, 600.0)).color(lab));
        if hovered && !it.hint.is_empty() {
            col_el = col_el.child(text(&it.hint).font(font(11.5, 450.0)).color(dimc));
        }
        labels.push(el().key(format!("slice/{i}")).absolute(m[0] - 60.0, m[1] - 30.0).size(120.0, 60.0).center().child(col_el));
    }
    if n == 0 {
        let ring = sector(c, R_IN, R_OUT, 90.0, 449.9);
        shapes = shapes.fill(ring, solid);
    }

    // hub
    let hub_fill = if hover == Hit::Hub { col_of(th.tint(Rgba(40, 32, 60, 255))) } else { col_of(th.tint(Rgba(26, 21, 40, 255))) };
    let (ring_name, sub) = match s.rings.get(s.ring) {
        Some((name, _)) => (name.clone(), s.hub_text()),
        None if s.got_drag => ("Unsupported".into(), s.hub_text()),
        None => ("Utsuri".into(), s.hub_text()),
    };
    let mut hub_col = col().items_center().gap(1.0).child(text(ring_name).key("hub/ring").font(font(14.5, 650.0)).color(lab)).child(text(elide(&sub.0, 14)).key("hub/file").font(font(11.0, 450.0)).color(dimc));
    if !sub.1.is_empty() {
        hub_col = hub_col.child(text(&sub.1).key("hub/size").font(font(10.5, 450.0)).color(dimc));
    }
    if s.rings.len() > 1 {
        let mut dots = row().gap(6.0).pt(3.0);
        for k in 0..s.rings.len() {
            let on = k == s.ring;
            dots = dots.child(el().size(if on { 7.0 } else { 5.0 }, if on { 7.0 } else { 5.0 }).radius(4.0).fill(if on { col_of(th.accent_a) } else { dimc }));
        }
        hub_col = hub_col.child(dots);
        if !s.key.is_empty() && hover == Hit::Hub {
            hub_col = hub_col.child(text(format!("tap {}", s.key)).font(font(10.5, 450.0)).color(dimc));
        }
    }
    let hub = el()
        .key("hub")
        .absolute(c.0 - R_HUB, c.1 - R_HUB)
        .size(R_HUB * 2.0, R_HUB * 2.0)
        .radius(R_HUB)
        .fill(hub_fill)
        .border(1.2, rim)
        .center()
        .child(hub_col);

    let mut root = el()
        .key("menu")
        .size(SIZE, SIZE)
        .child(canvas(shapes).absolute(0.0, 0.0).size(SIZE, SIZE))
        .child(hub);
    for l in labels {
        root = root.child(l);
    }
    let root = if s.ghost { root.dim(0.01) } else { root };
    root
        // a file dropped anywhere on the menu: the app picks the slice under the pointer
        .on_drop_files(|s: &mut MenuS, files, _tx| {
            for f in files {
                log::debug!("dropped: {}", f.display());
            }
            if let Some(app) = s.app.clone() {
                let files = files.to_vec();
                app.update(move |h, tx| h.menu_dropped(files, tx));
            }
        })
        .on_drop_hover(|s: &mut MenuS, over, _tx| {
            if over {
                log::debug!("drag over the menu: {}", s.paths.first().map(|p| p.display().to_string()).unwrap_or_default());
                if let Some(app) = s.app.clone() {
                    app.update(|h, _| h.menu_got_drag());
                }
            }
        })
        // click mode: hover follows the pointer, a click chooses
        .on_pointer_move(|s: &mut MenuS, p| {
            if s.click_mode {
                s.hover = Some(hit(p.x - SIZE / 2.0, p.y - SIZE / 2.0, s.items().len()));
            }
        })
        .on_press(|s: &mut MenuS| {
            if !s.click_mode {
                return;
            }
            let hover = s.hover;
            if let Some(app) = s.app.clone() {
                app.update(move |h, tx| h.menu_clicked(hover, tx));
            }
        })
        .on_key(|s: &mut MenuS, k, _tx| {
            if matches!(k.key, rune::Key::Escape) {
                if let Some(app) = s.app.clone() {
                    app.update(|h, _| h.hide_menu());
                }
                return true;
            }
            false
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_petal_is_at_twelve_and_order_is_clockwise() {
        let (a0, a1) = angles(4, 0);
        assert!(((a0 + a1) / 2.0 - 90.0).abs() < 0.01);
        let (b0, b1) = angles(4, 1);
        assert!(((b0 + b1) / 2.0 - 0.0).abs() < 0.01, "second petal at 3 o'clock");
    }

    #[test]
    fn hub_and_outside() {
        assert_eq!(hit(0.0, 0.0, 6), Hit::Hub);
        assert_eq!(hit(190.0, 0.0, 6), Hit::Outside);
        assert_eq!(hit(0.0, -100.0, 6), Hit::Slice(0));
    }

    #[test]
    fn every_angle_maps_to_exactly_one_petal() {
        for n in 1..=10 {
            for deg in 0..360 {
                let a = (deg as f32).to_radians();
                let (dx, dy) = (110.0 * a.cos(), -110.0 * a.sin());
                assert!(matches!(hit(dx, dy, n), Hit::Slice(_)), "n={n} deg={deg}");
            }
        }
    }
}
