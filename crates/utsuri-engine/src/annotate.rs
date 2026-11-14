//! Annotations drawn into the image at full resolution: arrows, boxes, circles, pen,
//! marker, text and numbered steps. Ported from tools.py (Shape, draw_shape).

use anyhow::Result;
use image::RgbaImage;
use std::path::{Path, PathBuf};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Arrow,
    Rect,
    Ellipse,
    Pen,
    Highlight,
    Text,
    Step,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub kind: Kind,
    /// Image pixels. Arrow/Rect/Ellipse: [start, end]; Pen/Highlight: the stroke;
    /// Text/Step: [anchor] (text: top-left; step: centre).
    pub pts: Vec<(f32, f32)>,
    pub color: [u8; 3],
    /// Line width in image pixels (text and step sizes derive from it).
    pub width: f32,
    pub text: String,
    /// The number on a Step.
    pub n: u32,
}

pub const COLORS: &[[u8; 3]] = &[[0xff, 0x3b, 0x30], [0xff, 0x95, 0x00], [0xff, 0xcc, 0x00], [0x34, 0xc7, 0x59], [0x00, 0x7a, 0xff], [0xaf, 0x52, 0xde], [0xff, 0xff, 0xff], [0, 0, 0]];

fn paint(c: [u8; 3], a: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], a);
    p.anti_alias = true;
    p
}

fn stroke(w: f32) -> Stroke {
    Stroke { width: w.max(1.0), line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() }
}

fn font_bytes(bold: bool) -> Option<&'static [u8]> {
    use std::sync::OnceLock;
    static R: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    static B: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    let load = |name: &str| crate::fonts::read(name);
    if bold { B.get_or_init(|| load("segoeuib.ttf")).as_deref() } else { R.get_or_init(|| load("segoeui.ttf")).as_deref() }
}

/// Text size for a shape of line width `w`.
pub fn text_size(w: f32) -> f32 {
    (w * 7.0).max(14.0)
}

pub fn text_width(text: &str, size: f32) -> f32 {
    use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont};
    let Some(data) = font_bytes(true) else { return text.chars().count() as f32 * size * 0.6 };
    let Ok(font) = FontRef::try_from_slice(data) else { return 0.0 };
    let sf = font.as_scaled(PxScale::from(size));
    text.chars().map(|c| sf.h_advance(sf.glyph_id(c))).sum()
}

/// Draw `text` with its baseline-left at (x, y).
fn draw_text(pm: &mut Pixmap, x: f32, y: f32, size: f32, text: &str, color: [u8; 3]) {
    use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont};
    let Some(bytes) = font_bytes(true) else { return };
    let Ok(font) = FontRef::try_from_slice(bytes) else { return };
    let scale = PxScale::from(size);
    let sf = font.as_scaled(scale);
    let mut cx = x;
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    for ch in text.chars() {
        let id = sf.glyph_id(ch);
        let g = id.with_scale_and_position(scale, ab_glyph::point(cx, y));
        cx += sf.h_advance(id);
        let Some(o) = font.outline_glyph(g) else { continue };
        let b = o.px_bounds();
        o.draw(|gx, gy, cov| {
            let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
            if px < 0 || py < 0 || px >= w || py >= h {
                return;
            }
            let i = ((py * w + px) * 4) as usize;
            let a = cov.clamp(0.0, 1.0);
            for k in 0..3 {
                data[i + k] = (color[k] as f32 * a + data[i + k] as f32 * (1.0 - a)).round() as u8;
            }
            data[i + 3] = (255.0 * a + data[i + 3] as f32 * (1.0 - a)).round() as u8;
        });
    }
}

fn light(c: [u8; 3]) -> bool {
    c.iter().map(|v| *v as u32).sum::<u32>() > 382
}

