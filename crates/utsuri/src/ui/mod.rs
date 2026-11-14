//! Utsuri's look on Rune, after the Python app's stylesheet (glass.py `_qss`): translucent
//! white pill buttons with a lit top edge, a gradient primary button, gradient sliders with a
//! white knob, dark see-through fields, all over liquid glass.

pub mod frame;
pub mod menu;
pub mod toasts;

use rune::tok::TextStyle;
pub use rune::{Color, El, Tx, col, el, row, text};
pub use shared::theme::{Rgba, Theme};

/// A theme colour (sRGB bytes) as Rune's linear colour.
pub fn col_of(c: Rgba) -> Color {
    let d = |v: u8| rune::color::srgb_decode(v as f32 / 255.0);
    Color { r: d(c.0), g: d(c.1), b: d(c.2), a: c.3 as f32 / 255.0 }
}

pub fn white(a: f32) -> Color {
    Color::WHITE.alpha(a)
}

pub fn black(a: f32) -> Color {
    Color::BLACK.alpha(a)
}

/// Body text (10 pt Segoe-sized in the original: 14 px here).
pub fn font(size: f32, weight: f32) -> TextStyle {
    TextStyle::new(size, weight)
}

pub const BODY: TextStyle = TextStyle::new(14.0, 420.0);
pub const SMALL: TextStyle = TextStyle::new(12.5, 420.0);
pub const STRONG: TextStyle = TextStyle::new(14.0, 600.0);
pub const TITLE: TextStyle = TextStyle::new(15.0, 600.0);

pub fn label(t: impl Into<String>, th: &Theme) -> El {
    text(t).font(BODY).color(col_of(th.label()))
}

pub fn dim(t: impl Into<String>, th: &Theme) -> El {
    text(t).font(SMALL).color(col_of(th.text_dim))
}

pub const GOOD: Rgba = Rgba(141, 255, 184, 255);
pub const WARN: Rgba = Rgba(255, 211, 107, 255);
pub const BAD: Rgba = Rgba(255, 120, 120, 255);

/// QLabel#section: small caps, letter-spaced, dim.
pub fn section(t: &str, th: &Theme) -> El {
    text(t.to_uppercase()).font(TextStyle { tracking: 1.0, ..TextStyle::new(11.5, 650.0) }).color(col_of(th.text_dim))
}

/// The lit top edge every glass control has (QSS border-top lighter than the bottom).
fn lit(e: El, strong: bool) -> El {
    e.linear_gradient(std::f32::consts::PI, [(0.0, white(if strong { 0.10 } else { 0.06 })), (0.5, white(0.0)), (1.0, black(0.04))])
}

/// A glass pill button (QPushButton). Chain `.on_press(..)`.
pub fn pill(key: &str, t: &str, th: &Theme) -> El {
    lit(row().key(key).h(30.0).px(15.0).radius(15.0).center().fill(white(0.13)).border(1.0, white(0.30)), false)
        .hover(white(0.09))
        .press(black(0.05))
        .cursor(rune::Cursor::Pointer)
        .role("button")
        .label(t)
        .focusable(true)
        .child(text(t).font(BODY).color(col_of(th.label())))
}

/// A pill that can be greyed out.
pub fn pill_on(key: &str, t: &str, enabled: bool, th: &Theme) -> El {
    let p = pill(key, t, th);
    if enabled { p } else { p.dim(0.4).cursor(rune::Cursor::NotAllowed) }
}

/// The gradient primary button (QPushButton#primary).
pub fn primary(key: &str, t: &str, enabled: bool, th: &Theme) -> El {
    let b = row()
        .key(key)
        .h(32.0)
        .px(20.0)
        .radius(16.0)
        .center()
        .linear_gradient(2.0, [(0.0, col_of(th.accent_a)), (0.55, col_of(th.accent_b)), (1.0, col_of(th.accent_c))])
        .border(1.0, white(0.55))
        .shadow(4.0, 14.0, col_of(th.accent_b).alpha(0.35))
        .hover(white(0.10))
        .press(black(0.10))
        .cursor(rune::Cursor::Pointer)
        .role("button")
        .label(t)
        .focusable(true)
        .child(text(t).font(STRONG).color(Color::WHITE));
    if enabled { b } else { b.dim(0.45).cursor(rune::Cursor::NotAllowed) }
}

