//! Test files for the E2E run and screenshots: one of every kind Utsuri handles.

use anyhow::Result;
use lopdf::{Document, Object, Stream, dictionary};
use std::path::{Path, PathBuf};

/// A colourful photo-like image with a gradient.
pub fn photo(w: u32, h: u32) -> image::RgbImage {
    image::RgbImage::from_fn(w, h, |x, y| {
        let n = ((x * 7919 + y * 104729) % 23) as u8;
        image::Rgb([(x * 255 / w) as u8 ^ n, (y * 255 / h) as u8, ((x + y) % 256) as u8])
    })
}

/// `n` pages, each with "Page N" in Helvetica and a red box; optional bookmarks and a
/// filled-in form field on page 1.
pub fn sample_pdf(path: &Path, n: usize, bookmarks: bool, form: bool) -> Result<()> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! {"Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica"});
    let res = doc.add_object(dictionary! {"Font" => dictionary! {"F1" => font}});
    let mut kids = vec![];
    let mut page_ids = vec![];
    for i in 0..n {
        let content = format!("1 0 0 rg 100 500 200 100 re f BT /F1 24 Tf 100 700 Td (Page {}) Tj ET BT /F1 12 Tf 100 660 Td (Utsuri test document, chapter {}) Tj ET", i + 1, i / 2 + 1);
        let cid = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let pid = doc.add_object(dictionary! {"Type" => "Page", "Parent" => pages_id, "Contents" => cid});
        kids.push(Object::Reference(pid));
        page_ids.push(pid);
    }
    let count = kids.len() as i64;
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => kids, "Count" => count, "Resources" => res, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()]}));
    let mut catalog = dictionary! {"Type" => "Catalog", "Pages" => pages_id};
    if bookmarks && n >= 2 {
        // a top-level bookmark every 2 pages
        let outlines = doc.new_object_id();
        let marks: Vec<_> = (0..n).step_by(2).map(|i| (format!("Chapter {}", i / 2 + 1), page_ids[i])).collect();
        let ids: Vec<_> = marks.iter().map(|_| doc.new_object_id()).collect();
        for (k, ((title, page), id)) in marks.iter().zip(&ids).enumerate() {
            let mut d = dictionary! {"Title" => Object::string_literal(title.clone()), "Parent" => outlines, "Dest" => vec![Object::Reference(*page), "Fit".into()]};
            if k > 0 {
                d.set("Prev", ids[k - 1]);
            }
            if k + 1 < ids.len() {
                d.set("Next", ids[k + 1]);
            }
            doc.objects.insert(*id, Object::Dictionary(d));
        }
        doc.objects.insert(outlines, Object::Dictionary(dictionary! {"Type" => "Outlines", "First" => ids[0], "Last" => *ids.last().unwrap(), "Count" => ids.len() as i64}));
        catalog.set("Outlines", outlines);
    }
    if form {
        let ap = doc.add_object(Stream::new(dictionary! {"Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 200.into(), 24.into()], "Resources" => dictionary! {"Font" => dictionary! {"F1" => font}}}, b"0 0 1 rg BT /F1 14 Tf 4 6 Td (Filled in by Utsuri) Tj ET".to_vec()));
        let field = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "T" => Object::string_literal("name"),
            "V" => Object::string_literal("Filled in by Utsuri"), "Rect" => vec![100.into(), 400.into(), 300.into(), 424.into()],
            "P" => page_ids[0], "AP" => dictionary! {"N" => ap},
        });
        doc.get_object_mut(page_ids[0])?.as_dict_mut()?.set("Annots", vec![Object::Reference(field)]);
        catalog.set("AcroForm", dictionary! {"Fields" => vec![Object::Reference(field)]});
    }
    let info = doc.add_object(dictionary! {"Title" => Object::string_literal("Utsuri test report"), "Author" => Object::string_literal("E2E Author"), "Producer" => Object::string_literal("utsuri fixtures")});
    doc.trailer.set("Info", info);
    let cat = doc.add_object(catalog);
    doc.trailer.set("Root", cat);
    doc.save(path)?;
    Ok(())
}