fn draw_shape(pm: &mut Pixmap, s: &Shape) {
    let t = Transform::identity();
    let w = s.width;
    match s.kind {
        Kind::Arrow if s.pts.len() >= 2 => {
            let ((x0, y0), (x1, y1)) = (s.pts[0], s.pts[s.pts.len() - 1]);
            let ang = (y1 - y0).atan2(x1 - x0);
            let head = (w * 4.5).max(12.0);
            let mut pb = PathBuilder::new();
            // the shaft stops inside the head so the tip stays sharp
            pb.move_to(x0, y0);
            pb.line_to(x1 - head * 0.6 * ang.cos(), y1 - head * 0.6 * ang.sin());
            if let Some(p) = pb.finish() {
                pm.stroke_path(&p, &paint(s.color, 255), &stroke(w), t, None);
            }
            let mut hb = PathBuilder::new();
            hb.move_to(x1, y1);
            hb.line_to(x1 - head * (ang - 0.45).cos(), y1 - head * (ang - 0.45).sin());
            hb.line_to(x1 - head * (ang + 0.45).cos(), y1 - head * (ang + 0.45).sin());
            hb.close();
            if let Some(p) = hb.finish() {
                pm.fill_path(&p, &paint(s.color, 255), FillRule::Winding, t, None);
            }
        }
        Kind::Rect | Kind::Ellipse if s.pts.len() >= 2 => {
            let ((x0, y0), (x1, y1)) = (s.pts[0], s.pts[s.pts.len() - 1]);
            let (l, tp) = (x0.min(x1), y0.min(y1));
            let Some(r) = tiny_skia::Rect::from_ltrb(l, tp, x0.max(x1).max(l + 1.0), y0.max(y1).max(tp + 1.0)) else { return };
            let p = if s.kind == Kind::Rect { Some(PathBuilder::from_rect(r)) } else { PathBuilder::from_oval(r) };
            if let Some(p) = p {
                pm.stroke_path(&p, &paint(s.color, 255), &stroke(w), t, None);
            }
        }
        Kind::Pen | Kind::Highlight if !s.pts.is_empty() => {
            let mut pb = PathBuilder::new();
            pb.move_to(s.pts[0].0, s.pts[0].1);
            for &(x, y) in &s.pts[1..] {
                pb.line_to(x, y);
            }
            if s.pts.len() == 1 {
                pb.line_to(s.pts[0].0 + 0.1, s.pts[0].1);
            }
            if let Some(p) = pb.finish() {
                let (a, mut st) = if s.kind == Kind::Highlight { (100, stroke(w * 4.0)) } else { (255, stroke(w)) };
                if s.kind == Kind::Highlight {
                    st.line_cap = LineCap::Butt;
                }
                pm.stroke_path(&p, &paint(s.color, a), &st, t, None);
            }
        }
        Kind::Text if !s.pts.is_empty() && !s.text.is_empty() => {
            let size = text_size(w);
            let (x, y) = s.pts[0];
            // a thin halo keeps text readable on any photo
            let halo = if light(s.color) { [0, 0, 0] } else { [255, 255, 255] };
            let o = size / 24.0;
            for (dx, dy) in [(-o, 0.0), (o, 0.0), (0.0, -o), (0.0, o)] {
                draw_text(pm, x + dx, y + size * 0.8 + dy, size, &s.text, halo);
            }
            draw_text(pm, x, y + size * 0.8, size, &s.text, s.color);
        }
        Kind::Step if !s.pts.is_empty() => {
            let r = (w * 5.0).max(12.0);
            let (cx, cy) = s.pts[0];
            if let Some(c) = PathBuilder::from_circle(cx, cy, r) {
                pm.fill_path(&c, &paint(s.color, 255), FillRule::Winding, t, None);
                pm.stroke_path(&c, &paint([255, 255, 255], 255), &stroke((r / 7.0).max(1.5)), t, None);
            }
            let label = s.n.to_string();
            let size = r * 1.15;
            let fg = if light(s.color) && s.color != [0xff, 0x3b, 0x30] { [0, 0, 0] } else { [255, 255, 255] };
            draw_text(pm, cx - text_width(&label, size) / 2.0, cy + size * 0.36, size, &label, fg);
        }
        _ => {}
    }
}

