//! Image conversions. Ported from utsuri/convert.py (image_to, save_image).
//! Not yet ported: HEIC, AVIF and PDF targets, PDF/DOCX/text/video/audio sources.

use anyhow::{Context as _, Result, bail};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbImage};
use shared::paths::{atomic_write, sibling};
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

pub const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "tif", "tiff", "bmp", "gif", "ico"];
/// Order matches utsuri/convert.py IMAGE_TARGETS, minus the formats not ported yet.
pub const IMAGE_TARGETS: &[&str] = &["PNG", "JPG", "WEBP", "TIFF", "BMP", "ICO", "GIF"];
pub const JPG_QUALITY: u8 = 90;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Image,
}

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

pub fn category(p: &Path) -> Option<Category> {
    IMAGE_EXTS.contains(&ext_of(p).as_str()).then_some(Category::Image)
}

/// Every image target except the file's own format.
pub fn image_targets(src: &Path) -> Vec<&'static str> {
    let cur = match ext_of(src).as_str() {
        "jpg" | "jpeg" => "JPG".to_string(),
        "tif" | "tiff" => "TIFF".to_string(),
        e => e.to_ascii_uppercase(),
    };
    IMAGE_TARGETS.iter().copied().filter(|t| *t != cur).collect()
}

pub fn hint(target: &str) -> &'static str {
    match target {
        "JPG" => "Small, universal",
        "PNG" => "Lossless · transparency",
        "WEBP" => "Smallest for web",
        "TIFF" => "Print · archival",
        "BMP" => "Uncompressed",
        "ICO" => "Windows icon",
        "GIF" => "Legacy web",
        _ => "",
    }
}

/// Open an image upright (EXIF orientation applied).
pub fn open_image(src: &Path) -> Result<DynamicImage> {
    let mut decoder = ImageReader::open(src)?.with_guessed_format()?.into_decoder()
        .with_context(|| format!("{} isn't an image Utsuri can read", src.display()))?;
    let orientation = decoder.orientation()?;
    let mut im = DynamicImage::from_decoder(decoder)?;
    im.apply_orientation(orientation);
    Ok(im)
}

/// Transparent pixels onto white, for formats without alpha.
fn flatten(im: &DynamicImage) -> RgbImage {
    let rgba = im.to_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let a = a as u32;
        let f = |c: u8| ((c as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
        image::Rgb([f(r), f(g), f(b)])
    })
}

pub fn save_image(im: &DynamicImage, out: &Path, target: &str) -> Result<()> {
    atomic_write(out, |tmp| {
        let w = &mut BufWriter::new(File::create(tmp)?);
        let r = match target {
            "JPG" => image::codecs::jpeg::JpegEncoder::new_with_quality(w, JPG_QUALITY)
                .encode_image(&DynamicImage::ImageRgb8(flatten(im))),
            "BMP" => DynamicImage::ImageRgb8(flatten(im)).write_to(w, ImageFormat::Bmp),
            "PNG" => im.write_to(w, ImageFormat::Png),
            // The image crate only encodes lossless WebP; the Python app used quality 90.
            "WEBP" => DynamicImage::ImageRgba8(im.to_rgba8()).write_to(w, ImageFormat::WebP),
            "TIFF" => im.write_to(w, ImageFormat::Tiff),
            "GIF" => DynamicImage::ImageRgba8(im.to_rgba8()).write_to(w, ImageFormat::Gif),
            "ICO" => {
                let side = im.width().max(im.height()).min(256);
                DynamicImage::ImageRgba8(im.resize(side, side, image::imageops::FilterType::Lanczos3).to_rgba8())
                    .write_to(w, ImageFormat::Ico)
            }
            other => return Err(std::io::Error::other(format!("can't convert to {other}"))),
        };
        r.map_err(std::io::Error::other)
    })
    .with_context(|| format!("couldn't save {}", out.display()))
}

/// Convert `src` to `target`, saved beside it. Returns the new file.
pub fn image_to(src: &Path, target: &str) -> Result<PathBuf> {
    if !IMAGE_TARGETS.contains(&target) {
        bail!("can't convert to {target} yet");
    }
    let im = open_image(src)?;
    let out = sibling(src, None, "", Some(&target.to_ascii_lowercase()));
    save_image(&im, &out, target)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("utsuri-convert-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn targets_skip_own_format() {
        assert!(!image_targets(Path::new("a.JPEG")).contains(&"JPG"));
        assert!(!image_targets(Path::new("a.tif")).contains(&"TIFF"));
        assert_eq!(image_targets(Path::new("a.heic")).len(), IMAGE_TARGETS.len());
        assert_eq!(category(Path::new("x.PNG")), Some(Category::Image));
        assert_eq!(category(Path::new("x.pdf")), None);
    }

    #[test]
    fn png_with_alpha_to_every_target() {
        let d = tmpdir("all");
        let src = d.join("pic.png");
        let im = image::RgbaImage::from_fn(40, 30, |x, _| image::Rgba([255, 0, 0, if x < 20 { 0 } else { 255 }]));
        im.save(&src).unwrap();
        for t in image_targets(&src) {
            let out = image_to(&src, t).unwrap();
            assert!(out.exists(), "{t}");
            let back = image::open(&out).unwrap();
            let expect = if t == "ICO" { (40, 40) } else { (40, 30) };
            // ICO pads? no: resize keeps aspect, so check the long side only
            if t == "ICO" {
                assert_eq!(back.width().max(back.height()), expect.0, "{t}");
            } else {
                assert_eq!((back.width(), back.height()), expect, "{t}");
            }
        }
        // JPG flattened transparency onto white
        let jpg = image::open(d.join("pic.jpg")).unwrap().to_rgb8();
        let p = jpg.get_pixel(2, 2).0;
        assert!(p.iter().all(|&c| c > 230), "transparent -> white, got {p:?}");
        // never overwrites
        let again = image_to(&src, "JPG").unwrap();
        assert_eq!(again, d.join("pic (1).jpg"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
