//! Image tools: compress, resize, metadata, redact, edit, add background.
//! Ported from imaging.py.

use crate::imgio::{self, encode, open_image, save_image, target_of_ext};
use crate::out::{atomic_write, sibling};
use crate::refuse;
use anyhow::Result;
use image::{DynamicImage, GrayImage, Rgba, RgbaImage, imageops::FilterType};
use std::path::{Path, PathBuf};

/// Save `im` next to `src` as "<name> <label>.<ext>" in the source's format.
pub fn save_like(im: &DynamicImage, src: &Path, label: &str, force_ext: Option<&str>) -> Result<PathBuf> {
    let mut ext = force_ext.map(str::to_string).unwrap_or_else(|| crate::kinds::ext(src));
    if ext == "gif" || ext == "ico" {
        ext = "png".into();
    }
    let out = sibling(src, label, Some(&ext));
    save_image(im, &out, target_of_ext(&ext), None, None)?;
    Ok(out)
}

fn thumbnail(im: &DynamicImage, max_side: u32) -> DynamicImage {
    if im.width().max(im.height()) > max_side { im.resize(max_side, max_side, FilterType::Lanczos3) } else { im.clone() }
}

// ---------------------------------------------------------------- PNG quantising

/// Lossy PNG: quantise to a palette (like pngquant), keeping transparency.
pub fn write_quantized_png(im: &DynamicImage, quality: u8, w: &mut Vec<u8>) -> Result<()> {
    let colors = ((quality as f32 * 2.56) as usize).clamp(16, 256);
    let rgba = im.to_rgba8();
    let (iw, ih) = rgba.dimensions();
    let nq = color_quant::NeuQuant::new(10, colors, rgba.as_raw());
    let palette = nq.color_map_rgba();
    // Floyd–Steinberg dithering over RGBA
    let mut err = vec![[0f32; 4]; (iw as usize + 2) * 2];
    let row = iw as usize + 2;
    let mut idx = Vec::with_capacity((iw * ih) as usize);
    for y in 0..ih as usize {
        let (cur, next) = if y % 2 == 0 { (0, row) } else { (row, 0) };
        for e in &mut err[next..next + row] {
            *e = [0.0; 4];
        }
        for x in 0..iw as usize {
            let p = rgba.get_pixel(x as u32, y as u32).0;
            let e = err[cur + x + 1];
            let want = [0, 1, 2, 3].map(|c| (p[c] as f32 + e[c]).clamp(0.0, 255.0));
            let px = [want[0] as u8, want[1] as u8, want[2] as u8, want[3] as u8];
            let i = nq.index_of(&px);
            idx.push(i as u8);
            let got = &palette[i * 4..i * 4 + 4];
            for c in 0..4 {
                let d = want[c] - got[c] as f32;
                err[cur + x + 2][c] += d * 7.0 / 16.0;
                err[next + x][c] += d * 3.0 / 16.0;
                err[next + x + 1][c] += d * 5.0 / 16.0;
                err[next + x + 2][c] += d / 16.0;
            }
        }
    }
    let mut enc = png::Encoder::new(&mut *w, iw, ih);
    enc.set_color(png::ColorType::Indexed);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Best);
    let n = palette.len() / 4;
    enc.set_palette((0..n).flat_map(|i| palette[i * 4..i * 4 + 3].to_vec()).collect::<Vec<_>>());
    let trns: Vec<u8> = (0..n).map(|i| palette[i * 4 + 3]).collect();
    if trns.iter().any(|a| *a < 255) {
        enc.set_trns(trns);
    }
    let mut wr = enc.write_header()?;
    wr.write_image_data(&idx)?;
    wr.finish()?;
    Ok(())
}

// ---------------------------------------------------------------- compress

/// Format a compressed copy is written in (BMP/TIFF/GIF/ICO become JPG).
pub fn lossy_target(src: &Path, to_webp: bool) -> &'static str {
    if to_webp {
        return "WEBP";
    }
    match target_of_ext(&crate::kinds::ext(src)) {
        t @ ("JPG" | "PNG" | "WEBP" | "AVIF" | "HEIC") => t,
        _ => "JPG",
    }
}

fn write_compressed(im: &DynamicImage, out: &Path, target: &str, quality: u8) -> Result<()> {
    if target == "PNG" {
        let mut v = Vec::new();
        write_quantized_png(im, quality, &mut v)?;
        atomic_write(out, |tmp| std::fs::write(tmp, &v))?;
        Ok(())
    } else {
        save_image(im, out, target, Some(quality), None)
    }
}

