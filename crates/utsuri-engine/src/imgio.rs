//! Reading and writing images in every format Utsuri offers.
//! Ported from convert.py (open_image, save_image, image_to, images_to_single_pdf).
//!
//! HEIC/HEIF and AVIF are read through Windows' codecs (WIC). HEIC is written through
//! WIC too (HEIF + HEVC extensions), AVIF with ravif, lossy WebP with libwebp.

use crate::out::{atomic_write, sibling};
use crate::refuse;
use anyhow::{Context as _, Result};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbImage, RgbaImage};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub const IMAGE_TARGETS: &[&str] = &["PNG", "JPG", "WEBP", "HEIC", "TIFF", "AVIF", "BMP", "PDF", "ICO", "GIF"];

pub fn format_hint(t: &str) -> &'static str {
    match t {
        "JPG" => "Small, universal",
        "PNG" => "Lossless · transparency",
        "WEBP" => "Smallest for web",
        "HEIC" => "iPhone format",
        "TIFF" => "Print · archival",
        "AVIF" => "Next-gen, tiny",
        "BMP" => "Uncompressed",
        "PDF" => "Document",
        "ICO" => "Windows icon",
        "GIF" => "Legacy web",
        "DOCX" => "Editable Word",
        "TXT" => "Plain text",
        _ => "",
    }
}

/// Whether HEIC can be written here (the WIC HEIF encoder needs the Store extensions).
pub fn heic_ok() -> bool {
    use std::sync::OnceLock;
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(crate::winrt::heif_encoder_available)
}

/// Format name for a file extension ("jpeg" -> "JPG").
pub fn target_of_ext(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().trim_start_matches('.') {
        "jpg" | "jpeg" => "JPG",
        "png" => "PNG",
        "webp" => "WEBP",
        "tif" | "tiff" => "TIFF",
        "bmp" => "BMP",
        "heic" | "heif" => "HEIC",
        "avif" => "AVIF",
        "gif" => "GIF",
        "ico" => "ICO",
        _ => "PNG",
    }
}

pub fn ext_of_target(t: &str) -> &'static str {
    match t {
        "JPG" => "jpg",
        "PNG" => "png",
        "WEBP" => "webp",
        "TIFF" => "tiff",
        "BMP" => "bmp",
        "HEIC" => "heic",
        "AVIF" => "avif",
        "GIF" => "gif",
        "ICO" => "ico",
        "PDF" => "pdf",
        _ => "png",
    }
}

/// Every image target except the file's own format.
pub fn targets_for(src: &Path) -> Vec<&'static str> {
    let cur = target_of_ext(&crate::kinds::ext(src));
    IMAGE_TARGETS.iter().copied().filter(|t| *t != cur && (*t != "HEIC" || heic_ok())).collect()
}

fn needs_wic(p: &Path) -> bool {
    matches!(crate::kinds::ext(p).as_str(), "heic" | "heif" | "avif")
}

/// Open an image upright (camera rotation applied), as RGB, RGBA or grey.
pub fn open_image(path: &Path) -> Result<DynamicImage> {
    if needs_wic(path) {
        return Ok(DynamicImage::ImageRgba8(crate::winrt::wic_decode(path)?)).map(normalize);
    }
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    let mut decoder = match reader.into_decoder() {
        Ok(d) => d,
        // a file with the wrong extension, or a format only Windows reads
        Err(_) => return crate::winrt::wic_decode(path).map(DynamicImage::ImageRgba8).map_err(|_| unreadable(path)),
    };
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut im = DynamicImage::from_decoder(decoder).map_err(|_| unreadable(path))?;
    im.apply_orientation(orientation);
    Ok(normalize(im))
}

fn unreadable(path: &Path) -> anyhow::Error {
    crate::UserError(format!("{} isn't an image Utsuri can read", path.file_name().unwrap_or_default().to_string_lossy())).into()
}

fn normalize(im: DynamicImage) -> DynamicImage {
    match im {
        DynamicImage::ImageRgb8(_) | DynamicImage::ImageRgba8(_) | DynamicImage::ImageLuma8(_) => im,
        DynamicImage::ImageLumaA8(_) | DynamicImage::ImageRgba16(_) | DynamicImage::ImageRgba32F(_) | DynamicImage::ImageLumaA16(_) => {
            DynamicImage::ImageRgba8(im.to_rgba8())
        }
        _ => {
            if im.color().has_alpha() {
                DynamicImage::ImageRgba8(im.to_rgba8())
            } else {
                DynamicImage::ImageRgb8(im.to_rgb8())
            }
        }
    }
}

/// Fast, reduced-size decode for on-screen previews.
pub fn open_preview(path: &Path, max_side: u32) -> Result<DynamicImage> {
    let im = open_image(path)?;
    Ok(if im.width().max(im.height()) > max_side { im.resize(max_side, max_side, image::imageops::FilterType::Triangle) } else { im })
}

