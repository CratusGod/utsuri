//! Edit: light, colour, sharpness, warmth, size; Auto, rotate, flip, B&W, hold to compare.

use super::subtitle;
use crate::ui::frame::{Tool, ToolS, footer};
use crate::ui::*;
use image::DynamicImage;
use rune::{Cx, Tx};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use utsuri_engine::imaging::{Adjust, apply_adjust, auto_enhance};

pub struct Edit {
    paths: Vec<PathBuf>,
    pv: Arc<DynamicImage>,
    orig: rune::Pixels,
    shown: Option<rune::Pixels>,
    a: Adjust,
    work: rune::Latest<rune::Pixels>,
    full: (u32, u32),
    compare: bool,
}

impl Edit {
    pub fn new(paths: Vec<PathBuf>) -> anyhow::Result<(Self, String)> {
        let full = utsuri_engine::imgio::open_image(&paths[0])?;
        let fd = (full.width(), full.height());
        let pv = if fd.0.max(fd.1) > 1000 { full.resize(1000, 1000, image::imageops::FilterType::Triangle) } else { full };
        let orig = pixels_of(&pv);
        let sub = subtitle(&paths);
        Ok((Edit { paths, pv: Arc::new(pv), orig, shown: None, a: Adjust::default(), work: rune::Latest::default().debounce(Duration::from_millis(30)), full: fd, compare: false }, sub))
    }

    fn kick(&self) {
        let pv = self.pv.clone();
        let a = Adjust { resize_pct: 100, ..self.a };
        self.work.request((), move |_| pixels_of(&apply_adjust(&pv, &a)));
    }
}

type S = ToolS<Edit>;

fn adjust_slider(key: &'static str, name: &str, v: f32, lo: f32, hi: f32, th: &Theme, set: fn(&mut Adjust, f32)) -> El {
    row().items_center().gap(10.0).child(el().w(96.0).child(label(name, th))).child(slider(key, v, lo, hi, 200.0, th, move |s: &mut S, x| {
        set(&mut s.t.a, x);
        s.t.kick();
    }))
}

impl Tool for Edit {
    fn title(&self) -> String {
        "Edit".into()
    }
    fn size(&self) -> (f32, f32) {
        (960.0, 760.0)
    }
    fn setup(s: &mut S, waker: rune::Waker) {
        s.t.work.set_waker(waker);
        s.t.kick();
    }
    fn poll(s: &mut S, _tx: &mut Tx) -> bool {
        match s.t.work.take() {
            Some(px) => {
                s.t.shown = Some(px);
                true
            }
            None => false,
        }
    }
    fn body(s: &S, _cx: &mut Cx<S>) -> El {
        let th = &s.theme;
        let e = &s.t;
        let a = e.a;
        let left = col()
            .gap(8.0)
            .child(adjust_slider("brightness", "Brightness", a.brightness, 0.4, 1.6, th, |a, v| a.brightness = v))
            .child(adjust_slider("contrast", "Contrast", a.contrast, 0.4, 1.6, th, |a, v| a.contrast = v))
            .child(adjust_slider("saturation", "Saturation", a.saturation, 0.0, 2.0, th, |a, v| a.saturation = v));
        let right = col()
            .gap(8.0)
            .child(adjust_slider("sharpness", "Sharpness", a.sharpness, 0.0, 3.0, th, |a, v| a.sharpness = v))
            .child(adjust_slider("warmth", "Warmth", a.warmth, -1.0, 1.0, th, |a, v| a.warmth = v))
            .child(row().items_center().gap(10.0).child(adjust_slider("size", "Size", a.resize_pct as f32, 10.0, 100.0, th, |a, v| a.resize_pct = v.round() as u32)).child(dim(format!("{}%", a.resize_pct), th)));
        let (w, h) = (e.full.0 * a.resize_pct / 100, e.full.1 * a.resize_pct / 100);
        let (w, h) = if a.rotate % 180 == 90 { (h, w) } else { (w, h) };
        let buttons = row()
            .w_full()
            .gap(8.0)
            .items_center()
            .child(pill("auto", "✦ Auto", th).on_press(|s: &mut S| {
                let auto = auto_enhance(&s.t.pv);
                let a = s.t.a;
                s.t.a = Adjust { rotate: a.rotate, flip_h: a.flip_h, flip_v: a.flip_v, grayscale: a.grayscale, resize_pct: a.resize_pct, ..auto };
                s.t.kick();
            }))
            .child(pill("rotate-left", "⟲", th).help("Rotate left").on_press(|s: &mut S| {
                s.t.a.rotate = (s.t.a.rotate + 270).rem_euclid(360);
                s.t.kick();
            }))
            .child(pill("rotate-right", "⟳", th).help("Rotate right").on_press(|s: &mut S| {
                s.t.a.rotate = (s.t.a.rotate + 90).rem_euclid(360);
                s.t.kick();
            }))
            .child(pill("flip-h", "⇋", th).help("Flip horizontally").on_press(|s: &mut S| {
                s.t.a.flip_h = !s.t.a.flip_h;
                s.t.kick();
            }))
            .child(pill("flip-v", "⇵", th).help("Flip vertically").on_press(|s: &mut S| {
                s.t.a.flip_v = !s.t.a.flip_v;
                s.t.kick();
            }))
            .child(toggle("bw", a.grayscale, "B&W", th).on_press(|s: &mut S| {
                s.t.a.grayscale = !s.t.a.grayscale;
                s.t.kick();
            }))
            .child(pill("compare", "Hold to compare", th).on_drag(|s: &mut S, d, _tx| {
                s.t.compare = !matches!(d.phase, rune::Phase::Up);
            }))
            .child(rune::spacer())
            .child(dim(format!("{w} × {h}"), th).key("dims"));
        let px = if e.compare { Some(&e.orig) } else { e.shown.as_ref() };
        let preview = picture("preview", px).w_full().grow(1.0).min_h(120.0);
        let reset = pill("reset", "Reset", th).on_press(|s: &mut S| {
            s.t.a = Adjust::default();
            s.t.kick();
        });
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .gap(12.0)
            .child(row().w_full().gap(28.0).child(left).child(right))
            .child(buttons)
            .child(preview)
            .child(footer(s, "Save", true, Some(reset)))
    }
    fn enter(s: &mut S, tx: &mut Tx) {
        let src = s.t.paths[0].clone();
        let a = s.t.a;
        s.submit(tx, "tool:Edit", &[src.clone()], "Saving…", String::new(), move || Ok(vec![utsuri_engine::imaging::edit_and_save(&src, &a)?]));
    }
}
