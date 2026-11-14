//! Crop: drag a box (or its edges and corners), lock a ratio, rotate in 90° steps.

use super::subtitle;
use crate::ui::frame::{Tool, ToolS, footer};
use crate::ui::*;
use image::DynamicImage;
use rune::{Cx, Tx};
use std::path::PathBuf;

const RATIOS: &[(&str, Option<f32>)] = &[("Free", None), ("Original", Some(-1.0)), ("1:1", Some(1.0)), ("4:3", Some(4.0 / 3.0)), ("3:2", Some(1.5)), ("16:9", Some(16.0 / 9.0)), ("9:16", Some(9.0 / 16.0)), ("4:5", Some(0.8))];
const HANDLE: f32 = 10.0;

#[derive(Clone, Copy, PartialEq)]
enum Grab {
    New((f32, f32)),
    Move((f32, f32), [f32; 4]),
    /// Which edges move: left, top, right, bottom.
    Edge([bool; 4]),
}

pub struct Crop {
    paths: Vec<PathBuf>,
    base: DynamicImage,
    /// preview pixels per full-image pixel
    k: f32,
    rotate: i32,
    px: rune::Pixels,
    /// x0, y0, x1, y1 in (rotated) preview pixels
    rect: [f32; 4],
    ratio: usize,
    grab: Option<Grab>,
    full: (u32, u32),
}

impl Crop {
    pub fn new(paths: Vec<PathBuf>) -> anyhow::Result<(Self, String)> {
        let full = utsuri_engine::imgio::open_image(&paths[0])?;
        let fdims = (full.width(), full.height());
        let base = if fdims.0.max(fdims.1) > 1400 { full.resize(1400, 1400, image::imageops::FilterType::Triangle) } else { full };
        let k = base.width() as f32 / fdims.0 as f32;
        let px = pixels_of(&base);
        let rect = [0.0, 0.0, base.width() as f32, base.height() as f32];
        let sub = subtitle(&paths);
        Ok((Crop { paths, px, rect, base, k, rotate: 0, ratio: 0, grab: None, full: fdims }, sub))
    }

    fn view_image(&self) -> DynamicImage {
        match self.rotate.rem_euclid(360) {
            90 => self.base.rotate90(),
            180 => self.base.rotate180(),
            270 => self.base.rotate270(),
            _ => self.base.clone(),
        }
    }

    fn dims(&self) -> (f32, f32) {
        let (w, h) = (self.base.width() as f32, self.base.height() as f32);
        if self.rotate.rem_euclid(180) == 90 { (h, w) } else { (w, h) }
    }

    fn ratio_value(&self) -> Option<f32> {
        RATIOS[self.ratio].1.map(|r| if r < 0.0 { let (w, h) = self.dims(); w / h } else { r })
    }

    fn whole(&mut self) {
        let (w, h) = self.dims();
        self.rect = [0.0, 0.0, w, h];
    }

    /// Largest box of the current ratio, centred.
    fn fit_ratio(&mut self) {
        let (w, h) = self.dims();
        let Some(r) = self.ratio_value() else { return self.whole() };
        let (bw, bh) = if w / h > r { (h * r, h) } else { (w, w / r) };
        self.rect = [(w - bw) / 2.0, (h - bh) / 2.0, (w + bw) / 2.0, (h + bh) / 2.0];
    }

    fn clamp(&mut self) {
        let (w, h) = self.dims();
        let r = &mut self.rect;
        if r[0] > r[2] {
            r.swap(0, 2);
        }
        if r[1] > r[3] {
            r.swap(1, 3);
        }
        r[0] = r[0].clamp(0.0, w);
        r[2] = r[2].clamp(0.0, w);
        r[1] = r[1].clamp(0.0, h);
        r[3] = r[3].clamp(0.0, h);
    }

    fn rotate_by(&mut self, d: i32) {
        self.rotate = (self.rotate + d).rem_euclid(360);
        self.px = pixels_of(&self.view_image());
        self.fit_ratio();
    }