/// Compress by quality, optionally capping the long side and switching to WebP.
pub fn compress_image(src: &Path, quality: u8, max_side: Option<u32>, to_webp: bool) -> Result<PathBuf> {
    let im = open_image(src)?;
    let im = max_side.map_or(im.clone(), |s| thumbnail(&im, s));
    let target = lossy_target(src, to_webp);
    let out = sibling(src, "Compressed", Some(imgio::ext_of_target(target)));
    write_compressed(&im, &out, target, quality)?;
    Ok(out)
}

/// Find (quality, max_side) that fits under `max_bytes`: binary-search quality, then shrink.
pub fn plan_target_size(im: &DynamicImage, target: &str, max_bytes: usize) -> Result<(u8, Option<u32>)> {
    let mut side = None;
    let mut work = im.clone();
    for _ in 0..8 {
        let (mut lo, mut hi, mut best) = (30i32, 92i32, None);
        while lo <= hi {
            let q = (lo + hi) / 2;
            if imgio::encoded_size(&work, target, q as u8)? <= max_bytes {
                best = Some(q as u8);
                lo = q + 1;
            } else {
                hi = q - 1;
            }
        }
        if let Some(b) = best {
            return Ok((b, side));
        }
        let s = (work.width().max(work.height()) as f32 * 0.8) as u32;
        side = Some(s);
        work = thumbnail(im, s);
        if s < 200 {
            return Ok((30, side));
        }
    }
    Ok((30, side))
}

pub fn compress_to(src: &Path, quality: u8, max_side: Option<u32>, target: &str) -> Result<PathBuf> {
    let im = open_image(src)?;
    let im = max_side.map_or(im.clone(), |s| thumbnail(&im, s));
    let out = sibling(src, "Compressed", Some(imgio::ext_of_target(target)));
    write_compressed(&im, &out, target, quality)?;
    Ok(out)
}

/// Compress to fit under `max_bytes`.
pub fn compress_under(src: &Path, max_bytes: usize, to_webp: bool) -> Result<PathBuf> {
    let im = open_image(src)?;
    let target = lossy_target(src, to_webp);
    let (q, side) = plan_target_size(&im, target, max_bytes)?;
    compress_to(src, q, side, target)
}

// ---------------------------------------------------------------- resize

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeMode {
    Percent,
    LongSide,
    Width,
    Height,
}

pub fn resize_dims((w, h): (u32, u32), mode: ResizeMode, value: u32) -> (u32, u32) {
    let k = match mode {
        ResizeMode::Percent => value as f64 / 100.0,
        ResizeMode::LongSide => value as f64 / w.max(h) as f64,
        ResizeMode::Width => value as f64 / w as f64,
        ResizeMode::Height => value as f64 / h as f64,
    };
    (((w as f64 * k).round() as u32).max(1), ((h as f64 * k).round() as u32).max(1))
}

pub fn resize_image(src: &Path, dims: (u32, u32)) -> Result<PathBuf> {
    let im = open_image(src)?;
    let r = im.resize_exact(dims.0, dims.1, FilterType::Lanczos3);
    save_like(&r, src, &format!("{}x{}", dims.0, dims.1), None)
}

pub fn resize_by(src: &Path, mode: ResizeMode, value: u32) -> Result<PathBuf> {
    let (w, h) = image::image_dimensions(src).or_else(|_| open_image(src).map(|i| (i.width(), i.height())))?;
    resize_image(src, resize_dims((w, h), mode, value))
}

// ---------------------------------------------------------------- metadata

fn read_exif(src: &Path) -> Option<exif::Exif> {
    let f = std::fs::File::open(src).ok()?;
    exif::Reader::new().read_from_container(&mut std::io::BufReader::new(f)).ok()
}

/// Every field, for the "all fields" list.
pub fn read_image_metadata(src: &Path) -> Vec<(String, String)> {
    let mut out = vec![];
    if let Ok((w, h)) = image::image_dimensions(src) {
        out.push(("Dimensions".into(), format!("{w} × {h}")));
    }
    out.push(("Format".into(), imgio::target_of_ext(&crate::kinds::ext(src)).to_string()));
    if let Some(ex) = read_exif(src) {
        for f in ex.fields() {
            let name = if f.ifd_num == exif::In::PRIMARY && f.tag.context() == exif::Context::Gps { format!("GPS {}", f.tag) } else { f.tag.to_string() };
            let mut v = f.display_value().with_unit(&ex).to_string();
            if v.chars().count() > 120 {
                v = format!("{}…", v.chars().take(117).collect::<String>());
            }
            out.push((name, v));
        }
    }
    out
}

