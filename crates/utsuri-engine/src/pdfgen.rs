//! New PDFs: images to pages, plain text, and Word documents (a small flow layout with
//! headings, bold/italic/underline runs, lists and tables). Ported from convert.py
//! (images_to_single_pdf, html_to_pdf, text_to_pdf, docx_to_pdf's built-in renderer).
//!
//! Fonts come from Windows (Segoe UI, Consolas), embedded as subsets, so Vietnamese and
//! other non-Latin text comes out right.

use crate::docx::{Block, Run, Style};
use anyhow::{Context as _, Result, anyhow};
use image::DynamicImage;
use krilla::Document;
use krilla::geom::{PathBuilder, Point, Size, Transform};
use krilla::image::Image;
use krilla::page::PageSettings;
use krilla::paint::{Fill, Stroke};
use krilla::text::{Font, TextDirection};
use std::sync::OnceLock;

/// Images are placed at this resolution (as the Python app did).
pub const IMAGE_DPI: f32 = 150.0;
const LETTER: (f32, f32) = (612.0, 792.0);
const MARGIN: f32 = 54.0;

/// A system font (`crate::fonts`), with its metrics for line breaking.
#[derive(Clone)]
struct Face {
    font: Font,
    data: std::sync::Arc<Vec<u8>>,
}

impl Face {
    fn load(names: &[&str]) -> Result<Face> {
        for n in names {
            if let Some(bytes) = crate::fonts::read(n) {
                let data = std::sync::Arc::new(bytes);
                if let Some(font) = Font::new(data.clone().into(), 0) {
                    return Ok(Face { font, data });
                }
            }
        }
        Err(anyhow!("none of these fonts are installed: {names:?}"))
    }

    /// Width of `s` in points at `size`.
    fn width(&self, s: &str, size: f32) -> f32 {
        let Ok(f) = ttf_parser::Face::parse(&self.data, 0) else { return s.chars().count() as f32 * size * 0.5 };
        let upem = f.units_per_em() as f32;
        s.chars()
            .map(|c| f.glyph_index(c).and_then(|g| f.glyph_hor_advance(g)).unwrap_or((upem * 0.5) as u16) as f32)
            .sum::<f32>()
            * size
            / upem
    }
}

struct Faces {
    regular: Face,
    bold: Face,
    italic: Face,
    bold_italic: Face,
    mono: Face,
}