/// A toggle pill: checked ones get the theme's check gradient (QPushButton:checked).
pub fn toggle(key: &str, on: bool, t: &str, th: &Theme) -> El {
    let b = row().key(key).h(30.0).px(14.0).radius(15.0).center().cursor(rune::Cursor::Pointer).role("radio").label(t).checked(on).focusable(true);
    let b = if on {
        b.linear_gradient(2.0, [(0.0, col_of(th.check_a)), (1.0, col_of(th.check_b))]).border(1.0, white(0.55))
    } else {
        lit(b.fill(white(0.10)).border(1.0, white(0.24)), false).hover(white(0.09))
    };
    b.child(text(t).font(if on { STRONG } else { BODY }).color(if on { Color::WHITE } else { col_of(th.label()) }))
}

/// A row of toggle pills; `set` gets the picked index.
pub fn toggles<S: 'static>(key: &str, labels: &[&str], cur: usize, th: &Theme, set: impl Fn(&mut S, usize) + Clone + 'static) -> El {
    let mut r = row().key(key).gap(6.0).flex_wrap();
    for (i, l) in labels.iter().enumerate() {
        let set = set.clone();
        r = r.child(toggle(&format!("{key}/{i}"), cur == i, l, th).on_press(move |s: &mut S| set(s, i)));
    }
    r
}

/// QCheckBox: a rounded box with the accent gradient when checked, and its label.
pub fn checkbox(key: &str, on: bool, t: &str, th: &Theme) -> El {
    let boxed = el().size(17.0, 17.0).radius(5.0).center().border(1.0, white(0.5));
    let boxed = if on {
        boxed.linear_gradient(2.0, [(0.0, col_of(th.accent_a)), (1.0, col_of(th.accent_b))]).child(text("✓").font(font(12.0, 700.0)).color(Color::WHITE))
    } else {
        boxed.fill(white(0.10))
    };
    row().key(key).gap(9.0).items_center().cursor(rune::Cursor::Pointer).role("checkbox").label(t).checked(on).focusable(true).child(boxed).child(label(t, th))
}

/// QSlider: a gradient groove and a white knob. `set` gets the new value.
pub fn slider<S: 'static>(key: &str, v: f32, min: f32, max: f32, width: f32, th: &Theme, set: impl Fn(&mut S, f32) + 'static) -> El {
    let t = ((v - min) / (max - min)).clamp(0.0, 1.0);
    let groove = el().absolute(0.0, 9.0).size(width, 4.0).radius(2.0).fill(white(0.22));
    let fill = el().absolute(0.0, 9.0).size((width * t).max(4.0), 4.0).radius(2.0).linear_gradient(std::f32::consts::FRAC_PI_2, [(0.0, col_of(th.accent_a)), (0.5, col_of(th.accent_b)), (1.0, col_of(th.accent_c))]);
    let knob = el().key(&format!("{key}/knob")).absolute(width * t - 7.0, 4.0).size(14.0, 14.0).radius(7.0).fill(Color::WHITE).border(1.0, black(0.15)).shadow(1.0, 4.0, black(0.35));
    el().key(key)
        .size(width, 22.0)
        .role("slider")
        .cursor(rune::Cursor::Pointer)
        .child(groove)
        .child(fill)
        .child(knob)
        .on_drag(move |s: &mut S, d, _tx| {
            let k = (d.x / d.w.max(1.0)).clamp(0.0, 1.0);
            set(s, min + k * (max - min));
        })
}

/// A dark see-through field box (QLineEdit) around any content.
pub fn field_box(e: El) -> El {
    e.radius(10.0).fill(black(0.16)).border(1.0, white(0.22))
}

/// A kit for the fields and pickers rune-kit draws, from the theme.
pub fn kit(th: &Theme) -> rune_kit::Kit {
    rune_kit::Kit::from_palette(rune_kit::PaletteSeed { bg: col_of(th.surface_variant), fg: col_of(th.text), accent: col_of(th.accent_b) })
}