    /// Full-image pixel rect (x, y, w, h) of the box, in the rotated image.
    fn pixel_rect(&self) -> (u32, u32, u32, u32) {
        let s = 1.0 / self.k;
        let (fw, fh) = if self.rotate.rem_euclid(180) == 90 { (self.full.1, self.full.0) } else { self.full };
        let x = ((self.rect[0] * s).round() as u32).min(fw.saturating_sub(1));
        let y = ((self.rect[1] * s).round() as u32).min(fh.saturating_sub(1));
        let w = (((self.rect[2] - self.rect[0]) * s).round() as u32).clamp(1, fw - x);
        let h = (((self.rect[3] - self.rect[1]) * s).round() as u32).clamp(1, fh - y);
        (x, y, w, h)
    }

    fn drag(&mut self, f: Fit, d: &rune::Drag) {
        let (w, h) = self.dims();
        let (mx, my) = f.to_image(d.x, d.y);
        match d.phase {
            rune::Phase::Down => {
                let (vx0, vy0) = f.to_view(self.rect[0], self.rect[1]);
                let (vx1, vy1) = f.to_view(self.rect[2], self.rect[3]);
                let near = |a: f32, b: f32| (a - b).abs() < HANDLE;
                let edges = [near(d.x, vx0), near(d.y, vy0), near(d.x, vx1), near(d.y, vy1)];
                let in_band = d.x > vx0 - HANDLE && d.x < vx1 + HANDLE && d.y > vy0 - HANDLE && d.y < vy1 + HANDLE;
                let inside = d.x > vx0 && d.x < vx1 && d.y > vy0 && d.y < vy1;
                self.grab = Some(if in_band && edges.iter().any(|e| *e) {
                    Grab::Edge(edges)
                } else if inside {
                    Grab::Move((mx, my), self.rect)
                } else {
                    Grab::New((mx.clamp(0.0, w), my.clamp(0.0, h)))
                });
            }
            rune::Phase::Move => match self.grab {
                Some(Grab::New((x0, y0))) => {
                    let (mut x1, mut y1) = (mx.clamp(0.0, w), my.clamp(0.0, h));
                    if let Some(r) = self.ratio_value() {
                        let (dx, dy) = (x1 - x0, y1 - y0);
                        let bw = dx.abs().max(dy.abs() * r);
                        x1 = x0 + bw * dx.signum();
                        y1 = y0 + bw / r * if dy == 0.0 { 1.0 } else { dy.signum() };
                    }
                    self.rect = [x0, y0, x1, y1];
                    self.clamp();
                }
                Some(Grab::Move((sx, sy), r0)) => {
                    let (bw, bh) = (r0[2] - r0[0], r0[3] - r0[1]);
                    let x = (r0[0] + mx - sx).clamp(0.0, w - bw);
                    let y = (r0[1] + my - sy).clamp(0.0, h - bh);
                    self.rect = [x, y, x + bw, y + bh];
                }
                Some(Grab::Edge(e)) => {
                    if e[0] {
                        self.rect[0] = mx;
                    }
                    if e[1] {
                        self.rect[1] = my;
                    }
                    if e[2] {
                        self.rect[2] = mx;
                    }
                    if e[3] {
                        self.rect[3] = my;
                    }
                    if let Some(r) = self.ratio_value() {
                        let bh = (self.rect[2] - self.rect[0]) / r;
                        if e[1] && !e[3] {
                            self.rect[1] = self.rect[3] - bh;
                        } else {
                            self.rect[3] = self.rect[1] + bh;
                        }
                    }
                    self.clamp();
                }
                None => {}
            },
            rune::Phase::Up => {
                self.grab = None;
                self.clamp();
                if self.rect[2] - self.rect[0] < 4.0 || self.rect[3] - self.rect[1] < 4.0 {
                    self.whole();
                }
            }
        }
    }
}

type S = ToolS<Crop>;