fn faces() -> Result<&'static Faces> {
    static F: OnceLock<Result<Faces, String>> = OnceLock::new();
    F.get_or_init(|| {
        let r = || -> Result<Faces> {
            let regular = Face::load(&["segoeui.ttf", "arial.ttf"])?;
            Ok(Faces {
                bold: Face::load(&["segoeuib.ttf", "arialbd.ttf"]).unwrap_or_else(|_| regular.clone()),
                italic: Face::load(&["segoeuii.ttf", "ariali.ttf"]).unwrap_or_else(|_| regular.clone()),
                bold_italic: Face::load(&["segoeuiz.ttf", "arialbi.ttf"]).unwrap_or_else(|_| regular.clone()),
                mono: Face::load(&["consola.ttf", "cour.ttf"]).unwrap_or_else(|_| regular.clone()),
                regular,
            })
        };
        r().map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(|e| anyhow!("{e}"))
}

fn finish(doc: Document) -> Result<Vec<u8>> {
    doc.finish().map_err(|e| anyhow!("couldn't write the PDF: {e:?}"))
}

fn png_image(im: &DynamicImage) -> Result<Image> {
    let bytes = crate::imaging::png_bytes(im)?;
    Image::from_png(bytes.into(), true).map_err(|e| anyhow!("{e}"))
}

fn jpeg_or_png(im: &DynamicImage) -> Result<Image> {
    // Photos as JPEG keep the PDF small; anything with transparency stays PNG.
    if im.color().has_alpha() {
        return png_image(im);
    }
    let mut v = Vec::new();
    crate::imgio::encode(im, "JPG", 90, &mut v)?;
    Image::from_jpeg(v.into(), true).map_err(|e| anyhow!("{e}"))
}

/// One page per image, each page the image's size at 150 dpi.
pub fn images_pdf(pages: &[DynamicImage]) -> Result<Vec<u8>> {
    let mut doc = Document::new();
    for im in pages {
        let (w, h) = (im.width() as f32 * 72.0 / IMAGE_DPI, im.height() as f32 * 72.0 / IMAGE_DPI);
        let mut page = doc.start_page_with(PageSettings::from_wh(w.max(1.0), h.max(1.0)).context("bad page size")?);
        let mut s = page.surface();
        s.draw_image(jpeg_or_png(im)?, Size::from_wh(w.max(1.0), h.max(1.0)).context("bad image size")?);
        s.finish();
        page.finish();
    }
    finish(doc)
}

// ---------------------------------------------------------------- flow layout

struct Writer {
    doc: Document,
    lines: Vec<Vec<(Point, Face, f32, String, bool)>>,
    rules: Vec<Vec<(f32, f32, f32, f32)>>,
    y: f32,
}

impl Writer {
    fn new() -> Self {
        Writer { doc: Document::new(), lines: vec![vec![]], rules: vec![vec![]], y: MARGIN }
    }

    fn need(&mut self, h: f32) {
        if self.y + h > LETTER.1 - MARGIN && self.y > MARGIN {
            self.lines.push(vec![]);
            self.rules.push(vec![]);
            self.y = MARGIN;
        }
    }

    fn text(&mut self, x: f32, baseline: f32, face: &Face, size: f32, s: &str, underline: bool) {
        self.lines.last_mut().unwrap().push((Point::from_xy(x, baseline), face.clone(), size, s.to_string(), underline));
    }

    fn rule(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.rules.last_mut().unwrap().push((x0, y0, x1, y1));
    }

    fn finish(mut self) -> Result<Vec<u8>> {
        for (texts, rules) in self.lines.iter().zip(&self.rules) {
            let mut page = self.doc.start_page_with(PageSettings::from_wh(LETTER.0, LETTER.1).unwrap());
            let mut s = page.surface();
            s.set_fill(Some(Fill::default()));
            for (p, face, size, t, ul) in texts {
                s.draw_text(*p, face.font.clone(), *size, t, false, TextDirection::Auto);
                if *ul {
                    let w = face.width(t, *size);
                    let mut pb = PathBuilder::new();
                    pb.move_to(p.x, p.y + size * 0.12);
                    pb.line_to(p.x + w, p.y + size * 0.12);
                    if let Some(path) = pb.finish() {
                        s.set_stroke(Some(Stroke { width: size / 16.0, ..Default::default() }));
                        s.draw_path(&path);
                        s.set_stroke(None);
                    }
                }
            }
            if !rules.is_empty() {
                s.set_fill(None);
                s.set_stroke(Some(Stroke { width: 0.6, ..Default::default() }));
                for (x0, y0, x1, y1) in rules {
                    let mut pb = PathBuilder::new();
                    pb.move_to(*x0, *y0);
                    pb.line_to(*x1, *y1);
                    if let Some(path) = pb.finish() {
                        s.draw_path(&path);
                    }
                }
            }
            s.finish();
            page.finish();
        }
        finish(self.doc)
    }
}

/// A piece of a line: text in one face.
struct Piece {
    text: String,
    face: Face,
    underline: bool,
}

/// Break runs into lines no wider than `width`. Words never split unless one alone is too wide.
fn wrap(runs: &[(String, Face, bool)], size: f32, width: f32) -> Vec<Vec<Piece>> {
    let mut lines: Vec<Vec<Piece>> = vec![vec![]];
    let mut x = 0.0;
    for (text, face, ul) in runs {
        for (i, raw_line) in text.split('\n').enumerate() {
            if i > 0 {
                lines.push(vec![]);
                x = 0.0;
            }
            for word in split_keep_spaces(raw_line) {
                let mut w = face.width(&word, size);
                if x + w > width && x > 0.0 && !word.trim().is_empty() {
                    lines.push(vec![]);
                    x = 0.0;
                }
                let word = if x == 0.0 { word.trim_start().to_string() } else { word };
                if word.is_empty() {
                    continue;
                }
                w = face.width(&word, size);
                // a single word wider than the line: hard-break by characters
                if w > width {
                    let mut chunk = String::new();
                    for c in word.chars() {
                        if face.width(&format!("{chunk}{c}"), size) > width - x && !chunk.is_empty() {
                            lines.last_mut().unwrap().push(Piece { text: std::mem::take(&mut chunk), face: face.clone(), underline: *ul });
                            lines.push(vec![]);
                            x = 0.0;
                        }
                        chunk.push(c);
                    }
                    x += face.width(&chunk, size);
                    lines.last_mut().unwrap().push(Piece { text: chunk, face: face.clone(), underline: *ul });
                    continue;
                }
                x += w;
                let line = lines.last_mut().unwrap();
                match line.last_mut() {
                    Some(p) if p.face.data.as_ptr() == face.data.as_ptr() && p.underline == *ul => p.text.push_str(&word),
                    _ => line.push(Piece { text: word, face: face.clone(), underline: *ul }),
                }
            }
        }
    }
    lines
}

/// "a b  c" -> ["a", " b", "  c"]: words with their leading spaces.
fn split_keep_spaces(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut in_word = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if in_word {
                out.push(std::mem::take(&mut cur));
                in_word = false;
            }
            cur.push(c);
        } else {
            in_word = true;
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn face_for<'a>(f: &'a Faces, r: &Run) -> &'a Face {
    match (r.bold, r.italic) {
        (true, true) => &f.bold_italic,
        (true, false) => &f.bold,
        (false, true) => &f.italic,
        _ => &f.regular,
    }
}