/// GPS position in decimal degrees.
pub fn gps_decimal(ex: &exif::Exif) -> Option<(f64, f64)> {
    let conv = |tag, rtag| -> Option<f64> {
        let f = ex.get_field(tag, exif::In::PRIMARY)?;
        let exif::Value::Rational(v) = &f.value else { return None };
        if v.len() < 3 {
            return None;
        }
        let val = v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0;
        let r = ex.get_field(rtag, exif::In::PRIMARY).map(|f| f.display_value().to_string()).unwrap_or_default();
        Some(if r.contains('S') || r.contains('W') { -val } else { val })
    };
    Some((conv(exif::Tag::GPSLatitude, exif::Tag::GPSLatitudeRef)?, conv(exif::Tag::GPSLongitude, exif::Tag::GPSLongitudeRef)?))
}

/// The handful of fields people care about.
pub fn metadata_summary(src: &Path) -> Vec<(String, String)> {
    let mut out = vec![];
    if let Ok((w, h)) = image::image_dimensions(src) {
        out.push(("Size".into(), format!("{w} × {h}")));
    }
    let Some(ex) = read_exif(src) else { return out };
    let get = |t| ex.get_field(t, exif::In::PRIMARY).map(|f| f.display_value().to_string().trim_matches('"').trim().to_string()).filter(|s| !s.is_empty());
    let (make, model) = (get(exif::Tag::Make).unwrap_or_default(), get(exif::Tag::Model).unwrap_or_default());
    if !make.is_empty() || !model.is_empty() {
        out.push(("Camera".into(), if !make.is_empty() && model.starts_with(&make) { model } else { format!("{make} {model}").trim().to_string() }));
    }
    if let Some(t) = get(exif::Tag::DateTimeOriginal).or_else(|| get(exif::Tag::DateTime)) {
        out.push(("Taken".into(), t));
    }
    if let Some(s) = get(exif::Tag::Software) {
        out.push(("Software".into(), s));
    }
    if let Some(f) = ex.get_field(exif::Tag::ExposureTime, exif::In::PRIMARY) {
        if let exif::Value::Rational(v) = &f.value {
            let t = v[0].to_f64();
            out.push(("Exposure".into(), if t > 0.0 && t < 1.0 { format!("1/{} s", (1.0 / t).round()) } else { format!("{t} s") }));
        }
    }
    if let Some(f) = ex.get_field(exif::Tag::FNumber, exif::In::PRIMARY) {
        if let exif::Value::Rational(v) = &f.value {
            out.push(("Aperture".into(), format!("f/{:.1}", v[0].to_f64())));
        }
    }
    if let Some(i) = get(exif::Tag::PhotographicSensitivity) {
        out.push(("ISO".into(), i));
    }
    if let Some((la, lo)) = gps_decimal(&ex) {
        out.push(("Location".into(), format!("{la:.5}, {lo:.5}")));
    }
    out
}

/// (number of metadata entries, has GPS).
pub fn count_metadata(src: &Path) -> (usize, bool) {
    match read_exif(src) {
        Some(ex) => (ex.fields().count(), ex.fields().any(|f| f.tag.context() == exif::Context::Gps)),
        None => (0, false),
    }
}

/// EXIF bytes with these fields (Orientation set to 1: the pixels are already upright).
fn exif_bytes(fields: &[exif::Field]) -> Result<Vec<u8>> {
    let mut w = exif::experimental::Writer::new();
    for f in fields {
        w.push_field(f);
    }
    let mut c = std::io::Cursor::new(Vec::new());
    w.write(&mut c, false)?;
    Ok(c.into_inner())
}

/// Remove only GPS, keep the camera info and the rest.
pub fn strip_location(src: &Path) -> Result<PathBuf> {
    let ex = read_exif(src);
    let im = open_image(src)?;
    let e = crate::kinds::ext(src);
    let ext = if ["jpg", "jpeg", "png", "webp", "tif", "tiff"].contains(&e.as_str()) { e } else { "jpg".into() };
    let out = sibling(src, "No Location", Some(&ext));
    let kept: Vec<exif::Field> = ex
        .map(|ex| {
            ex.fields()
                .filter(|f| f.ifd_num == exif::In::PRIMARY && f.tag.context() != exif::Context::Gps && f.tag != exif::Tag::GPSInfoIFDPointer)
                .filter(|f| !matches!(f.tag, exif::Tag::ExifIFDPointer | exif::Tag::InteropIFDPointer | exif::Tag::JPEGInterchangeFormat | exif::Tag::JPEGInterchangeFormatLength))
                .map(|f| if f.tag == exif::Tag::Orientation { exif::Field { tag: f.tag, ifd_num: f.ifd_num, value: exif::Value::Short(vec![1]) } } else { f.clone() })
                .collect()
        })
        .unwrap_or_default();
    let blob = if kept.is_empty() { None } else { exif_bytes(&kept).ok() };
    save_image(&im, &out, target_of_ext(&ext), None, blob.as_deref())?;
    Ok(out)
}