/// A drop-down that opens Windows' own menu (dark, keyboard and all).
pub fn picker<S: 'static>(key: &'static str, options: &[&'static str], cur: usize, width: f32, th: &Theme, set: impl Fn(&mut S, usize) + Clone + 'static) -> El {
    let opts: Vec<(String, &'static str)> = options.iter().enumerate().map(|(i, l)| (i.to_string(), *l)).collect();
    let current = options.get(cur).copied().unwrap_or_default();
    field_box(row().key(key).w(width).h(30.0).px(12.0).items_center().cursor(rune::Cursor::Pointer).role("button").label(current))
        .hover(white(0.06))
        .child(text(current).font(BODY).color(col_of(th.label())))
        .child(rune::spacer())
        .child(text("▾").font(font(12.0, 500.0)).color(col_of(th.text_dim)))
        .on_press_tx(move |_s: &mut S, tx: &mut Tx| {
            let set = set.clone();
            let menu = rune::PopupMenu::picker(opts.clone(), &cur.to_string());
            tx.popup_menu(menu, key, move |s: &mut S, picked, _| {
                if let Some(i) = picked.and_then(|p| p.parse::<usize>().ok()) {
                    set(s, i);
                }
            });
        })
}

/// QFrame#card.
pub fn card(e: El) -> El {
    e.radius(14.0).fill(white(0.07)).border(1.0, white(0.14)).px(16.0).py(12.0)
}

/// An image box: a dark rounded well with the picture fitted inside.
pub fn picture(key: &str, px: Option<&rune::Pixels>) -> El {
    let well = el().key(key).radius(10.0).fill(black(0.32)).border(1.0, white(0.10)).center();
    match px {
        Some(p) => well.child(el().w_full().h_full().radius(10.0).pixels_fit(p, rune::SurfaceFit::Fit)),
        None => well.child(text("…").font(BODY).color(white(0.5))),
    }
}

/// Pixels from an image (straight-alpha sRGB).
pub fn pixels_of(im: &image::DynamicImage) -> rune::Pixels {
    let rgba = im.to_rgba8();
    let px = rune::Pixels::new();
    px.set(rgba.width(), rgba.height(), rgba.into_raw());
    px
}

/// An image fitted into a canvas: canvas point = origin + image pixel × s.
#[derive(Clone, Copy, Debug)]
pub struct Fit {
    pub x: f32,
    pub y: f32,
    pub s: f32,
    pub iw: f32,
    pub ih: f32,
}

impl Fit {
    /// Fit an image of `img` pixels into a canvas of `area` points (centred, a 6 pt margin).
    pub fn new(area: [f32; 2], img: (f32, f32)) -> Fit {
        let (aw, ah) = ((area[0] - 12.0).max(1.0), (area[1] - 12.0).max(1.0));
        let s = (aw / img.0.max(1.0)).min(ah / img.1.max(1.0));
        Fit { x: (area[0] - img.0 * s) / 2.0, y: (area[1] - img.1 * s) / 2.0, s, iw: img.0, ih: img.1 }
    }
    pub fn to_image(&self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.x) / self.s, (y - self.y) / self.s)
    }
    pub fn to_view(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.s, self.y + y * self.s)
    }
    /// The image's rect in the canvas.
    pub fn rect(&self) -> [f32; 4] {
        [self.x, self.y, self.iw * self.s, self.ih * self.s]
    }
}

/// A canvas that fills the free space: a dark well with the image fitted in it, and overlays
/// (absolutely placed, in canvas points) on top. Chain `.on_drag(..)` for interaction.
pub fn canvas_area(key: &str, px: Option<&rune::Pixels>, fit: Option<Fit>, overlays: Vec<El>) -> El {
    let mut c = el().key(key).w_full().grow(1.0).min_h(160.0).radius(10.0).fill(black(0.32)).border(1.0, white(0.10)).clip_rounded(10.0);
    if let (Some(p), Some(f)) = (px, fit) {
        let r = f.rect();
        c = c.child(el().absolute(r[0], r[1]).size(r[2], r[3]).pixels(p));
    }
    for o in overlays {
        c = c.child(o);
    }
    c
}

/// An absolutely placed box (canvas points).
pub fn boxed(x: f32, y: f32, w: f32, h: f32) -> El {
    el().absolute(x, y).size(w.max(0.0), h.max(0.0))
}
