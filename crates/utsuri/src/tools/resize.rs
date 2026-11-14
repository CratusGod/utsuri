//! Resize: presets (percent or longest side) or an exact size with proportions locked.

use super::subtitle;
use crate::ui::frame::{Tool, ToolS, footer};
use crate::ui::*;
use rune::{Cx, Tx};
use std::path::{Path, PathBuf};
use utsuri_engine::imaging::{ResizeMode, resize_dims};

const PRESETS: &[(&str, ResizeMode, u32)] = &[
    ("25%", ResizeMode::Percent, 25),
    ("50%", ResizeMode::Percent, 50),
    ("75%", ResizeMode::Percent, 75),
    ("3840", ResizeMode::LongSide, 3840),
    ("2560", ResizeMode::LongSide, 2560),
    ("1920", ResizeMode::LongSide, 1920),
    ("1280", ResizeMode::LongSide, 1280),
    ("800", ResizeMode::LongSide, 800),
];

pub struct Resize {
    paths: Vec<PathBuf>,
    orig: (u32, u32),
    preset: Option<usize>,
    w: u32,
    h: u32,
    lock: bool,
    w_edit: rune::TextEdit,
    h_edit: rune::TextEdit,
}

/// Camera-rotated photos (EXIF orientation 5–8) are taller than they're stored.
fn upright(p: &Path, (w, h): (u32, u32)) -> (u32, u32) {
    let rotated = std::fs::File::open(p)
        .ok()
        .and_then(|f| exif::Reader::new().read_from_container(&mut std::io::BufReader::new(f)).ok())
        .and_then(|e| e.get_field(exif::Tag::Orientation, exif::In::PRIMARY).and_then(|f| f.value.get_uint(0)))
        .is_some_and(|o| (5..=8).contains(&o));
    if rotated { (h, w) } else { (w, h) }
}

impl Resize {
    pub fn new(paths: Vec<PathBuf>) -> anyhow::Result<(Self, String)> {
        let orig = image::image_dimensions(&paths[0]).ok().map(|d| upright(&paths[0], d)).or_else(|| utsuri_engine::imgio::open_image(&paths[0]).ok().map(|i| (i.width(), i.height()))).unwrap_or((1, 1));
        let (w, h) = resize_dims(orig, ResizeMode::Percent, 50);
        let sub = subtitle(&paths);
        Ok((Resize { paths, orig, preset: Some(1), w, h, lock: true, w_edit: rune::TextEdit::new(w.to_string()), h_edit: rune::TextEdit::new(h.to_string()) }, sub))
    }

    fn set_size(&mut self, w: u32, h: u32) {
        (self.w, self.h) = (w.max(1), h.max(1));
        self.w_edit = rune::TextEdit::new(self.w.to_string());
        self.h_edit = rune::TextEdit::new(self.h.to_string());
    }
}

type S = ToolS<Resize>;

fn typed(e: &rune::TextEdit) -> Option<u32> {
    e.text.trim().parse::<u32>().ok().filter(|v| (1..=30000).contains(v))
}

impl Tool for Resize {
    fn title(&self) -> String {
        "Resize".into()
    }
    fn size(&self) -> (f32, f32) {
        (640.0, 430.0)
    }
    fn body(s: &S, _cx: &mut Cx<S>) -> El {
        let th = &s.theme;
        let r = &s.t;
        let labels: Vec<&str> = PRESETS.iter().map(|p| p.0).collect();
        let presets = toggles("preset", &labels, r.preset.unwrap_or(usize::MAX), th, |s: &mut S, i| {
            s.t.preset = Some(i);
            let (w, h) = resize_dims(s.t.orig, PRESETS[i].1, PRESETS[i].2);
            s.t.set_size(w, h);
        });
        let num = |key: &str, e: &rune::TextEdit, w_side: bool| {
            rune_kit::TextInput::new(e)
                .width(Some(90.0))
                .on_edit(move |s: &mut S| if w_side { &mut s.t.w_edit } else { &mut s.t.h_edit })
                .on_change(move |s: &mut S, _| {
                    let r = &mut s.t;
                    let v = if w_side { typed(&r.w_edit) } else { typed(&r.h_edit) };
                    let Some(v) = v else { return };
                    r.preset = None;
                    if w_side {
                        r.w = v;
                        if r.lock {
                            r.h = ((v as f64 * r.orig.1 as f64 / r.orig.0 as f64).round() as u32).max(1);
                            r.h_edit = rune::TextEdit::new(r.h.to_string());
                        }
                    } else {
                        r.h = v;
                        if r.lock {
                            r.w = ((v as f64 * r.orig.0 as f64 / r.orig.1 as f64).round() as u32).max(1);
                            r.w_edit = rune::TextEdit::new(r.w.to_string());
                        }
                    }
                })
                .build(&s.kit)
                .key(key)
        };
        let exact = row()
            .items_center()
            .gap(10.0)
            .child(label("W", th))
            .child(num("w", &r.w_edit, true))
            .child(label("H", th))
            .child(num("h", &r.h_edit, false))
            .child(dim("px", th))
            .child(el().w(10.0))
            .child(checkbox("lock", r.lock, "Keep proportions", th).on_press(|s: &mut S| s.t.lock = !s.t.lock));
        let mut c = col()
            .w_full()
            .grow(1.0)
            .gap(12.0)
            .child(section("Presets — percent or longest side in px", th))
            .child(presets)
            .child(section("Exact size", th))
            .child(exact)
            .child(text(format!("{} × {}  →  {} × {}", r.orig.0, r.orig.1, r.w, r.h)).key("result").font(font(18.0, 650.0)).color(col_of(th.label())).pt(6.0));
        if r.paths.len() > 1 {
            c = c.child(dim("Several images: each is scaled by the same factor (or to the same longest side).", th));
        }
        c.child(rune::spacer()).child(footer(s, "Resize", true, None))
    }
    fn enter(s: &mut S, tx: &mut Tx) {
        let paths = s.t.paths.clone();
        let (orig, w, h, preset) = (s.t.orig, s.t.w, s.t.h, s.t.preset.map(|i| (PRESETS[i].1, PRESETS[i].2)));
        s.submit(tx, "tool:Resize", &paths.clone(), "Resizing…", String::new(), move || {
            paths
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    if i == 0 && preset.is_none() {
                        return utsuri_engine::imaging::resize_image(p, (w, h));
                    }
                    let d = image::image_dimensions(p).map(|d| upright(p, d)).unwrap_or(orig);
                    let dims = match preset {
                        Some((m, v)) => resize_dims(d, m, v),
                        None => (((d.0 as f64) * w as f64 / orig.0 as f64).round() as u32, ((d.1 as f64) * h as f64 / orig.1 as f64).round() as u32),
                    };
                    utsuri_engine::imaging::resize_image(p, (dims.0.max(1), dims.1.max(1)))
                })
                .collect()
        });
    }
}