/// Transparent pixels onto a background colour, for formats without alpha.
pub fn flatten(im: &DynamicImage, bg: [u8; 3]) -> RgbImage {
    if !im.color().has_alpha() {
        return im.to_rgb8();
    }
    let rgba = im.to_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let a = a as u32;
        let f = |c: u8, k: u8| ((c as u32 * a + k as u32 * (255 - a) + 127) / 255) as u8;
        image::Rgb([f(r, bg[0]), f(g, bg[1]), f(b, bg[2])])
    })
}

/// Encode `im` as `target` into `w`. `quality` 1..=100 for lossy formats.
pub fn encode(im: &DynamicImage, target: &str, quality: u8, w: &mut dyn Write) -> Result<()> {
    let q = quality.clamp(1, 100);
    match target {
        "JPG" => {
            let rgb = flatten(im, [255, 255, 255]);
            image::codecs::jpeg::JpegEncoder::new_with_quality(w, q).encode_image(&rgb)?;
        }
        "PNG" => {
            let enc = image::codecs::png::PngEncoder::new_with_quality(w, image::codecs::png::CompressionType::Best, image::codecs::png::FilterType::Adaptive);
            im.write_with_encoder(enc)?;
        }
        "WEBP" => {
            let rgba = im.to_rgba8();
            let has_alpha = rgba.pixels().any(|p| p.0[3] < 255);
            let mem = if has_alpha {
                webp::Encoder::from_rgba(rgba.as_raw(), rgba.width(), rgba.height()).encode(q as f32)
            } else {
                let rgb = im.to_rgb8();
                webp::Encoder::from_rgb(rgb.as_raw(), rgb.width(), rgb.height()).encode(q as f32)
            };
            w.write_all(&mem)?;
        }
        "AVIF" => {
            let enc = image::codecs::avif::AvifEncoder::new_with_speed_quality(w, 8, q);
            DynamicImage::ImageRgba8(im.to_rgba8()).write_with_encoder(enc)?;
        }
        "BMP" => DynamicImage::ImageRgb8(flatten(im, [255, 255, 255])).write_to(&mut SeekWrite::new(w), ImageFormat::Bmp)?,
        "TIFF" => im.write_to(&mut SeekWrite::new(w), ImageFormat::Tiff)?,
        "GIF" => DynamicImage::ImageRgba8(im.to_rgba8()).write_to(&mut SeekWrite::new(w), ImageFormat::Gif)?,
        "ICO" => {
            let side = im.width().max(im.height()).min(256);
            let sq = contain_square(im, side);
            DynamicImage::ImageRgba8(sq).write_to(&mut SeekWrite::new(w), ImageFormat::Ico)?;
        }
        other => refuse!("Can't save images as {other}"),
    }
    Ok(())
}

/// The image centred on a transparent square (icons are square).
fn contain_square(im: &DynamicImage, side: u32) -> RgbaImage {
    let r = im.resize(side, side, image::imageops::FilterType::Lanczos3).to_rgba8();
    let mut sq = RgbaImage::new(side, side);
    image::imageops::overlay(&mut sq, &r, ((side - r.width()) / 2) as i64, ((side - r.height()) / 2) as i64);
    sq
}

/// Some encoders need Seek; buffer them in memory.
struct SeekWrite<'a> {
    out: &'a mut dyn Write,
    buf: std::io::Cursor<Vec<u8>>,
}

impl<'a> SeekWrite<'a> {
    fn new(out: &'a mut dyn Write) -> Self {
        Self { out, buf: std::io::Cursor::new(Vec::new()) }
    }
}

impl Write for SeekWrite<'_> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.buf.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl std::io::Seek for SeekWrite<'_> {
    fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
        self.buf.seek(p)
    }
}

impl Drop for SeekWrite<'_> {
    fn drop(&mut self) {
        let _ = self.out.write_all(self.buf.get_ref());
    }
}

/// Bytes `im` takes as `target` at `quality`.
pub fn encoded_size(im: &DynamicImage, target: &str, quality: u8) -> Result<usize> {
    let mut v = Vec::new();
    if target == "PNG" {
        crate::imaging::write_quantized_png(im, quality, &mut v)?;
    } else {
        encode(im, target, quality, &mut v)?;
    }
    Ok(v.len())
}

/// Save atomically. HEIC goes through WIC (it writes the file itself), PDF through krilla.
pub fn save_image(im: &DynamicImage, out: &Path, target: &str, quality: Option<u8>, exif: Option<&[u8]>) -> Result<()> {
    let q = quality.unwrap_or(crate::options().jpg_quality);
    atomic_write(out, |tmp| {
        let r: Result<()> = (|| {
            match target {
                "HEIC" => crate::winrt::wic_encode_heic(&im.to_rgba8(), tmp, q as f32 / 100.0)?,
                "PDF" => std::fs::write(tmp, crate::pdfgen::images_pdf(&[im.clone()])?)?,
                _ => {
                    let mut v = Vec::new();
                    encode(im, target, q, &mut v)?;
                    if let Some(exif) = exif {
                        v = with_exif(v, target, exif)?;
                    }
                    let mut f = BufWriter::new(std::fs::File::create(tmp)?);
                    f.write_all(&v)?;
                    f.flush()?;
                }
            }
            Ok(())
        })();
        r.map_err(std::io::Error::other)
    })
    .with_context(|| format!("couldn't save {}", out.display()))
}