/// Re-encode without any metadata (rotation already baked in).
pub fn strip_image_metadata(src: &Path) -> Result<PathBuf> {
    let im = open_image(src)?;
    save_like(&im, src, "Clean", None)
}

// ---------------------------------------------------------------- redact

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedactMode {
    Black,
    Pixelate,
    Blur,
}

/// Rects are (x0, y0, x1, y1) in image pixels, any corner order.
pub fn redact(im: &DynamicImage, rects: &[(i64, i64, i64, i64)], mode: RedactMode) -> DynamicImage {
    let mut out = im.to_rgba8();
    let (w, h) = (out.width() as i64, out.height() as i64);
    for &(x0, y0, x1, y1) in rects {
        let (l, t) = (x0.min(x1).max(0), y0.min(y1).max(0));
        let (r, b) = (x0.max(x1).min(w), y0.max(y1).min(h));
        if r - l < 1 || b - t < 1 {
            continue;
        }
        let (bw, bh) = ((r - l) as u32, (b - t) as u32);
        match mode {
            RedactMode::Black => {
                for y in t..b {
                    for x in l..r {
                        out.put_pixel(x as u32, y as u32, Rgba([0, 0, 0, 255]));
                    }
                }
            }
            RedactMode::Pixelate | RedactMode::Blur => {
                let region = image::imageops::crop_imm(&out, l as u32, t as u32, bw, bh).to_image();
                let done = if mode == RedactMode::Pixelate {
                    let f = (bw.min(bh) / 6).max(8);
                    let small = image::imageops::resize(&region, (bw / f).max(1), (bh / f).max(1), FilterType::Triangle);
                    image::imageops::resize(&small, bw, bh, FilterType::Nearest)
                } else {
                    // strong enough that text can't be recovered
                    image::imageops::blur(&region, (bw.min(bh) as f32 / 3.0).max(12.0))
                };
                image::imageops::replace(&mut out, &done, l, t);
            }
        }
    }
    if im.color().has_alpha() { DynamicImage::ImageRgba8(out) } else { DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(out).to_rgb8()) }
}

// ---------------------------------------------------------------- edit

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adjust {
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub sharpness: f32,
    /// -1 … 1
    pub warmth: f32,
    /// multiples of 90, clockwise
    pub rotate: i32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub grayscale: bool,
    pub resize_pct: u32,
}

impl Default for Adjust {
    fn default() -> Self {
        Self { brightness: 1.0, contrast: 1.0, saturation: 1.0, sharpness: 1.0, warmth: 0.0, rotate: 0, flip_h: false, flip_v: false, grayscale: false, resize_pct: 100 }
    }
}

fn luma(p: [u8; 3]) -> f32 {
    (p[0] as f32 * 299.0 + p[1] as f32 * 587.0 + p[2] as f32 * 114.0) / 1000.0
}

/// Gentle adjustments from the histogram (like a phone's "auto" button).
pub fn auto_enhance(im: &DynamicImage) -> Adjust {
    let rgb = thumbnail(im, 512).to_rgb8();
    let n = (rgb.width() * rgb.height()).max(1) as f32;
    let ls: Vec<f32> = rgb.pixels().map(|p| luma(p.0)).collect();
    let mean = ls.iter().sum::<f32>() / n;
    let std = (ls.iter().map(|l| (l - mean).powi(2)).sum::<f32>() / n).sqrt();
    let sat = rgb
        .pixels()
        .map(|p| {
            let mx = *p.0.iter().max().unwrap() as f32;
            let mn = *p.0.iter().min().unwrap() as f32;
            if mx > 0.0 { (mx - mn) / mx * 255.0 } else { 0.0 }
        })
        .sum::<f32>()
        / n;
    Adjust {
        brightness: (1.0 + (118.0 - mean) / 400.0).clamp(0.85, 1.25),
        contrast: (1.0 + (58.0 - std) / 160.0).clamp(0.95, 1.3),
        saturation: (1.0 + (110.0 - sat) / 400.0).clamp(1.0, 1.3),
        sharpness: 1.15,
        ..Default::default()
    }
}