/// The image with every shape drawn on it.
pub fn render(im: &RgbaImage, shapes: &[Shape]) -> RgbaImage {
    let (w, h) = im.dimensions();
    let Some(mut pm) = Pixmap::new(w, h) else { return im.clone() };
    // straight -> premultiplied
    for (d, s) in pm.data_mut().chunks_exact_mut(4).zip(im.pixels()) {
        let a = s.0[3] as u32;
        d[0] = (s.0[0] as u32 * a / 255) as u8;
        d[1] = (s.0[1] as u32 * a / 255) as u8;
        d[2] = (s.0[2] as u32 * a / 255) as u8;
        d[3] = s.0[3];
    }
    for s in shapes {
        draw_shape(&mut pm, s);
    }
    let mut out = RgbaImage::new(w, h);
    for (o, d) in out.pixels_mut().zip(pm.data().chunks_exact(4)) {
        let a = d[3] as u32;
        o.0 = if a == 0 { [0, 0, 0, 0] } else { [(d[0] as u32 * 255 / a).min(255) as u8, (d[1] as u32 * 255 / a).min(255) as u8, (d[2] as u32 * 255 / a).min(255) as u8, d[3]] };
    }
    out
}

pub fn annotate_and_save(src: &Path, shapes: &[Shape]) -> Result<PathBuf> {
    if shapes.is_empty() {
        crate::refuse!("Draw something first");
    }
    let im = crate::imgio::open_image(src)?;
    let had_alpha = im.color().has_alpha();
    let out = render(&im.to_rgba8(), shapes);
    let out = if had_alpha { image::DynamicImage::ImageRgba8(out) } else { image::DynamicImage::ImageRgb8(image::DynamicImage::ImageRgba8(out).to_rgb8()) };
    crate::imaging::save_like(&out, src, "Annotated", None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(kind: Kind, pts: Vec<(f32, f32)>) -> Shape {
        Shape { kind, pts, color: [255, 0, 0], width: 4.0, text: "Hi".into(), n: 1 }
    }

    #[test]
    fn every_kind_draws_ink_where_expected() {
        let base = RgbaImage::from_pixel(200, 150, image::Rgba([255, 255, 255, 255]));
        for (k, pts, probe) in [
            (Kind::Arrow, vec![(20.0, 20.0), (180.0, 20.0)], (100, 20)),
            (Kind::Rect, vec![(40.0, 40.0), (120.0, 100.0)], (40, 70)),
            (Kind::Ellipse, vec![(40.0, 40.0), (120.0, 100.0)], (80, 40)),
            (Kind::Pen, vec![(10.0, 140.0), (60.0, 130.0), (90.0, 140.0)], (60, 130)),
            (Kind::Highlight, vec![(10.0, 75.0), (190.0, 75.0)], (100, 75)),
            (Kind::Step, vec![(150.0, 110.0)], (150, 96)),
        ] {
            let out = render(&base, &[shape(k, pts)]);
            let p = out.get_pixel(probe.0, probe.1).0;
            assert!(p[1] < 200, "{k:?} at {probe:?}: {p:?}");
            assert_eq!(out.get_pixel(199, 0).0, [255, 255, 255, 255], "{k:?} corner untouched");
        }
        let t = render(&base, &[Shape { pts: vec![(20.0, 20.0)], ..shape(Kind::Text, vec![]) }]);
        assert!(t.pixels().any(|p| p.0[0] > 200 && p.0[1] < 80), "red text ink");
    }

    #[test]
    fn saves_beside() {
        let d = crate::testutil::tmpdir("annot");
        let src = d.join("a.png");
        crate::testutil::photo(80, 60).save(&src).unwrap();
        let o = annotate_and_save(&src, &[shape(Kind::Rect, vec![(5.0, 5.0), (50.0, 40.0)])]).unwrap();
        assert_eq!(o.file_name().unwrap(), "a Annotated.png");
        assert!(annotate_and_save(&src, &[]).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