/// A scanned-looking PDF: one page that's only an image of text (needs OCR).
pub fn scanned_pdf(path: &Path) -> Result<()> {
    let mark = crate::pdfgen::text_stamp_image("Scanned page for OCR 2026", 28.0, [0, 0, 0, 255])?;
    let mut page = image::RgbaImage::from_pixel(1275, 1650, image::Rgba([255, 255, 255, 255]));
    image::imageops::overlay(&mut page, &mark, 120, 200);
    let bytes = crate::pdfgen::images_pdf(&[image::DynamicImage::ImageRgba8(page)])?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Write every fixture into `dir`; returns the files.
pub fn write_all(dir: &Path) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let mut out = vec![];
    let p = |n: &str| dir.join(n);
    // photo with camera info and a GPS position (the Eiffel Tower)
    let jpg = p("photo.jpg");
    let exif = crate::imaging::tests_exif_with(1, Some((48.85837, 2.29448)));
    crate::imgio::save_image(&image::DynamicImage::ImageRgb8(photo(1600, 1067)), &jpg, "JPG", Some(95), Some(&exif))?;
    out.push(jpg);
    // a logo with transparency
    let png = p("logo.png");
    image::RgbaImage::from_fn(400, 300, |x, y| {
        let d = ((x as f32 - 200.0).powi(2) + (y as f32 - 150.0).powi(2)).sqrt();
        image::Rgba([40, 120, 230, if d < 120.0 { 255 } else { 0 }])
    })
    .save(&png)?;
    out.push(png);
    let shot = p("screenshot.png");
    photo(800, 500).save(&shot)?;
    out.push(shot);
    // a text picture for OCR
    let txtimg = p("text-picture.png");
    let mark = crate::pdfgen::text_stamp_image("Hello from Utsuri 2026", 30.0, [0, 0, 0, 255])?;
    let mut white = image::RgbaImage::from_pixel(mark.width() + 80, mark.height() + 80, image::Rgba([255, 255, 255, 255]));
    image::imageops::overlay(&mut white, &mark, 40, 40);
    white.save(&txtimg)?;
    out.push(txtimg);
    // PDFs
    let pdf = p("report.pdf");
    sample_pdf(&pdf, 6, true, true)?;
    out.push(pdf.clone());
    let pdf2 = p("appendix.pdf");
    sample_pdf(&pdf2, 2, false, false)?;
    out.push(pdf2);
    let scanned = p("scan.pdf");
    scanned_pdf(&scanned)?;
    out.push(scanned);
    let locked = crate::pdf::protect_pdf(&pdf, "secret")?;
    let locked_named = p("locked.pdf");
    std::fs::rename(&locked, &locked_named)?;
    out.push(locked_named);
    // text and Word
    let txt = p("notes.txt");
    std::fs::write(&txt, "Ghi chú: Tiếng Việt có dấu.\nSecond line.\n\nThird paragraph after a blank line.")?;
    out.push(txt.clone());
    let md = p("readme.md");
    std::fs::write(&md, "# Title\n\nSome *markdown* text.")?;
    out.push(md);
    let docx = p("letter.docx");
    {
        use crate::docx::{Item, Run, Style};
        crate::docx::write_docx(
            &docx,
            &[
                Item::Para { style: Style::Heading(1), runs: vec![Run { text: "Dear reader".into(), ..Default::default() }] },
                Item::Para { style: Style::Normal, runs: vec![Run { text: "This letter has ".into(), ..Default::default() }, Run { text: "bold".into(), bold: true, ..Default::default() }, Run { text: " and ".into(), ..Default::default() }, Run { text: "italic".into(), italic: true, ..Default::default() }, Run { text: " words.".into(), ..Default::default() }] },
                Item::Para { style: Style::List, runs: vec![Run { text: "A list item".into(), ..Default::default() }] },
            ],
        )?;
    }
    out.push(docx);
    // a folder of images
    let folder = p("holiday");
    std::fs::create_dir_all(folder.join("day 2"))?;
    photo(640, 480).save(folder.join("beach.png"))?;
    crate::imgio::save_image(&image::DynamicImage::ImageRgb8(photo(500, 500)), &folder.join("day 2").join("sunset.jpg"), "JPG", Some(90), None)?;
    std::fs::write(folder.join("notes.txt"), "not an image")?;
    out.push(folder);
    // an archive
    let zip_src = p("bundle");
    std::fs::create_dir_all(&zip_src)?;
    std::fs::write(zip_src.join("a.txt"), "alpha")?;
    let (zip, _, _) = crate::archive::pack(std::slice::from_ref(&zip_src), "zip")?;
    std::fs::remove_dir_all(&zip_src)?;
    let zip_named = p("bundle.zip");
    if zip != zip_named {
        std::fs::rename(&zip, &zip_named)?;
    }
    out.push(zip_named);
    // video and audio (needs ffmpeg)
    if let Some(exe) = crate::media::ffmpeg() {
        use crate::proc::Quiet as _;
        let clip = p("clip.mp4");
        let ok = std::process::Command::new(&exe)
            .args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc=duration=2:size=320x240:rate=15", "-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-shortest", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"])
            .arg(&clip)
            .quiet()
            .status()?
            .success();
        if ok {
            out.push(clip.clone());
            let song = p("song.wav");
            if std::process::Command::new(&exe).args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=330:duration=2"]).arg(&song).quiet().status()?.success() {
                out.push(song);
            }
        }
    }
    // an unknown file (only Pack applies)
    let other = p("data.bin");
    std::fs::write(&other, [1u8, 2, 3])?;
    out.push(other);
    Ok(out)
}