impl Tool for Crop {
    fn title(&self) -> String {
        "Crop".into()
    }
    fn size(&self) -> (f32, f32) {
        (900.0, 760.0)
    }
    fn body(s: &S, cx: &mut Cx<S>) -> El {
        let th = &s.theme;
        let c = &s.t;
        let labels: Vec<&str> = RATIOS.iter().map(|r| r.0).collect();
        let ratios = row().items_center().gap(12.0).child(label("Aspect ratio", th)).child(toggles("ratio", &labels, c.ratio, th, |s: &mut S, i| {
            s.t.ratio = i;
            s.t.fit_ratio();
        }));
        let (_, _, pw, ph) = c.pixel_rect();
        let tools = row()
            .items_center()
            .gap(8.0)
            .child(pill("rotate-left", "⟲ Rotate left", th).on_press(|s: &mut S| s.t.rotate_by(270)))
            .child(pill("rotate-right", "⟳ Rotate right", th).on_press(|s: &mut S| s.t.rotate_by(90)))
            .child(el().w(8.0))
            .child(text(format!("{pw} × {ph} px")).key("size").font(STRONG).color(col_of(th.label())));
        let fit = cx.layout_size("canvas").map(|a| Fit::new(a, c.dims()));
        let mut overlays = vec![];
        if let Some(f) = fit {
            let (x0, y0) = f.to_view(c.rect[0].min(c.rect[2]), c.rect[1].min(c.rect[3]));
            let (x1, y1) = f.to_view(c.rect[0].max(c.rect[2]), c.rect[1].max(c.rect[3]));
            let r = f.rect();
            let shade = black(0.55);
            // dim outside the box
            overlays.push(boxed(r[0], r[1], r[2], y0 - r[1]).fill(shade));
            overlays.push(boxed(r[0], y1, r[2], r[1] + r[3] - y1).fill(shade));
            overlays.push(boxed(r[0], y0, x0 - r[0], y1 - y0).fill(shade));
            overlays.push(boxed(x1, y0, r[0] + r[2] - x1, y1 - y0).fill(shade));
            // the box, its thirds and corner handles
            overlays.push(boxed(x0, y0, x1 - x0, y1 - y0).key("box").border(2.0, Color::WHITE));
            for i in 1..3 {
                let t = i as f32 / 3.0;
                overlays.push(boxed(x0 + (x1 - x0) * t, y0, 1.0, y1 - y0).fill(white(0.3)));
                overlays.push(boxed(x0, y0 + (y1 - y0) * t, x1 - x0, 1.0).fill(white(0.3)));
            }
            for (cx_, cy_) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
                overlays.push(boxed(cx_ - 6.0, cy_ - 6.0, 12.0, 12.0).radius(6.0).fill(Color::WHITE).shadow(1.0, 4.0, black(0.4)));
            }
        }
        let canvas = canvas_area("canvas", Some(&c.px), fit, overlays).cursor(rune::Cursor::Crosshair).on_drag(|s: &mut S, d, _tx| {
            let f = Fit::new([d.w, d.h], s.t.dims());
            s.t.drag(f, d);
        });
        let reset = pill("reset", "Reset", th).on_press(|s: &mut S| {
            s.t.rotate = 0;
            s.t.ratio = 0;
            s.t.px = pixels_of(&s.t.base);
            s.t.whole();
        });
        col().w_full().grow(1.0).min_h(0.0).gap(12.0).child(ratios).child(tools).child(canvas).child(footer(s, "Apply", true, Some(reset)))
    }
    fn enter(s: &mut S, tx: &mut Tx) {
        let src = s.t.paths[0].clone();
        let (rect, rot) = (s.t.pixel_rect(), s.t.rotate);
        s.submit(tx, "tool:Crop", &[src.clone()], "Cropping…", String::new(), move || {
            let im = utsuri_engine::imgio::open_image(&src)?;
            let im = match rot {
                90 => im.rotate90(),
                180 => im.rotate180(),
                270 => im.rotate270(),
                _ => im,
            };
            let (x, y, w, h) = rect;
            let w = w.min(im.width().saturating_sub(x)).max(1);
            let h = h.min(im.height().saturating_sub(y)).max(1);
            Ok(vec![utsuri_engine::imaging::save_like(&im.crop_imm(x, y, w, h), &src, "Cropped", None)?])
        });
    }
}