/// Put an EXIF block into encoded JPEG/PNG/WebP bytes (other formats unchanged).
fn with_exif(bytes: Vec<u8>, target: &str, exif: &[u8]) -> Result<Vec<u8>> {
    use img_parts::{Bytes, ImageEXIF};
    let exif = Bytes::copy_from_slice(exif);
    let out = match target {
        "JPG" => {
            let mut j = img_parts::jpeg::Jpeg::from_bytes(bytes.into())?;
            j.set_exif(Some(exif));
            j.encoder().bytes().to_vec()
        }
        "PNG" => {
            let mut p = img_parts::png::Png::from_bytes(bytes.into())?;
            p.set_exif(Some(exif));
            p.encoder().bytes().to_vec()
        }
        "WEBP" => {
            let mut w = img_parts::webp::WebP::from_bytes(bytes.into())?;
            w.set_exif(Some(exif));
            w.encoder().bytes().to_vec()
        }
        _ => bytes,
    };
    Ok(out)
}

/// Convert one image to `target`, saved beside it.
pub fn image_to(src: &Path, target: &str) -> Result<PathBuf> {
    if !IMAGE_TARGETS.contains(&target) {
        refuse!("Can't convert images to {target}");
    }
    let im = open_image(src)?;
    let out = sibling(src, "", Some(ext_of_target(target)));
    save_image(&im, &out, target, None, None)?;
    Ok(out)
}

/// Several images become one PDF, a page each.
pub fn images_to_single_pdf(srcs: &[PathBuf]) -> Result<PathBuf> {
    let pages = srcs.iter().map(|s| open_image(s)).collect::<Result<Vec<_>>>()?;
    let out = sibling(&srcs[0], "Combined", Some("pdf"));
    let bytes = crate::pdfgen::images_pdf(&pages)?;
    atomic_write(&out, |tmp| std::fs::write(tmp, &bytes))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{photo, tmpdir};

    #[test]
    fn every_target_round_trips() {
        let d = tmpdir("imgio");
        let src = d.join("pic.png");
        let im = RgbaImage::from_fn(60, 40, |x, y| image::Rgba([(x * 4) as u8, (y * 6) as u8, 128, if x < 10 { 0 } else { 255 }]));
        im.save(&src).unwrap();
        for t in targets_for(&src) {
            let out = image_to(&src, t).unwrap_or_else(|e| panic!("{t}: {e:#}"));
            assert!(out.exists() && out.metadata().unwrap().len() > 0, "{t}");
            assert_eq!(out.extension().unwrap(), ext_of_target(t), "{t}");
            if t == "PDF" {
                assert_eq!(crate::pdf::page_count(&out).unwrap(), 1);
                continue;
            }
            let back = open_image(&out).unwrap_or_else(|e| panic!("reopen {t}: {e:#}"));
            if t == "ICO" {
                assert_eq!((back.width(), back.height()), (60, 60), "{t}");
            } else {
                assert_eq!((back.width(), back.height()), (60, 40), "{t}");
            }
        }
        assert!(targets_for(&src).contains(&"HEIC"), "HEIC encoder should be available on this PC");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn lossy_webp_is_smaller_than_lossless_png() {
        let d = tmpdir("webp");
        let src = d.join("p.png");
        photo(400, 300).save(&src).unwrap();
        let w = image_to(&src, "WEBP").unwrap();
        assert!(w.metadata().unwrap().len() < src.metadata().unwrap().len() / 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn jpg_keeps_exif_and_orientation_is_applied() {
        let d = tmpdir("exif");
        let src = d.join("p.jpg");
        let im = DynamicImage::ImageRgb8(photo(30, 20));
        // orientation 6 = rotate 90° clockwise to view
        let exif = crate::imaging::tests_exif_with(6, None);
        save_image(&im, &src, "JPG", Some(90), Some(&exif)).unwrap();
        let back = open_image(&src).unwrap();
        assert_eq!((back.width(), back.height()), (20, 30));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn several_images_one_pdf() {
        let d = tmpdir("multi");
        let a = d.join("a.png");
        let b = d.join("b.jpg");
        photo(50, 50).save(&a).unwrap();
        photo(80, 40).save(&b).unwrap();
        let out = images_to_single_pdf(&[a, b]).unwrap();
        assert_eq!(out.file_name().unwrap(), "a Combined.pdf");
        assert_eq!(crate::pdf::page_count(&out).unwrap(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }
}