/// PIL's ImageEnhance: blend between a "degenerate" image and the original.
pub fn apply_adjust(im: &DynamicImage, a: &Adjust) -> DynamicImage {
    let alpha = im.color().has_alpha().then(|| im.to_rgba8());
    let mut rgb = im.to_rgb8();
    let (w, h) = rgb.dimensions();
    let blend = |d: f32, o: f32, f: f32| (d + (o - d) * f).clamp(0.0, 255.0);
    if a.brightness != 1.0 {
        for p in rgb.pixels_mut() {
            p.0 = p.0.map(|c| blend(0.0, c as f32, a.brightness) as u8);
        }
    }
    if a.contrast != 1.0 {
        let mean = rgb.pixels().map(|p| luma(p.0)).sum::<f32>() / (w * h).max(1) as f32;
        let m = mean.round();
        for p in rgb.pixels_mut() {
            p.0 = p.0.map(|c| blend(m, c as f32, a.contrast) as u8);
        }
    }
    if a.saturation != 1.0 {
        for p in rgb.pixels_mut() {
            let l = luma(p.0);
            p.0 = p.0.map(|c| blend(l, c as f32, a.saturation) as u8);
        }
    }
    if a.sharpness != 1.0 {
        let src = rgb.clone();
        // PIL's SMOOTH kernel: centre 5, neighbours 1, /13
        for y in 1..h.saturating_sub(1) {
            for x in 1..w.saturating_sub(1) {
                let mut s = [0f32; 3];
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let k = if dx == 0 && dy == 0 { 5.0 } else { 1.0 };
                        let q = src.get_pixel((x as i32 + dx) as u32, (y as i32 + dy) as u32).0;
                        for c in 0..3 {
                            s[c] += q[c] as f32 * k;
                        }
                    }
                }
                let o = src.get_pixel(x, y).0;
                let p = rgb.get_pixel_mut(x, y);
                for c in 0..3 {
                    p.0[c] = blend(s[c] / 13.0, o[c] as f32, a.sharpness) as u8;
                }
            }
        }
    }
    if a.warmth != 0.0 {
        let k = a.warmth * 25.0;
        for p in rgb.pixels_mut() {
            p.0[0] = (p.0[0] as f32 + k).clamp(0.0, 255.0) as u8;
            p.0[2] = (p.0[2] as f32 - k).clamp(0.0, 255.0) as u8;
        }
    }
    if a.grayscale {
        for p in rgb.pixels_mut() {
            let l = luma(p.0) as u8;
            p.0 = [l, l, l];
        }
    }
    let mut out = match alpha {
        Some(al) => {
            let mut o = RgbaImage::new(w, h);
            for (x, y, p) in o.enumerate_pixels_mut() {
                let c = rgb.get_pixel(x, y).0;
                *p = Rgba([c[0], c[1], c[2], al.get_pixel(x, y).0[3]]);
            }
            DynamicImage::ImageRgba8(o)
        }
        None => DynamicImage::ImageRgb8(rgb),
    };
    if a.flip_h {
        out = out.fliph();
    }
    if a.flip_v {
        out = out.flipv();
    }
    out = match a.rotate.rem_euclid(360) {
        90 => out.rotate90(),
        180 => out.rotate180(),
        270 => out.rotate270(),
        _ => out,
    };
    if a.resize_pct != 100 {
        let nw = ((out.width() as f32 * a.resize_pct as f32 / 100.0).round() as u32).max(1);
        let nh = ((out.height() as f32 * a.resize_pct as f32 / 100.0).round() as u32).max(1);
        out = out.resize_exact(nw, nh, FilterType::Lanczos3);
    }
    out
}

// ---------------------------------------------------------------- add background

#[derive(Clone, Debug, PartialEq)]
pub struct Background {
    pub colors: Vec<[u8; 3]>,
    /// degrees, CSS-style (0 = towards the top, 90 = towards the right)
    pub angle: f32,
    /// fraction of the longer image side
    pub padding: f32,
    /// px on the source image scale
    pub radius: u32,
    /// 0 … 1
    pub shadow: f32,
    /// "Auto", "1:1", "4:3", "16:9", "9:16", "4:5", "3:2"
    pub aspect: String,
}

impl Default for Background {
    fn default() -> Self {
        Self { colors: vec![[0xff, 0x9a, 0x5a], [0xff, 0x3d, 0x6e], [0x7b, 0x5c, 0xff]], angle: 135.0, padding: 0.12, radius: 24, shadow: 0.5, aspect: "Auto".into() }
    }
}

pub const ASPECTS: &[(&str, f32)] = &[("1:1", 1.0), ("4:3", 4.0 / 3.0), ("16:9", 16.0 / 9.0), ("9:16", 9.0 / 16.0), ("4:5", 0.8), ("3:2", 1.5)];

