//! The real-screen cases and their checks, shared by the Windows and Linux drivers: what to
//! drag, which slice to drop it on, and what the result must be (checked with the engine).

use anyhow::{Context as _, Result, bail};
use std::path::{Path, PathBuf};

/// Menu geometry (matches crates/utsuri/src/radial.rs).
pub(crate) const R_IN: f32 = 60.0;
pub(crate) const R_OUT: f32 = 158.0;
/// What a case should produce.
pub(crate) type Check = Box<dyn Fn(&Outcome) -> Result<String>>;

#[allow(dead_code)]
pub(crate) struct Outcome {
    pub(crate) outs: Vec<PathBuf>,
    pub(crate) title: String,
    pub(crate) detail: String,
    pub(crate) dir: PathBuf,
    pub(crate) srcs: Vec<PathBuf>,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Drive {
    /// a background job: nothing to do after the drop
    None,
    /// a tool window: press Enter
    Enter,
    /// draw a shape on the canvas, then Enter
    DrawEnter,
    /// type a password (twice when `twice`), then Enter
    Password(&'static str, bool),
}

pub(crate) struct Case {
    pub(crate) name: String,
    pub(crate) files: Vec<&'static str>,
    pub(crate) key: String,
    pub(crate) drive: Drive,
    pub(crate) window: Option<&'static str>,
    pub(crate) check: Check,
    /// expect an error toast instead of a result
    pub(crate) expect_error: Option<&'static str>,
}

pub(crate) fn case(name: &str, files: &[&'static str], key: &str, check: impl Fn(&Outcome) -> Result<String> + 'static) -> Case {
    Case { name: name.into(), files: files.to_vec(), key: key.into(), drive: Drive::None, window: None, check: Box::new(check), expect_error: None }
}

impl Case {
    pub(crate) fn tool(mut self, window: &'static str, drive: Drive) -> Self {
        self.window = Some(window);
        self.drive = drive;
        self
    }
    pub(crate) fn error(mut self, msg: &'static str) -> Self {
        self.expect_error = Some(msg);
        self
    }
}

// ---------------------------------------------------------------- checks

pub(crate) fn one(o: &Outcome) -> Result<&PathBuf> {
    match o.outs.as_slice() {
        [p] => Ok(p),
        v => bail!("expected one output, got {}", v.len()),
    }
}

pub(crate) fn exists_nonempty(p: &Path) -> Result<()> {
    let n = std::fs::metadata(p).with_context(|| format!("{} missing", p.display()))?.len();
    if n == 0 {
        bail!("{} is empty", p.display());
    }
    Ok(())
}

pub(crate) fn originals_intact(o: &Outcome) -> Result<()> {
    for s in &o.srcs {
        if !s.exists() {
            bail!("the original {} is gone", s.display());
        }
    }
    Ok(())
}

pub(crate) fn image_out(ext: &'static str, dims: Option<(u32, u32)>) -> impl Fn(&Outcome) -> Result<String> {
    move |o| {
        let p = one(o)?;
        exists_nonempty(p)?;
        if utsuri_engine::kinds::ext(p) != ext {
            bail!("expected .{ext}, got {}", p.display());
        }
        let im = utsuri_engine::imgio::open_image(p)?;
        if let Some(d) = dims {
            if (im.width(), im.height()) != d {
                bail!("expected {}x{}, got {}x{}", d.0, d.1, im.width(), im.height());
            }
        }
        originals_intact(o)?;
        Ok(format!("{} {}x{}", p.file_name().unwrap().to_string_lossy(), im.width(), im.height()))
    }
}

pub(crate) fn pdf_pages(n: usize) -> impl Fn(&Outcome) -> Result<String> {
    move |o| {
        let p = one(o)?;
        let got = utsuri_engine::pdf::page_count(p)?;
        if got != n {
            bail!("expected {n} pages, got {got}");
        }
        originals_intact(o)?;
        Ok(format!("{} · {got} pages", p.file_name().unwrap().to_string_lossy()))
    }
}

pub(crate) fn outs_count(n: usize, ext: &'static str) -> impl Fn(&Outcome) -> Result<String> {
    move |o| {
        if o.outs.len() != n {
            bail!("expected {n} outputs, got {}", o.outs.len());
        }
        for p in &o.outs {
            exists_nonempty(p)?;
            if utsuri_engine::kinds::ext(p) != ext {
                bail!("expected .{ext}: {}", p.display());
            }
        }
        originals_intact(o)?;
        Ok(format!("{n} × .{ext}"))
    }
}

pub(crate) fn file_ext(ext: &'static str) -> impl Fn(&Outcome) -> Result<String> {
    move |o| {
        let p = one(o)?;
        exists_nonempty(p)?;
        if utsuri_engine::kinds::ext(p) != ext {
            bail!("expected .{ext}, got {}", p.display());
        }
        originals_intact(o)?;
        Ok(format!("{} ({} bytes)", p.file_name().unwrap().to_string_lossy(), std::fs::metadata(p)?.len()))
    }
}

pub(crate) fn text_contains(ext: &'static str, needle: &'static str) -> impl Fn(&Outcome) -> Result<String> {
    move |o| {
        let p = one(o)?;
        let text = match ext {
            "pdf" => utsuri_engine::pdf::extract_text(p)?,
            "docx" => utsuri_engine::docx::read_text(p)?,
            _ => std::fs::read_to_string(p)?,
        };
        if !text.contains(needle) {
            bail!("{} doesn't contain {needle:?}: {:?}", p.display(), text.chars().take(120).collect::<String>());
        }
        originals_intact(o)?;
        Ok(format!("{} has {needle:?}", p.file_name().unwrap().to_string_lossy()))
    }
}

pub(crate) fn smaller(o: &Outcome) -> Result<String> {
    let p = one(o)?;
    let (a, b) = (std::fs::metadata(&o.srcs[0])?.len(), std::fs::metadata(p)?.len());
    if b >= a {
        bail!("not smaller: {a} -> {b}");
    }
    originals_intact(o)?;
    Ok(format!("{a} → {b} bytes"))
}

// ---------------------------------------------------------------- the cases

pub(crate) fn cases() -> Vec<Case> {
    let mut v = vec![];
    // images: every conversion
    for (t, ext) in [("PNG", "png"), ("WEBP", "webp"), ("HEIC", "heic"), ("TIFF", "tiff"), ("AVIF", "avif"), ("BMP", "bmp"), ("GIF", "gif")] {
        v.push(case(&format!("image→{t}"), &["photo.jpg"], &format!("convert:{t}"), image_out(ext, Some((1600, 1067)))));
    }
    v.push(case("image→ICO", &["photo.jpg"], "convert:ICO", image_out("ico", Some((256, 256)))));
    v.push(case("image→PDF", &["photo.jpg"], "convert:PDF", pdf_pages(1)));
    v.push(case("png→JPG", &["logo.png"], "convert:JPG", image_out("jpg", Some((400, 300)))));
    v.push(case("2 images→1 PDF", &["logo.png", "screenshot.png"], "convert:PDF", pdf_pages(2)));
    // image tools (windows)
    v.push(case("Compress", &["photo.jpg"], "tool:Compress", smaller).tool("Compress — Utsuri", Drive::Enter));
    v.push(case("Resize 50%", &["photo.jpg"], "tool:Resize", image_out("jpg", Some((800, 534)))).tool("Resize — Utsuri", Drive::Enter));
    v.push(case("Crop", &["photo.jpg"], "tool:Crop", image_out("jpg", Some((1600, 1067)))).tool("Crop — Utsuri", Drive::Enter));
    v.push(case("Edit", &["photo.jpg"], "tool:Edit", image_out("jpg", Some((1600, 1067)))).tool("Edit — Utsuri", Drive::Enter));
    v.push(case("Annotate", &["screenshot.png"], "tool:Annotate", image_out("png", Some((800, 500)))).tool("Annotate — Utsuri", Drive::DrawEnter));
    v.push(case("Redact", &["screenshot.png"], "tool:Redact", image_out("png", Some((800, 500)))).tool("Redact — Utsuri", Drive::DrawEnter));
    v.push(case("Add BG", &["logo.png"], "tool:Add BG", image_out("png", None)).tool("Add background — Utsuri", Drive::Enter));
    v.push(case("Metadata (remove all)", &["photo.jpg"], "tool:Metadata", |o| {
        let p = one(o)?;
        let (n, gps) = utsuri_engine::imaging::count_metadata(p);
        if n != 0 || gps {
            bail!("still {n} fields, gps={gps}");
        }
        Ok(format!("{} clean", p.file_name().unwrap().to_string_lossy()))
    })
    .tool("Metadata — Utsuri", Drive::Enter));
    // image extras
    v.push(case("Copy image", &["logo.png"], "x:copy", |_| {
        let im = arboard::Clipboard::new()?.get_image()?;
        if (im.width, im.height) != (400, 300) {
            bail!("clipboard has {}x{}", im.width, im.height);
        }
        Ok("400x300 on the clipboard".into())
    }));
    v.push(case("OCR text", &["text-picture.png"], "x:ocr", |o| {
        let p = one(o)?;
        let t = std::fs::read_to_string(p)?;
        let clip = arboard::Clipboard::new()?.get_text()?;
        if !t.contains("Utsuri") || !clip.contains("Utsuri") {
            bail!("text {t:?} / clipboard {clip:?}");
        }
        Ok(format!("read {:?}", t.trim()))
    }));
    // PDF conversions
    v.push(case("pdf→JPG", &["report.pdf"], "convert:JPG", outs_count(6, "jpg")));
    v.push(case("pdf→PNG", &["appendix.pdf"], "convert:PNG", outs_count(2, "png")));
    v.push(case("pdf→DOCX", &["report.pdf"], "convert:DOCX", text_contains("docx", "Page 6")));
    v.push(case("pdf→TXT", &["report.pdf"], "convert:TXT", text_contains("txt", "Page 3")));
    // PDF tools
    v.push(case("PDF Compress", &["scan.pdf"], "tool:Compress", file_ext("pdf")));
    v.push(case("PDF Split", &["appendix.pdf"], "tool:Split", outs_count(2, "pdf")));
    v.push(case("PDF Rotate", &["appendix.pdf"], "tool:Rotate", pdf_pages(2)));
    v.push(case("PDF Merge", &["appendix.pdf", "report.pdf"], "tool:Merge", pdf_pages(8)));
    v.push(case("PDF Extract images", &["scan.pdf"], "tool:Extract", outs_count(1, "png")));
    v.push(case("PDF Extract (none)", &["report.pdf"], "tool:Extract", |_| Ok(String::new())).error("No images found"));
    v.push(case("PDF Protect", &["appendix.pdf"], "tool:Protect", |o| {
        let p = one(o)?;
        if !utsuri_engine::pdf::needs_password(p) || !utsuri_engine::pdf::password_ok(p, "e2e-pass") {
            bail!("not protected with the typed password");
        }
        Ok("AES-256, password works".into())
    })
    .tool("Protect PDF — Utsuri", Drive::Password("e2e-pass", true)));
    v.push(case("PDF Metadata", &["report.pdf"], "tool:Metadata", |o| {
        let p = one(o)?;
        let m = utsuri_engine::pdf::read_pdf_metadata(p)?;
        if m.iter().any(|(k, _)| k == "Author" || k == "Title") {
            bail!("metadata left: {m:?}");
        }
        Ok("Info removed".into())
    })
    .tool("Metadata — Utsuri", Drive::Enter));
    v.push(case("PDF Organize", &["report.pdf"], "xw:organize", pdf_pages(6)).tool("Organize pages — Utsuri", Drive::Enter));
    v.push(case("PDF Stamp", &["appendix.pdf"], "xw:watermark", pdf_pages(2)).tool("Stamp — Utsuri", Drive::Enter));
    v.push(case("PDF Unlock", &["locked.pdf"], "xw:unlock", |o| {
        let p = one(o)?;
        if utsuri_engine::pdf::is_encrypted(p)? {
            bail!("still encrypted");
        }
        Ok(format!("{} pages, no password", utsuri_engine::pdf::page_count(p)?))
    })
    .tool("Unlock PDF — Utsuri", Drive::Password("secret", false)));
    v.push(case("PDF Numbers", &["report.pdf"], "x:numbers", text_contains("pdf", "6 / 6")));
    v.push(case("PDF Gray", &["appendix.pdf"], "x:gray", pdf_pages(2)));
    v.push(case("PDF Flatten", &["report.pdf"], "x:flatten", text_contains("pdf", "Filled in by Utsuri")));
    v.push(case("PDF Halves", &["report.pdf"], "x:halves", outs_count(2, "pdf")));
    v.push(case("PDF Chapters", &["report.pdf"], "x:chapters", outs_count(3, "pdf")));
    v.push(case("PDF OCR", &["scan.pdf"], "x:searchable", text_contains("pdf", "OCR")));
    // Word and text
    v.push(case("docx→PDF", &["letter.docx"], "convert:PDF", text_contains("pdf", "Dear reader")));
    v.push(case("docx→TXT", &["letter.docx"], "convert:TXT", text_contains("txt", "italic")));
    v.push(case("txt→PDF", &["notes.txt"], "convert:PDF", text_contains("pdf", "Tiếng Việt")));
    v.push(case("txt→DOCX", &["notes.txt"], "convert:DOCX", text_contains("docx", "Second line")));
    v.push(case("md→PDF", &["readme.md"], "convert:PDF", text_contains("pdf", "markdown")));
    // video and audio
    v.push(case("video→WEBM", &["clip.mp4"], "xc:WEBM", file_ext("webm")));
    v.push(case("video→GIF", &["clip.mp4"], "xc:GIF", image_out("gif", Some((320, 240)))));
    v.push(case("video→MP3", &["clip.mp4"], "xc:MP3", file_ext("mp3")));
    v.push(case("video Compress", &["clip.mp4"], "x:compress", file_ext("mp4")));
    v.push(case("audio→MP3", &["song.wav"], "xc:MP3", file_ext("mp3")));
    v.push(case("audio→OPUS", &["song.wav"], "xc:OPUS", file_ext("opus")));
    v.push(case("audio→FLAC", &["song.wav"], "xc:FLAC", file_ext("flac")));
    // archives, folders, anything
    v.push(case("ZIP Extract", &["bundle.zip"], "x:extract", |o| {
        let p = one(o)?;
        if std::fs::read_to_string(p.join("a.txt")).ok().as_deref() != Some("alpha") {
            bail!("a.txt missing in {}", p.display());
        }
        Ok(format!("{}/a.txt", p.file_name().unwrap().to_string_lossy()))
    }));
    for (k, ext) in [("batch-webp", "webp"), ("batch-jpg", "jpg")] {
        v.push(case(&format!("Folder {k}"), &["holiday"], &format!("x:{k}"), move |o| {
            let root = one(o)?;
            let n = walk(root).iter().filter(|p| utsuri_engine::kinds::ext(p) == ext).count();
            if n < 1 {
                bail!("no .{ext} under {}", root.display());
            }
            Ok(format!("{n} × .{ext} in {}", root.file_name().unwrap().to_string_lossy()))
        }));
    }
    v.push(case("Folder compress", &["holiday"], "x:batch-compress", |o| {
        let root = one(o)?;
        let n = walk(root).len();
        if n != 2 {
            bail!("expected 2 images, got {n}");
        }
        Ok(format!("{n} compressed"))
    }));
    for (k, ext) in [("zip", "zip"), ("tar-gz", "gz"), ("tar-xz", "xz")] {
        v.push(case(&format!("Pack {k}"), &["data.bin"], &format!("x:{k}"), file_ext(ext)));
    }
    v.push(case("Pack folder ZIP", &["holiday"], "x:zip", file_ext("zip")));
    v.push(case("Copy path", &["notes.txt"], "x:copy-path", |o| {
        let t = arboard::Clipboard::new()?.get_text()?;
        if !t.ends_with("notes.txt") || !t.starts_with(&o.dir.display().to_string()[..2]) {
            bail!("clipboard: {t:?}");
        }
        Ok(t)
    }));
    v
}

pub(crate) fn walk(p: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let q = e.path();
            if q.is_dir() {
                out.extend(walk(&q));
            } else {
                out.push(q);
            }
        }
    }
    out
}

pub(crate) struct Result1 {
    pub(crate) name: String,
    pub(crate) ok: bool,
    pub(crate) msg: String,
    pub(crate) secs: f32,
}

pub(crate) fn parse_done(log: &str) -> Option<Result<(String, String, Vec<PathBuf>), String>> {
    for line in log.lines() {
        if let Some(rest) = line.split_once(" utsuri: done: ").map(|x| x.1) {
            let mut parts: Vec<&str> = rest.split(" · ").collect();
            let outs_part = parts.pop().unwrap_or("");
            let title = parts.first().copied().unwrap_or("").to_string();
            let detail = parts.get(1..).map(|d| d.join(" · ")).unwrap_or_default();
            let outs = outs_part.split(" | ").filter(|s| !s.is_empty()).map(PathBuf::from).collect();
            return Some(Ok((title, detail, outs)));
        }
        for marker in [" utsuri: Couldn't finish: ", " utsuri: Couldn't start that: ", " utsuri: Couldn't open that: "] {
            if let Some(rest) = line.split_once(marker).map(|x| x.1) {
                return Some(Err(rest.to_string()));
            }
        }
    }
    None
}

/// Where slice `key` is for these files: (ring index, slice index, slices in ring).
pub(crate) fn slice_of(files: &[PathBuf], key: &str) -> Result<(usize, usize, usize)> {
    let rings = utsuri_engine::actions::rings(files);
    for (ri, (_, slices)) in rings.iter().enumerate() {
        if let Some(si) = slices.iter().position(|s| s.key == key) {
            return Ok((ri, si, slices.len()));
        }
    }
    bail!("no slice {key} in {:?}", rings.iter().map(|r| &r.0).collect::<Vec<_>>())
}

/// Screen point of slice `i` of `n` around `c` at `scale`.
pub(crate) fn slice_point(c: (i32, i32), n: usize, i: usize, scale: f32) -> (i32, i32) {
    let span = 360.0 / n as f32;
    // petal 0 is centred at 12 o'clock (radial.rs angles())
    let mid = 90.0 - i as f32 * span;
    let r = (R_IN + R_OUT) / 2.0 * scale;
    let a = mid.to_radians();
    (c.0 + (r * a.cos()) as i32, c.1 - (r * a.sin()) as i32)
}

pub(crate) fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()))?;
        } else {
            std::fs::copy(&p, to.join(e.file_name()))?;
        }
    }
    Ok(())
}

