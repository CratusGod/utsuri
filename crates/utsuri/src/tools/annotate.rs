//! Annotate: arrows, boxes, circles, pen, marker, text and numbered steps, with undo.
//! Shapes live in full-image pixels; the canvas draws them scaled, the save renders them at
//! full resolution (utsuri_engine::annotate).

use super::subtitle;
use crate::ui::frame::{Tool, ToolS, footer};
use crate::ui::*;
use rune::shape::Shape as Vector;
use rune::{Cx, Tx, canvas};
use std::path::PathBuf;
use utsuri_engine::annotate::{COLORS, Kind, Shape, text_size};

const TOOLS: &[(&str, Kind)] = &[("➜ Arrow", Kind::Arrow), ("▢ Box", Kind::Rect), ("◯ Circle", Kind::Ellipse), ("✎ Pen", Kind::Pen), ("▬ Marker", Kind::Highlight), ("T Text", Kind::Text), ("① Step", Kind::Step)];

pub struct Annotate {
    paths: Vec<PathBuf>,
    px: rune::Pixels,
    full: (u32, u32),
    tool: usize,
    color: usize,
    width: f32,
    shapes: Vec<Shape>,
    redo: Vec<Shape>,
    cur: Option<Shape>,
    /// A text box being typed: anchor in image px, and the field.
    typing: Option<((f32, f32), rune::TextEdit)>,
}

impl Annotate {
    pub fn new(paths: Vec<PathBuf>) -> anyhow::Result<(Self, String)> {
        let full = utsuri_engine::imgio::open_image(&paths[0])?;
        let fd = (full.width(), full.height());
        let pv = if fd.0.max(fd.1) > 1600 { full.resize(1600, 1600, image::imageops::FilterType::Triangle) } else { full };
        let px = pixels_of(&pv);
        let sub = subtitle(&paths);
        Ok((Annotate { paths, px, full: fd, tool: 0, color: 0, width: 4.0, shapes: vec![], redo: vec![], cur: None, typing: None }, sub))
    }

    /// Line width in image pixels for the slider value (relative to a 1600 px image).
    fn px_width(&self) -> f32 {
        self.width * self.full.0.max(self.full.1) as f32 / 1600.0
    }

    fn next_step(&self) -> u32 {
        self.shapes.iter().filter(|s| s.kind == Kind::Step).count() as u32 + 1
    }

    fn commit_text(&mut self) {
        if let Some((at, edit)) = self.typing.take() {
            if !edit.text.trim().is_empty() {
                self.shapes.push(Shape { kind: Kind::Text, pts: vec![at], color: COLORS[self.color], width: self.px_width(), text: edit.text.clone(), n: 0 });
                self.redo.clear();
            }
        }
    }

    fn undo(&mut self) {
        if let Some(s) = self.shapes.pop() {
            self.redo.push(s);
        }
    }

    fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.shapes.push(s);
        }
    }

    fn full_f(&self) -> (f32, f32) {
        (self.full.0 as f32, self.full.1 as f32)
    }
}

fn rgb(c: [u8; 3], a: f32) -> Color {
    col_of(Rgba(c[0], c[1], c[2], 255)).alpha(a)
}