pub fn gradient(w: u32, h: u32, colors: &[[u8; 3]], angle: f32) -> RgbaImage {
    if colors.len() == 1 {
        let c = colors[0];
        return RgbaImage::from_pixel(w, h, Rgba([c[0], c[1], c[2], 255]));
    }
    let a = angle.to_radians();
    let (dx, dy) = (a.sin(), -a.cos());
    let len = (w as f32 * dx.abs() + h as f32 * dy.abs()).max(1.0);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let n = (colors.len() - 1) as f32;
    RgbaImage::from_fn(w, h, |x, y| {
        let t = (((x as f32 + 0.5 - cx) * dx + (y as f32 + 0.5 - cy) * dy) / len + 0.5).clamp(0.0, 1.0) * n;
        let j = (t as usize).min(colors.len() - 2);
        let f = t - j as f32;
        let (p, q) = (colors[j], colors[j + 1]);
        Rgba([0, 1, 2].map(|k| (p[k] as f32 + (q[k] as f32 - p[k] as f32) * f).round() as u8).into_iter().chain([255]).collect::<Vec<_>>().try_into().unwrap())
    })
}

/// Anti-aliased rounded-rectangle coverage mask.
pub fn rounded_mask(w: u32, h: u32, radius: u32) -> GrayImage {
    let r = radius.min(w.min(h) / 2) as f32;
    GrayImage::from_fn(w, h, |x, y| {
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let qx = (px - w as f32 / 2.0).abs() - (w as f32 / 2.0 - r);
        let qy = (py - h as f32 / 2.0).abs() - (h as f32 / 2.0 - r);
        let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
        image::Luma([((0.5 - outside).clamp(0.0, 1.0) * 255.0) as u8])
    })
}

pub fn add_background(im: &DynamicImage, bg: &Background) -> RgbaImage {
    let src = im.to_rgba8();
    let (w, h) = src.dimensions();
    let pad = (w.max(h) as f32 * bg.padding) as u32;
    let (mut cw, mut ch) = (w + 2 * pad, h + 2 * pad);
    if let Some((_, r)) = ASPECTS.iter().find(|(n, _)| *n == bg.aspect) {
        if (cw as f32 / ch as f32) < *r {
            cw = (ch as f32 * r).round() as u32;
        } else {
            ch = (cw as f32 / r).round() as u32;
        }
    }
    let mut canvas = gradient(cw, ch, &bg.colors, bg.angle);
    let (x, y) = ((cw - w) / 2, (ch - h) / 2);
    let mut mask = rounded_mask(w, h, bg.radius);
    for (mx, my, m) in mask.enumerate_pixels_mut() {
        m.0[0] = ((m.0[0] as u32 * src.get_pixel(mx, my).0[3] as u32) / 255) as u8;
    }
    if bg.shadow > 0.0 {
        let blur = ((w.max(h) as f32 * 0.035) as u32).max(4);
        let mut smask = GrayImage::new(cw, ch);
        image::imageops::replace(&mut smask, &mask, x as i64, (y + (blur as f32 * 0.6) as u32) as i64);
        let smask = image::imageops::blur(&smask, blur as f32);
        for (px, py, p) in canvas.enumerate_pixels_mut() {
            let a = smask.get_pixel(px, py).0[0] as f32 / 255.0 * 0.65 * bg.shadow;
            for c in 0..3 {
                p.0[c] = (p.0[c] as f32 * (1.0 - a)) as u8;
            }
        }
    }
    for (sx, sy, s) in src.enumerate_pixels() {
        let a = mask.get_pixel(sx, sy).0[0] as f32 / 255.0;
        if a <= 0.0 {
            continue;
        }
        let p = canvas.get_pixel_mut(x + sx, y + sy);
        for c in 0..3 {
            p.0[c] = (s.0[c] as f32 * a + p.0[c] as f32 * (1.0 - a)).round() as u8;
        }
    }
    canvas
}

/// The image tools' one-call versions (used by the More ring and tests).
pub fn edit_and_save(src: &Path, a: &Adjust) -> Result<PathBuf> {
    let im = open_image(src)?;
    save_like(&apply_adjust(&im, a), src, "Edited", None)
}

pub fn add_background_and_save(src: &Path, bg: &Background) -> Result<PathBuf> {
    let im = open_image(src)?;
    let out = sibling(src, "BG", Some("png"));
    save_image(&DynamicImage::ImageRgba8(add_background(&im, bg)), &out, "PNG", None, None)?;
    Ok(out)
}

/// Crop rect (x, y, w, h) in image pixels, plus extra clockwise rotation.
pub fn crop_and_save(src: &Path, rect: (u32, u32, u32, u32), rotate: i32) -> Result<PathBuf> {
    let im = open_image(src)?;
    let (x, y, w, h) = rect;
    if w == 0 || h == 0 || x + w > im.width() || y + h > im.height() {
        refuse!("The crop area is outside the image");
    }
    let mut c = im.crop_imm(x, y, w, h);
    c = match rotate.rem_euclid(360) {
        90 => c.rotate90(),
        180 => c.rotate180(),
        270 => c.rotate270(),
        _ => c,
    };
    save_like(&c, src, "Cropped", None)
}

