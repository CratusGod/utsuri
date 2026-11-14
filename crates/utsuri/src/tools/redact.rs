//! Redact: drag boxes over private things; black box, pixelate or blur, burned into pixels.

use super::subtitle;
use crate::ui::frame::{Tool, ToolS, footer};
use crate::ui::*;
use image::DynamicImage;
use rune::{Cx, Tx};
use std::path::PathBuf;
use std::sync::Arc;
use utsuri_engine::imaging::{RedactMode, redact};

const MODES: &[(&str, RedactMode)] = &[("Black box", RedactMode::Black), ("Pixelate", RedactMode::Pixelate), ("Blur", RedactMode::Blur)];

pub struct Redact {
    paths: Vec<PathBuf>,
    pv: Arc<DynamicImage>,
    px: rune::Pixels,
    full: (u32, u32),
    mode: usize,
    /// boxes in full-image pixels
    boxes: Vec<(i64, i64, i64, i64)>,
    cur: Option<((f32, f32), (f32, f32))>,
    work: rune::Latest<rune::Pixels>,
}

impl Redact {
    pub fn new(paths: Vec<PathBuf>) -> anyhow::Result<(Self, String)> {
        let full = utsuri_engine::imgio::open_image(&paths[0])?;
        let fd = (full.width(), full.height());
        let pv = if fd.0.max(fd.1) > 1400 { full.resize(1400, 1400, image::imageops::FilterType::Triangle) } else { full };
        let px = pixels_of(&pv);
        let sub = subtitle(&paths);
        Ok((Redact { paths, pv: Arc::new(pv), px, full: fd, mode: 0, boxes: vec![], cur: None, work: rune::Latest::default() }, sub))
    }

    /// Re-render the preview with the boxes burned in.
    fn refresh(&self) {
        let pv = self.pv.clone();
        let k = pv.width() as f64 / self.full.0 as f64;
        let boxes: Vec<_> = self.boxes.iter().map(|b| ((b.0 as f64 * k) as i64, (b.1 as f64 * k) as i64, (b.2 as f64 * k) as i64, (b.3 as f64 * k) as i64)).collect();
        let mode = MODES[self.mode].1;
        self.work.request((), move |_| pixels_of(&redact(&pv, &boxes, mode)));
    }

    fn undo(&mut self) {
        if self.boxes.pop().is_some() {
            self.refresh();
        }
    }

    fn full_f(&self) -> (f32, f32) {
        (self.full.0 as f32, self.full.1 as f32)
    }
}

type S = ToolS<Redact>;

impl Tool for Redact {
    fn title(&self) -> String {
        "Redact".into()
    }
    fn size(&self) -> (f32, f32) {
        (920.0, 760.0)
    }
    fn setup(s: &mut S, waker: rune::Waker) {
        s.t.work.set_waker(waker);
    }
    fn poll(s: &mut S, _tx: &mut Tx) -> bool {
        match s.t.work.take() {
            Some(px) => {
                s.t.px = px;
                true
            }
            None => false,
        }
    }
    fn key(s: &mut S, k: &rune::KeyEvent, _tx: &mut Tx) -> bool {
        if k.is_command('z') {
            s.t.undo();
            return true;
        }
        false
    }
    fn body(s: &S, cx: &mut Cx<S>) -> El {
        let th = &s.theme;
        let r = &s.t;
        let labels: Vec<&str> = MODES.iter().map(|m| m.0).collect();
        let bar = row()
            .w_full()
            .items_center()
            .gap(10.0)
            .child(toggles("mode", &labels, r.mode, th, |s: &mut S, i| {
                s.t.mode = i;
                s.t.refresh();
            }))
            .child(pill_on("undo", "Undo", !r.boxes.is_empty(), th).on_press(|s: &mut S| s.t.undo()))
            .child(rune::spacer())
            .child(dim(if r.boxes.is_empty() { "Drag over what to hide".to_string() } else { format!("{} area{}", r.boxes.len(), if r.boxes.len() != 1 { "s" } else { "" }) }, th).key("count"));
        let fit = cx.layout_size("canvas").map(|a| Fit::new(a, r.full_f()));
        let mut overlays = vec![];
        if let (Some(f), Some((a, b))) = (fit, r.cur) {
            let (x0, y0) = f.to_view(a.0.min(b.0), a.1.min(b.1));
            let (x1, y1) = f.to_view(a.0.max(b.0), a.1.max(b.1));
            overlays.push(boxed(x0, y0, x1 - x0, y1 - y0).radius(2.0).fill(black(0.45)).border(1.5, Color::WHITE));
        }
        let canvas = canvas_area("canvas", Some(&r.px), fit, overlays).cursor(rune::Cursor::Crosshair).on_drag(|s: &mut S, d, _tx| {
            let f = Fit::new([d.w, d.h], s.t.full_f());
            let (fw, fh) = s.t.full_f();
            let (x, y) = f.to_image(d.x, d.y);
            let q = (x.clamp(0.0, fw), y.clamp(0.0, fh));
            match d.phase {
                rune::Phase::Down => s.t.cur = Some((q, q)),
                rune::Phase::Move => {
                    if let Some(c) = s.t.cur.as_mut() {
                        c.1 = q;
                    }
                }
                rune::Phase::Up => {
                    if let Some((a, b)) = s.t.cur.take() {
                        if (a.0 - b.0).abs() > 3.0 && (a.1 - b.1).abs() > 3.0 {
                            s.t.boxes.push((a.0 as i64, a.1 as i64, b.0 as i64, b.1 as i64));
                            s.t.refresh();
                        }
                    }
                }
            }
        });
        let reset = pill("reset", "Reset", th).on_press(|s: &mut S| {
            s.t.boxes.clear();
            s.t.refresh();
        });
        col().w_full().grow(1.0).min_h(0.0).gap(12.0).child(bar).child(canvas).child(footer(s, "Save", !r.boxes.is_empty(), Some(reset)))
    }
    fn enter(s: &mut S, tx: &mut Tx) {
        if s.t.boxes.is_empty() {
            return;
        }
        let src = s.t.paths[0].clone();
        let (boxes, mode) = (s.t.boxes.clone(), MODES[s.t.mode]);
        let n = boxes.len();
        s.submit(tx, "tool:Redact", &[src.clone()], "Saving…", format!("{n} area{} · {}", if n != 1 { "s" } else { "" }, mode.0.to_lowercase()), move || Ok(vec![utsuri_engine::imaging::redact_and_save(&src, &boxes, mode.1)?]));
    }
}