/// Draw a shape into the vector canvas and/or as elements (an approximation of the saved
/// rendering).
fn draw(v: &mut Vector, els: &mut Vec<El>, f: &Fit, s: &Shape) {
    let w = (s.width * f.s).max(1.0);
    let c = rgb(s.color, 1.0);
    let pt = |q: (f32, f32)| {
        let (x, y) = f.to_view(q.0, q.1);
        [x, y]
    };
    let first = s.pts.first().copied();
    let last = s.pts.last().copied();
    match (s.kind, first, last) {
        (Kind::Arrow, Some(a), Some(b)) if s.pts.len() >= 2 => {
            let (a, b) = (pt(a), pt(b));
            let ang = (b[1] - a[1]).atan2(b[0] - a[0]);
            let head = (w * 4.5).max(12.0 * f.s);
            let shaft = [b[0] - ang.cos() * head * 0.6, b[1] - ang.sin() * head * 0.6];
            let wing = |d: f32| [b[0] - (ang + d).cos() * head, b[1] - (ang + d).sin() * head];
            *v = std::mem::replace(v, Vector::new()).line(a, shaft, w, rune::shape::LineCap::Round, c).fill(vec![b, wing(-0.45), wing(0.45)], c);
        }
        (Kind::Rect, Some(a), Some(b)) => {
            let (a, b) = (pt(a), pt(b));
            *v = std::mem::replace(v, Vector::new()).stroke(vec![a, [b[0], a[1]], b, [a[0], b[1]], a], w, c);
        }
        (Kind::Ellipse, Some(a), Some(b)) => {
            let (a, b) = (pt(a), pt(b));
            let (cx_, cy_, rx, ry) = ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (b[0] - a[0]).abs() / 2.0, (b[1] - a[1]).abs() / 2.0);
            let pts: Vec<[f32; 2]> = (0..=64).map(|i| {
                let t = i as f32 / 64.0 * std::f32::consts::TAU;
                [cx_ + rx * t.cos(), cy_ + ry * t.sin()]
            }).collect();
            *v = std::mem::replace(v, Vector::new()).stroke(pts, w, c);
        }
        (Kind::Pen | Kind::Highlight, Some(_), _) => {
            let pts: Vec<[f32; 2]> = s.pts.iter().map(|q| pt(*q)).collect();
            let (cc, ww) = if s.kind == Kind::Highlight { (rgb(s.color, 0.4), w * 4.0) } else { (c, w) };
            if pts.len() == 1 {
                *v = std::mem::replace(v, Vector::new()).line(pts[0], pts[0], ww, rune::shape::LineCap::Round, cc);
            } else {
                *v = std::mem::replace(v, Vector::new()).stroke(pts, ww, cc);
            }
        }
        (Kind::Text, Some(a), _) => {
            let p = pt(a);
            let size = (text_size(s.width) * f.s).max(6.0);
            els.push(text(&s.text).font(font(size, 600.0)).color(c).absolute(p[0], p[1]));
        }
        (Kind::Step, Some(a), _) => {
            let p = pt(a);
            let r = (s.width * 5.0).max(12.0) * f.s;
            els.push(boxed(p[0] - r, p[1] - r, r * 2.0, r * 2.0).radius(r).fill(c).border((r / 7.0).max(1.0), Color::WHITE).center().child(text(s.n.to_string()).font(font(r * 1.15, 700.0)).color(Color::WHITE)));
        }
        _ => {}
    }
}

type S = ToolS<Annotate>;

impl Tool for Annotate {
    fn title(&self) -> String {
        "Annotate".into()
    }
    fn size(&self) -> (f32, f32) {
        (980.0, 780.0)
    }
    fn key(s: &mut S, k: &rune::KeyEvent, _tx: &mut Tx) -> bool {
        if s.t.typing.is_some() {
            return false;
        }
        if k.is_command('z') && !k.mods.shift {
            s.t.undo();
            return true;
        }
        if k.is_command('y') || (k.is_command('z') && k.mods.shift) {
            s.t.redo();
            return true;
        }
        false
    }
    fn body(s: &S, cx: &mut Cx<S>) -> El {
        let th = &s.theme;
        let a = &s.t;
        let labels: Vec<&str> = TOOLS.iter().map(|t| t.0).collect();
        let tools = toggles("tool", &labels, a.tool, th, |s: &mut S, i| {
            s.t.commit_text();
            s.t.tool = i;
        });
        let mut colors = row().gap(6.0).items_center();
        for (i, c) in COLORS.iter().enumerate() {
            let on = a.color == i;
            colors = colors.child(
                el().key(format!("color/{i}"))
                    .size(24.0, 24.0)
                    .radius(12.0)
                    .fill(rgb(*c, 1.0))
                    .border(if on { 3.0 } else { 1.0 }, if on { Color::WHITE } else { white(0.3) })
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |s: &mut S| s.t.color = i),
            );
        }
        let bar2 = row()
            .w_full()
            .items_center()
            .gap(12.0)
            .child(colors)
            .child(label("Width", th))
            .child(slider("width", a.width, 1.0, 16.0, 160.0, th, |s: &mut S, v| s.t.width = v))
            .child(rune::spacer())
            .child(pill_on("undo", "Undo", !a.shapes.is_empty(), th).on_press(|s: &mut S| s.t.undo()))
            .child(pill_on("redo", "Redo", !a.redo.is_empty(), th).on_press(|s: &mut S| s.t.redo()));