pub fn redact_and_save(src: &Path, rects: &[(i64, i64, i64, i64)], mode: RedactMode) -> Result<PathBuf> {
    if rects.is_empty() {
        refuse!("Draw at least one box over what to hide");
    }
    let im = open_image(src)?;
    save_like(&redact(&im, rects, mode), src, "Redacted", None)
}

/// Copy encoded PNG bytes of an image (for the clipboard).
pub fn png_bytes(im: &DynamicImage) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    encode(im, "PNG", 90, &mut v)?;
    Ok(v)
}

#[doc(hidden)]
/// Test helper: an EXIF block with an orientation and optional GPS (lat, lon).
pub fn tests_exif_with(orientation: u16, gps: Option<(f64, f64)>) -> Vec<u8> {
    use exif::{Field, In, Rational, Tag, Value};
    let mut fields = vec![
        Field { tag: Tag::Orientation, ifd_num: In::PRIMARY, value: Value::Short(vec![orientation]) },
        Field { tag: Tag::Make, ifd_num: In::PRIMARY, value: Value::Ascii(vec![b"TestCam".to_vec()]) },
        Field { tag: Tag::Model, ifd_num: In::PRIMARY, value: Value::Ascii(vec![b"TestCam X1".to_vec()]) },
    ];
    if let Some((la, lo)) = gps {
        let dms = |v: f64| {
            let v = v.abs();
            let d = v.floor();
            let m = ((v - d) * 60.0).floor();
            let s = ((v - d) * 60.0 - m) * 60.0;
            vec![Rational::from((d as u32, 1)), Rational::from((m as u32, 1)), Rational::from(((s * 1000.0) as u32, 1000))]
        };
        fields.push(Field { tag: Tag::GPSLatitudeRef, ifd_num: In::PRIMARY, value: Value::Ascii(vec![if la < 0.0 { b"S".to_vec() } else { b"N".to_vec() }]) });
        fields.push(Field { tag: Tag::GPSLatitude, ifd_num: In::PRIMARY, value: Value::Rational(dms(la)) });
        fields.push(Field { tag: Tag::GPSLongitudeRef, ifd_num: In::PRIMARY, value: Value::Ascii(vec![if lo < 0.0 { b"W".to_vec() } else { b"E".to_vec() }]) });
        fields.push(Field { tag: Tag::GPSLongitude, ifd_num: In::PRIMARY, value: Value::Rational(dms(lo)) });
    }
    exif_bytes(&fields).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{photo, tmpdir};

    fn jpg_with_gps(d: &Path) -> PathBuf {
        let src = d.join("photo.jpg");
        let exif = tests_exif_with(1, Some((48.85837, 2.29448)));
        save_image(&DynamicImage::ImageRgb8(photo(300, 200)), &src, "JPG", Some(95), Some(&exif)).unwrap();
        src
    }

    #[test]
    fn compress_shrinks_and_converts_odd_formats_to_jpg() {
        let d = tmpdir("compress");
        let src = d.join("p.jpg");
        save_image(&DynamicImage::ImageRgb8(photo(600, 400)), &src, "JPG", Some(98), None).unwrap();
        let out = compress_image(&src, 50, None, false).unwrap();
        assert_eq!(out.file_name().unwrap(), "p Compressed.jpg");
        assert!(out.metadata().unwrap().len() < src.metadata().unwrap().len());
        let bmp = d.join("b.bmp");
        DynamicImage::ImageRgb8(photo(50, 50)).save(&bmp).unwrap();
        assert_eq!(compress_image(&bmp, 70, None, false).unwrap().extension().unwrap(), "jpg");
        let png = d.join("t.png");
        photo(200, 200).save(&png).unwrap();
        let q = compress_image(&png, 60, Some(100), false).unwrap();
        let back = image::open(&q).unwrap();
        assert_eq!(back.width(), 100);
        assert!(q.metadata().unwrap().len() < png.metadata().unwrap().len());
        assert_eq!(compress_image(&png, 60, None, true).unwrap().extension().unwrap(), "webp");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn compress_under_a_size() {
        let d = tmpdir("under");
        let src = d.join("big.jpg");
        save_image(&DynamicImage::ImageRgb8(photo(1600, 1200)), &src, "JPG", Some(98), None).unwrap();
        let out = compress_under(&src, 100 * 1024, false).unwrap();
        assert!(out.metadata().unwrap().len() <= 100 * 1024, "{}", out.metadata().unwrap().len());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn resize_modes() {
        assert_eq!(resize_dims((4000, 3000), ResizeMode::Percent, 50), (2000, 1500));
        assert_eq!(resize_dims((4000, 3000), ResizeMode::LongSide, 1920), (1920, 1440));
        assert_eq!(resize_dims((4000, 3000), ResizeMode::Width, 800), (800, 600));
        assert_eq!(resize_dims((4000, 3000), ResizeMode::Height, 300), (400, 300));
        let d = tmpdir("resize");
        let src = d.join("r.png");
        photo(200, 100).save(&src).unwrap();
        let out = resize_by(&src, ResizeMode::Percent, 50).unwrap();
        assert_eq!(out.file_name().unwrap(), "r 100x50.png");
        assert_eq!(image::image_dimensions(&out).unwrap(), (100, 50));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn metadata_summary_location_and_stripping() {
        let d = tmpdir("meta");
        let src = jpg_with_gps(&d);
        let sum = metadata_summary(&src);
        let get = |k: &str| sum.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
        assert_eq!(get("Camera").as_deref(), Some("TestCam X1"));
        assert_eq!(get("Location").as_deref(), Some("48.85837, 2.29448"));
        let (n, gps) = count_metadata(&src);
        assert!(n >= 5 && gps);

        let noloc = strip_location(&src).unwrap();
        assert_eq!(noloc.file_name().unwrap(), "photo No Location.jpg");
        let (n2, gps2) = count_metadata(&noloc);
        assert!(!gps2 && n2 >= 2, "kept camera fields: {n2}");
        assert!(metadata_summary(&noloc).iter().any(|(k, v)| k == "Camera" && v == "TestCam X1"));

        let clean = strip_image_metadata(&src).unwrap();
        assert_eq!(count_metadata(&clean), (0, false));
        assert!(!read_image_metadata(&src).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn redact_modes_change_only_the_box() {
        let im = DynamicImage::ImageRgb8(photo(120, 80));
        for mode in [RedactMode::Black, RedactMode::Pixelate, RedactMode::Blur] {
            let r = redact(&im, &[(70, 50, 10, 10)], mode).to_rgb8();
            assert_eq!(r.get_pixel(100, 70), im.to_rgb8().get_pixel(100, 70), "{mode:?} outside");
            assert_ne!(r.get_pixel(40, 30), im.to_rgb8().get_pixel(40, 30), "{mode:?} inside");
            if mode == RedactMode::Black {
                assert_eq!(r.get_pixel(40, 30).0, [0, 0, 0]);
            }
        }
    }

    #[test]
    fn adjust_basics() {
        let im = DynamicImage::ImageRgb8(photo(40, 20));
        let a = Adjust { rotate: 90, ..Default::default() };
        assert_eq!(apply_adjust(&im, &a).dimensions_pair(), (20, 40));
        let g = apply_adjust(&im, &Adjust { grayscale: true, ..Default::default() }).to_rgb8();
        assert!(g.pixels().all(|p| p.0[0] == p.0[1] && p.0[1] == p.0[2]));
        let dark = apply_adjust(&im, &Adjust { brightness: 0.5, ..Default::default() }).to_rgb8();
        assert!(dark.get_pixel(30, 10).0[0] <= im.to_rgb8().get_pixel(30, 10).0[0] / 2 + 1);
        let a = auto_enhance(&im);
        assert!((0.85..=1.25).contains(&a.brightness));
        let half = apply_adjust(&im, &Adjust { resize_pct: 50, ..Default::default() });
        assert_eq!(half.dimensions_pair(), (20, 10));
    }

    #[test]
    fn background_sizes_and_corners() {
        let im = DynamicImage::ImageRgb8(photo(100, 50));
        let bg = Background::default();
        let out = add_background(&im, &bg);
        assert_eq!(out.dimensions(), (124, 74));
        let sq = add_background(&im, &Background { aspect: "1:1".into(), ..bg.clone() });
        assert_eq!(sq.width(), sq.height());
        // corner of the photo is rounded away: shows background, not the photo pixel
        let (x, y) = (12, 12);
        assert_ne!(out.get_pixel(x, y).0[..3], im.to_rgb8().get_pixel(0, 0).0);
        // centre is the photo
        assert_eq!(out.get_pixel(62, 37).0[..3], im.to_rgb8().get_pixel(50, 25).0);
    }

    trait Dims {
        fn dimensions_pair(&self) -> (u32, u32);
    }
    impl Dims for DynamicImage {
        fn dimensions_pair(&self) -> (u32, u32) {
            (self.width(), self.height())
        }
    }
}