fn paragraph(w: &mut Writer, pieces: &[(String, Face, bool)], size: f32, indent: f32, after: f32) {
    let line_h = size * 1.35;
    for line in wrap(pieces, size, LETTER.0 - 2.0 * MARGIN - indent) {
        w.need(line_h);
        let baseline = w.y + size;
        let mut x = MARGIN + indent;
        for p in &line {
            w.text(x, baseline, &p.face, size, &p.text, p.underline);
            x += p.face.width(&p.text, size);
        }
        w.y += line_h;
    }
    w.y += after;
}

/// Lay out a Word document's blocks on Letter pages.
pub fn blocks_pdf(blocks: &[Block]) -> Result<Vec<u8>> {
    let f = faces()?;
    let mut w = Writer::new();
    for b in blocks {
        match b {
            Block::Para { runs, style } => {
                let (size, bold_all, indent, bullet) = match style {
                    Style::Title => (24.0, true, 0.0, false),
                    Style::Heading(1) => (20.0, true, 0.0, false),
                    Style::Heading(2) => (16.0, true, 0.0, false),
                    Style::Heading(_) => (13.0, true, 0.0, false),
                    Style::List => (11.0, false, 14.0, true),
                    Style::Normal => (11.0, false, 0.0, false),
                };
                let mut pieces: Vec<(String, Face, bool)> = runs
                    .iter()
                    .filter(|r| !r.text.is_empty())
                    .map(|r| {
                        let r2 = Run { bold: r.bold || bold_all, ..r.clone() };
                        (r.text.clone(), face_for(f, &r2).clone(), r.underline)
                    })
                    .collect();
                if bullet {
                    pieces.insert(0, ("• ".into(), f.regular.clone(), false));
                }
                if pieces.is_empty() {
                    w.need(size * 1.35);
                    w.y += size * 1.35;
                    continue;
                }
                if matches!(style, Style::Heading(_) | Style::Title) {
                    w.y += size * 0.4;
                }
                paragraph(&mut w, &pieces, size, indent, 6.0);
            }
            Block::Table(rows) => table(&mut w, f, rows),
        }
    }
    w.finish()
}

fn table(w: &mut Writer, f: &Faces, rows: &[Vec<String>]) {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if cols == 0 {
        return;
    }
    let size = 10.0;
    let pad = 4.0;
    let total = LETTER.0 - 2.0 * MARGIN;
    let cw = total / cols as f32;
    w.y += 6.0;
    for row in rows {
        let cells: Vec<Vec<Vec<Piece>>> = (0..cols)
            .map(|c| wrap(&[(row.get(c).cloned().unwrap_or_default(), f.regular.clone(), false)], size, cw - 2.0 * pad))
            .collect();
        let n_lines = cells.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let h = n_lines as f32 * size * 1.3 + 2.0 * pad;
        w.need(h);
        let top = w.y;
        for (c, lines) in cells.iter().enumerate() {
            let x = MARGIN + c as f32 * cw;
            for (i, line) in lines.iter().enumerate() {
                let mut lx = x + pad;
                for p in line {
                    w.text(lx, top + pad + size + i as f32 * size * 1.3, &p.face, size, &p.text, false);
                    lx += p.face.width(&p.text, size);
                }
            }
        }
        for c in 0..=cols {
            let x = MARGIN + c as f32 * cw;
            w.rule(x, top, x, top + h);
        }
        w.rule(MARGIN, top, MARGIN + total, top);
        w.rule(MARGIN, top + h, MARGIN + total, top + h);
        w.y += h;
    }
    w.y += 6.0;
}