        let fit = cx.layout_size("canvas").map(|area| Fit::new(area, a.full_f()));
        let mut overlays = vec![];
        if let Some(f) = fit {
            let mut v = Vector::new();
            let mut els = vec![];
            for sh in a.shapes.iter().chain(a.cur.iter()) {
                draw(&mut v, &mut els, &f, sh);
            }
            overlays.push(canvas(v).absolute(0.0, 0.0).w_full().h_full());
            overlays.extend(els);
            if let Some((at, edit)) = &a.typing {
                let (x, y) = f.to_view(at.0, at.1);
                overlays.push(
                    rune_kit::TextInput::new(edit)
                        .placeholder("Type, then Enter")
                        .width(Some(320.0))
                        .on_edit(|s: &mut S| &mut s.t.typing.as_mut().expect("typing").1)
                        .on_submit(|s: &mut S, _tx| s.t.commit_text())
                        .on_key(|s: &mut S, k, _tx| {
                            if matches!(k.key, rune::Key::Escape) {
                                s.t.typing = None;
                                return true;
                            }
                            false
                        })
                        .build(&s.kit)
                        .key("typing")
                        .absolute(x, y)
                        .autofocus(),
                );
            }
        }
        let cv = canvas_area("canvas", Some(&a.px), fit, overlays).cursor(rune::Cursor::Crosshair).on_drag(|s: &mut S, d, tx| {
            let f = Fit::new([d.w, d.h], s.t.full_f());
            let (fw, fh) = s.t.full_f();
            let (x, y) = f.to_image(d.x, d.y);
            let q = (x.clamp(0.0, fw), y.clamp(0.0, fh));
            let kind = TOOLS[s.t.tool].1;
            let t = &mut s.t;
            match (d.phase, kind) {
                (rune::Phase::Down, Kind::Step) => {
                    let n = t.next_step();
                    t.shapes.push(Shape { kind, pts: vec![q], color: COLORS[t.color], width: t.px_width(), text: String::new(), n });
                    t.redo.clear();
                }
                (rune::Phase::Down, Kind::Text) => {
                    t.commit_text();
                    t.typing = Some((q, rune::TextEdit::new("")));
                    tx.focus("typing", false);
                }
                (rune::Phase::Down, _) => {
                    t.commit_text();
                    t.cur = Some(Shape { kind, pts: vec![q, q], color: COLORS[t.color], width: t.px_width(), text: String::new(), n: 0 });
                }
                (rune::Phase::Move, _) => {
                    if let Some(sh) = t.cur.as_mut() {
                        if matches!(sh.kind, Kind::Pen | Kind::Highlight) {
                            if sh.pts.last().is_none_or(|l| (l.0 - q.0).abs() + (l.1 - q.1).abs() > 1.5) {
                                sh.pts.push(q);
                            }
                        } else if let Some(l) = sh.pts.last_mut() {
                            *l = q;
                        }
                    }
                }
                (rune::Phase::Up, _) => {
                    if let Some(sh) = t.cur.take() {
                        let tiny = sh.pts.len() == 2 && (sh.pts[0].0 - sh.pts[1].0).abs() + (sh.pts[0].1 - sh.pts[1].1).abs() < 3.0 && sh.kind != Kind::Pen;
                        if !tiny {
                            t.shapes.push(sh);
                            t.redo.clear();
                        }
                    }
                }
            }
        });
        let reset = pill("reset", "Reset", th).on_press(|s: &mut S| {
            s.t.shapes.clear();
            s.t.redo.clear();
            s.t.typing = None;
        });
        col().w_full().grow(1.0).min_h(0.0).gap(10.0).child(tools).child(bar2).child(cv).child(footer(s, "Save", !a.shapes.is_empty() || a.typing.is_some(), Some(reset)))
    }
    fn enter(s: &mut S, tx: &mut Tx) {
        s.t.commit_text();
        log::debug!("annotate: Enter with {} mark(s), tool {}", s.t.shapes.len(), s.t.tool);
        if s.t.shapes.is_empty() {
            return;
        }
        let src = s.t.paths[0].clone();
        let shapes = s.t.shapes.clone();
        let n = shapes.len();
        s.submit(tx, "tool:Annotate", &[src.clone()], "Saving…", format!("{n} mark{}", if n != 1 { "s" } else { "" }), move || Ok(vec![utsuri_engine::annotate::annotate_and_save(&src, &shapes)?]));
    }
}