/// Plain text, monospaced (like <pre>), wrapped to the page.
pub fn text_pdf(text: &str) -> Result<Vec<u8>> {
    let f = faces()?;
    let mut w = Writer::new();
    let size = 9.5;
    let line_h = size * 1.35;
    let width = LETTER.0 - 2.0 * MARGIN;
    for raw in text.replace("\r\n", "\n").replace('\t', "    ").split('\n') {
        if raw.is_empty() {
            w.need(line_h);
            w.y += line_h;
            continue;
        }
        for line in wrap(&[(raw.to_string(), f.mono.clone(), false)], size, width) {
            w.need(line_h);
            let baseline = w.y + size;
            let mut x = MARGIN;
            for p in &line {
                w.text(x, baseline, &p.face, size, &p.text, false);
                x += p.face.width(&p.text, size);
            }
            w.y += line_h;
        }
    }
    w.finish()
}

/// A text stamp rendered as an RGBA image (for watermarks): `size` points at `scale`.
pub fn text_stamp_image(text: &str, size: f32, rgba: [u8; 4]) -> Result<image::RgbaImage> {
    use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont};
    let face = &faces()?.bold;
    let font = FontRef::try_from_slice(&face.data).map_err(|e| anyhow!("{e}"))?;
    let scale = PxScale::from(size * 4.0);
    let sf = font.as_scaled(scale);
    let width: f32 = text.chars().map(|c| sf.h_advance(sf.glyph_id(c))).sum();
    let (w, h) = ((width.ceil() as u32 + 8).max(1), ((sf.ascent() - sf.descent()).ceil() as u32 + 8).max(1));
    let mut img = image::RgbaImage::new(w, h);
    let mut x = 4.0;
    for c in text.chars() {
        let g = sf.glyph_id(c).with_scale_and_position(scale, ab_glyph::point(x, 4.0 + sf.ascent()));
        x += sf.h_advance(sf.glyph_id(c));
        if let Some(o) = font.outline_glyph(g) {
            let b = o.px_bounds();
            o.draw(|gx, gy, cov| {
                let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                    let p = img.get_pixel_mut(px as u32, py as u32);
                    let a = (cov * rgba[3] as f32) as u8;
                    if a > p.0[3] {
                        *p = image::Rgba([rgba[0], rgba[1], rgba[2], a]);
                    }
                }
            });
        }
    }
    Ok(img)
}

#[allow(dead_code)]
fn _transform_unused() -> Transform {
    Transform::from_translate(0.0, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docx::{Block, Run, Style};

    fn text_of(pdf: &[u8]) -> String {
        let d = std::env::temp_dir().join(format!("utsuri-pdfgen-{}-{}.pdf", std::process::id(), pdf.len()));
        std::fs::write(&d, pdf).unwrap();
        let t = crate::pdf::extract_text(&d).unwrap();
        let _ = std::fs::remove_file(&d);
        t
    }

    #[test]
    fn text_pdf_keeps_vietnamese_and_paginates() {
        let mut s = String::from("Xin chào thế giới — Tiếng Việt có dấu\n");
        for i in 0..120 {
            s.push_str(&format!("line {i} with some words to fill the page\n"));
        }
        let pdf = text_pdf(&s).unwrap();
        let doc = lopdf::Document::load_mem(&pdf).unwrap();
        assert!(doc.get_pages().len() >= 2);
        let t = text_of(&pdf);
        assert!(t.contains("Xin chào thế giới"), "{}", &t[..t.len().min(200)]);
        assert!(t.contains("line 119"));
    }

    #[test]
    fn blocks_layout_headings_runs_tables() {
        let blocks = vec![
            Block::Para { style: Style::Heading(1), runs: vec![Run { text: "Report".into(), ..Default::default() }] },
            Block::Para {
                style: Style::Normal,
                runs: vec![
                    Run { text: "Plain then ".into(), ..Default::default() },
                    Run { text: "bold".into(), bold: true, ..Default::default() },
                    Run { text: " and ".into(), ..Default::default() },
                    Run { text: "underlined".into(), underline: true, ..Default::default() },
                ],
            },
            Block::Para { style: Style::List, runs: vec![Run { text: "a bullet".into(), ..Default::default() }] },
            Block::Table(vec![vec!["Name".into(), "Qty".into()], vec!["Apples".into(), "3".into()]]),
        ];
        let pdf = blocks_pdf(&blocks).unwrap();
        let t = text_of(&pdf);
        for want in ["Report", "bold", "underlined", "a bullet", "Apples"] {
            assert!(t.contains(want), "missing {want}: {t}");
        }
    }

    #[test]
    fn stamp_image_has_ink() {
        let im = text_stamp_image("DRAFT", 40.0, [200, 0, 0, 255]).unwrap();
        assert!(im.width() > im.height());
        assert!(im.pixels().any(|p| p.0[3] > 200));
    }
}
