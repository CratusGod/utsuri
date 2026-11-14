//! The menu: which rings and slices a drop gets, and what each slice does.
//! Ported from app.py (_upstream_rings, run_item), more.py (rings, job) and cli.py
//! (tool tables, batch, pack, extract).

use crate::imgio;
use crate::kinds::{Drop, Kind, sort_drop};
use crate::out::{human_size, tree_size};
use crate::refuse;
use crate::report::{Clip, Report, summarize};
use anyhow::Result;
use std::path::{Path, PathBuf};

pub const MAX_SLICES: usize = 8;
pub const IMAGE_TOOLS: &[&str] = &["Compress", "Resize", "Crop", "Edit", "Annotate", "Redact", "Add BG", "Metadata"];
pub const PDF_TOOLS: &[&str] = &["Compress", "Split", "Rotate", "Protect", "Extract", "Metadata"];

pub fn glyph(tool: &str) -> &'static str {
    match tool {
        "Compress" => "⇲",
        "Metadata" => "ⓘ",
        "Redact" => "▮",
        "Crop" => "⌗",
        "Edit" => "☼",
        "Annotate" => "✎",
        "Add BG" => "▣",
        "Resize" => "⤢",
        "Split" => "✂",
        "Rotate" => "⟳",
        "Merge" => "⧉",
        "Protect" => "⚿",
        "Extract" => "⧈",
        _ => "",
    }
}

pub fn tool_hint(tool: &str) -> &'static str {
    match tool {
        "Compress" => "Shrink file size",
        "Metadata" => "View · remove GPS",
        "Redact" => "Hide private info",
        "Crop" => "Trim to a ratio",
        "Edit" => "Light · colour · rotate",
        "Annotate" => "Arrows · text · steps",
        "Add BG" => "Gradient backdrop",
        "Resize" => "Change dimensions",
        "Split" => "One PDF per page",
        "Rotate" => "Turn pages 90°",
        "Merge" => "Join into one PDF",
        "Protect" => "Add a password",
        "Extract" => "Save embedded images",
        _ => "",
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Slice {
    pub label: String,
    pub hint: String,
    pub key: String,
    pub glyph: String,
}

fn slice(label: &str, key: String, hint: &str) -> Slice {
    Slice { label: label.into(), hint: hint.into(), key, glyph: String::new() }
}

/// A tool that needs a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Compress,
    Resize,
    Crop,
    Edit,
    Annotate,
    Redact,
    AddBg,
    Metadata,
    PdfMetadata,
    Protect,
    Organize,
    Stamp,
    Unlock,
}

pub type JobFn = Box<dyn FnOnce() -> Result<Report> + Send>;

pub enum Plan {
    /// Run in the background; `working` is the spinner text.
    Job { action: String, working: String, run: JobFn },
    /// Open a tool window on these files.
    Window { tool: Tool, files: Vec<PathBuf> },
}

pub fn convert_targets(kind: Kind, first: &Path) -> Vec<&'static str> {
    match kind {
        Kind::Image => imgio::targets_for(first),
        Kind::Pdf => vec!["JPG", "PNG", "DOCX", "TXT"],
        Kind::Docx => vec!["PDF", "TXT"],
        Kind::Text => vec!["PDF", "DOCX"],
        Kind::Video | Kind::Audio if crate::media::available() => crate::media::targets_for(kind, first),
        _ => vec![],
    }
}

/// Extra one-click PDF tools (More ring): id -> (label, hint).
const PDF_MORE: &[(&str, &str, &str)] = &[
    ("numbers", "Numbers", "1, 2, 3 at the foot"),
    ("gray", "Gray", "Black and white"),
    ("flatten", "Flatten", "Bake in forms"),
    ("halves", "Halves", "Two files"),
    ("chapters", "Chapters", "One file per chapter"),
];

/// The rings for this drop, in order (Convert, Tools, More..., Pack).
pub fn rings(paths: &[PathBuf]) -> Vec<(String, Vec<Slice>)> {
    rings_for(&sort_drop(paths), crate::ocr::ready(), crate::oumu::available())
}

pub fn rings_for(d: &Drop, ocr: bool, oumu: bool) -> Vec<(String, Vec<Slice>)> {
    let mut out = vec![];
    if d.everything.is_empty() {
        return out;
    }
    let first = &d.files[0];
    let n = d.files.len();
    let conv: Vec<Slice> = convert_targets(d.kind, first)
        .into_iter()
        .map(|t| {
            let hint = if d.kind == Kind::Image && t == "PDF" && n > 1 {
                format!("{n} images → 1 PDF")
            } else if matches!(d.kind, Kind::Video | Kind::Audio) {
                crate::media::hint(t).into()
            } else {
                imgio::format_hint(t).into()
            };
            let key = if matches!(d.kind, Kind::Video | Kind::Audio) { format!("xc:{t}") } else { format!("convert:{t}") };
            Slice { label: t.into(), hint, key, glyph: String::new() }
        })
        .collect();
    if !conv.is_empty() {
        out.push(("Convert".to_string(), conv));
    }
    let tool = |name: &str| Slice { label: name.into(), hint: tool_hint(name).into(), key: format!("tool:{name}"), glyph: glyph(name).into() };
    match d.kind {
        Kind::Image => out.push(("Tools".into(), IMAGE_TOOLS.iter().map(|t| tool(t)).collect())),
        Kind::Pdf => {
            let mut names: Vec<&str> = PDF_TOOLS[..3].to_vec();
            if n > 1 {
                names.push("Merge");
            }
            names.extend(&PDF_TOOLS[3..]);
            out.push(("Tools".into(), names.into_iter().map(tool).collect()));
        }
        _ => {}
    }
    let mut more: Vec<Slice> = vec![];
    match d.kind {
        Kind::Image => {
            more.push(slice("Copy", "x:copy".into(), "Image to clipboard"));
            if ocr {
                more.push(slice("Text", "x:ocr".into(), "Copy the text in it"));
            }
        }
        Kind::Pdf => {
            more.push(slice("Organize", "xw:organize".into(), "Reorder · rotate · delete"));
            more.push(slice("Stamp", "xw:watermark".into(), "Text or image watermark"));
            more.push(slice("Unlock", "xw:unlock".into(), "Remove a password"));
            more.extend(PDF_MORE.iter().map(|(id, l, h)| slice(l, format!("x:{id}"), h)));
            if ocr {
                more.push(slice("OCR", "x:searchable".into(), "OCR every page"));
            }
        }
        Kind::Video if crate::media::available() => more.insert(0, slice("Compress", "x:compress".into(), "Smaller · up to 1080p")),
        Kind::Archive => more.push(slice("Extract", "x:extract".into(), "Unpack beside it")),
        Kind::Folder => {
            more.push(slice("→ WEBP", "x:batch-webp".into(), "Every image inside"));
            more.push(slice("→ JPG", "x:batch-jpg".into(), "Every image inside"));
            more.push(slice("Compress", "x:batch-compress".into(), "Every image inside"));
        }
        _ => {}
    }
    if (matches!(d.kind, Kind::Text | Kind::Docx | Kind::Pdf) || (d.kind == Kind::Image && ocr)) && oumu {
        more.push(slice("Translate", "x:translate".into(), "Opens in Oumu"));
    }
    // spread evenly over as few rings as fit: 9 slices -> 5 + 4, never 8 + 1
    let n_rings = more.len().div_ceil(MAX_SLICES);
    if n_rings > 0 {
        let per = more.len().div_ceil(n_rings);
        for (k, chunk) in more.chunks(per).enumerate() {
            out.push((if k == 0 { "More".into() } else { format!("More {}", k + 1) }, chunk.to_vec()));
        }
    }
    let mut pack: Vec<Slice> = crate::archive::FORMATS.iter().map(|(id, label, _, hint)| slice(label, format!("x:{id}"), hint)).collect();
    pack.push(slice("Path", "x:copy-path".into(), "To clipboard"));
    out.push(("Pack".into(), pack));
    out
}

fn job(action: impl Into<String>, working: impl Into<String>, run: impl FnOnce() -> Result<Report> + Send + 'static) -> Plan {
    Plan::Job { action: action.into(), working: working.into(), run: Box::new(run) }
}

fn each(files: &[PathBuf], f: impl Fn(&Path) -> Result<Vec<PathBuf>>) -> Result<Vec<PathBuf>> {
    let mut outs = vec![];
    for p in files {
        outs.extend(f(p)?);
    }
    Ok(outs)
}

fn name(p: &Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

/// What running slice `key` on this drop means.
pub fn plan(key: &str, paths: &[PathBuf]) -> Result<Plan> {
    let d = sort_drop(paths);
    if d.everything.is_empty() {
        refuse!("The file was moved or deleted");
    }
    let files = d.files.clone();
    let everything = d.everything.clone();
    let (tag, id) = key.split_once(':').unwrap_or((key, ""));
    let id = id.to_string();
    match tag {
        "convert" => {
            let kind = d.kind;
            let t = id.clone();
            let working = if kind == Kind::Image && t == "PDF" && files.len() > 1 { format!("Combining {} images…", files.len()) } else { format!("Converting to {t}…") };
            Ok(job(format!("convert:{t}"), working, move || {
                let outs = convert(kind, &files, &t)?;
                Ok(summarize(&format!("convert:{t}"), &files, &outs, ""))
            }))
        }
        "xc" => {
            let kind = d.kind;
            Ok(job(format!("convert:{id}"), format!("Converting to {id}…"), move || {
                let outs = each(&files, |p| Ok(vec![crate::media::convert(p, kind, &id)?]))?;
                Ok(summarize(&format!("convert:{id}"), &files, &outs, ""))
            }))
        }
        "tool" => tool_plan(&d, &id),
        "xw" => Ok(Plan::Window {
            tool: match id.as_str() {
                "organize" => Tool::Organize,
                "watermark" => Tool::Stamp,
                "unlock" => Tool::Unlock,
                other => refuse!("Unknown window '{other}'"),
            },
            files,
        }),
        "x" => more_plan(&id, files, everything),
        _ => refuse!("Unknown action '{key}'"),
    }
}

fn tool_plan(d: &Drop, name: &str) -> Result<Plan> {
    let files = d.files.clone();
    if d.kind == Kind::Image {
        let tool = match name {
            "Compress" => Tool::Compress,
            "Resize" => Tool::Resize,
            "Crop" => Tool::Crop,
            "Edit" => Tool::Edit,
            "Annotate" => Tool::Annotate,
            "Redact" => Tool::Redact,
            "Add BG" => Tool::AddBg,
            "Metadata" => Tool::Metadata,
            other => refuse!("Unknown tool '{other}'"),
        };
        return Ok(Plan::Window { tool, files });
    }
    if d.kind != Kind::Pdf {
        refuse!("{name} works on images and PDFs");
    }
    let action = format!("tool:{name}");
    let a2 = action.clone();
    let run = |working: &str, f: fn(&Path) -> Result<Vec<PathBuf>>| -> Plan {
        let a = a2.clone();
        job(a.clone(), working, move || {
            let outs = each(&files, f)?;
            Ok(summarize(&a, &files, &outs, ""))
        })
    };
    Ok(match name {
        "Compress" => run("Compressing…", |p| Ok(vec![crate::pdf::compress_pdf(p, 150, 70)?])),
        "Split" => run("Splitting…", crate::pdf::split_pdf),
        "Rotate" => run("Rotating…", |p| Ok(vec![crate::pdf::rotate_pdf(p, 90)?])),
        "Extract" => run("Extracting images…", crate::pdf::extract_images),
        "Merge" => {
            let files = d.files.clone();
            job(action.clone(), "Merging…", move || {
                let out = crate::pdf::merge_pdfs(&files)?;
                Ok(summarize("tool:Merge", &files, &[out], ""))
            })
        }
        "Protect" => Plan::Window { tool: Tool::Protect, files: d.files.clone() },
        "Metadata" => Plan::Window { tool: Tool::PdfMetadata, files: d.files.clone() },
        other => refuse!("Unknown tool '{other}'"),
    })
}

/// Convert every file of `kind` to `t`.
pub fn convert(kind: Kind, files: &[PathBuf], t: &str) -> Result<Vec<PathBuf>> {
    match kind {
        Kind::Image if t == "PDF" && files.len() > 1 => Ok(vec![imgio::images_to_single_pdf(files)?]),
        Kind::Image => each(files, |p| Ok(vec![imgio::image_to(p, t)?])),
        Kind::Pdf => each(files, |p| match t {
            "JPG" | "PNG" => crate::pdf::pdf_to_images(p, t, None),
            "TXT" => Ok(vec![crate::pdf::pdf_to_txt(p)?]),
            "DOCX" => Ok(vec![crate::pdf::pdf_to_docx(p)?]),
            _ => refuse!("Can't convert a PDF to {t}"),
        }),
        Kind::Docx => each(files, |p| match t {
            "PDF" => {
                let blocks = crate::docx::read_blocks(p)?;
                let bytes = crate::pdfgen::blocks_pdf(&blocks)?;
                let o = crate::out::sibling(p, "", Some("pdf"));
                crate::out::atomic_write(&o, |tmp| std::fs::write(tmp, &bytes))?;
                Ok(vec![o])
            }
            "TXT" => {
                let text = crate::docx::read_text(p)?;
                let o = crate::out::sibling(p, "", Some("txt"));
                crate::out::atomic_write(&o, |tmp| std::fs::write(tmp, text.as_bytes()))?;
                Ok(vec![o])
            }
            _ => refuse!("Can't convert Word to {t}"),
        }),
        Kind::Text => each(files, |p| {
            let text = crate::docx::read_any_text(p)?;
            match t {
                "PDF" => {
                    let bytes = crate::pdfgen::text_pdf(&text)?;
                    let o = crate::out::sibling(p, "", Some("pdf"));
                    crate::out::atomic_write(&o, |tmp| std::fs::write(tmp, &bytes))?;
                    Ok(vec![o])
                }
                "DOCX" => {
                    use crate::docx::{Item, Run, Style};
                    let items: Vec<Item> = text.lines().map(|l| Item::Para { style: Style::Normal, runs: vec![Run { text: l.to_string(), ..Default::default() }] }).collect();
                    let o = crate::out::sibling(p, "", Some("docx"));
                    crate::docx::write_docx(&o, &items)?;
                    Ok(vec![o])
                }
                _ => refuse!("Can't convert text to {t}"),
            }
        }),
        _ => refuse!("Can't convert {} files", kind.name()),
    }
}

fn reply(title: impl Into<String>, detail: impl Into<String>, outs: Vec<PathBuf>, srcs: &[PathBuf]) -> Report {
    Report::new(title, detail, outs, srcs.to_vec())
}

fn more_plan(id: &str, files: Vec<PathBuf>, everything: Vec<PathBuf>) -> Result<Plan> {
    let id = id.to_string();
    Ok(match id.as_str() {
        "copy" => job("x:copy", "Copying…", move || {
            let im = imgio::open_image(&files[0])?.to_rgba8();
            let extra = if files.len() > 1 { " (first file)" } else { "" };
            Ok(reply("Image copied", format!("{}{extra}", name(&files[0])), vec![], &files).clip(Clip::Image(im)))
        }),
        "ocr" => job("x:ocr", "Reading text…", move || {
            let (out, text) = crate::ocr::ocr_to_file(&files[0], &crate::options().ocr_lang)?;
            let words = text.split_whitespace().count();
            Ok(reply("Text copied", format!("{} · {words} word{}", name(&out), if words != 1 { "s" } else { "" }), vec![out], &files).clip(Clip::Text(text)))
        }),
        "copy-path" => job("x:copy-path", "Copying…", move || {
            let n = everything.len();
            let text = everything.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\r\n");
            Ok(reply(if n == 1 { "Path copied".into() } else { format!("{n} paths copied") }, if n == 1 { name(&everything[0]) } else { String::new() }, vec![], &everything).clip(Clip::Text(text)))
        }),
        "numbers" | "gray" | "flatten" | "halves" | "chapters" => {
            let (label, working, f): (&str, &str, fn(&Path) -> Result<Vec<PathBuf>>) = match id.as_str() {
                "numbers" => ("Page numbers added", "Numbering…", |p| Ok(vec![crate::pdf::number_pages(p)?])),
                "gray" => ("Converted to grayscale", "Converting…", |p| Ok(vec![crate::pdf::grayscale(p)?])),
                "flatten" => ("Flattened", "Flattening…", |p| Ok(vec![crate::pdf::flatten(p)?])),
                "halves" => ("Split", "Splitting…", crate::pdf::split_halves),
                _ => ("Split", "Splitting…", crate::pdf::split_chapters),
            };
            let action = format!("tool:{label}");
            job(action.clone(), working, move || {
                let outs = each(&files, f)?;
                Ok(summarize(&action, &files, &outs, ""))
            })
        }
        "searchable" => job("x:searchable", "Recognising text…", move || {
            for f in &files {
                crate::pdf::check_needs_ocr(f)?;
            }
            let lang = crate::options().ocr_lang;
            let mut outs = vec![];
            let mut done = 0;
            for f in &files {
                let (o, n) = crate::pdf::make_searchable(f, &lang)?;
                outs.push(o);
                done += n;
            }
            let title = if outs.len() == 1 { "Searchable PDF".to_string() } else { format!("{} searchable PDFs", outs.len()) };
            Ok(reply(title, format!("{} · {done} page{} recognised", name(&outs[0]), if done != 1 { "s" } else { "" }), outs, &files))
        }),
        "compress" => job("tool:Compress", "Compressing…", move || {
            let outs = each(&files, |p| Ok(vec![crate::media::compress_video(p)?]))?;
            Ok(summarize("tool:Compress", &files, &outs, ""))
        }),
        "extract" => job("x:extract", "Extracting…", move || {
            let mut outs = vec![];
            let mut count = 0;
            for f in &files {
                let (dest, n) = crate::archive::unpack(f)?;
                crate::out::remember_dir(&dest);
                outs.push(dest);
                count += n;
            }
            let title = if outs.len() == 1 { "Extracted".to_string() } else { format!("Extracted {} archives", outs.len()) };
            let detail = format!("{} · {count} item{} · {}", name(&outs[0]), if count != 1 { "s" } else { "" }, human_size(tree_size(&outs)));
            Ok(reply(title, detail, outs, &files))
        }),
        "batch-webp" | "batch-jpg" | "batch-compress" => {
            let mode = match id.as_str() {
                "batch-webp" => "WEBP",
                "batch-jpg" => "JPG",
                _ => "Compressed",
            };
            job(format!("x:{id}"), "Converting images…", move || batch(&files, mode))
        }
        "zip" | "tar-gz" | "tar-xz" => job(format!("x:{id}"), "Packing…", move || {
            let (dest, _, skipped) = crate::archive::pack(&everything, &id)?;
            let (before, after) = (human_size(tree_size(&everything)), human_size(crate::out::file_size(&dest)));
            let what = if everything.len() == 1 { name(&everything[0]) } else { format!("{} items", everything.len()) };
            let mut detail = format!("{} · {before} → {after}", name(&dest));
            if skipped > 0 {
                detail.push_str(&format!(" · {skipped} link{} left out", if skipped != 1 { "s" } else { "" }));
            }
            Ok(reply(format!("Archived {what}"), detail, vec![dest], &everything))
        }),
        "translate" => job("x:translate", "Reading text…", move || {
            let text = files.iter().map(|f| crate::oumu::text_of(f)).collect::<Result<Vec<_>>>()?.join("\n\n");
            let (text, cut) = crate::oumu::clip(&text, crate::oumu::MAX_BYTES);
            crate::oumu::send(&text)?;
            let what = if files.len() == 1 { name(&files[0]) } else { format!("{} files", files.len()) };
            Ok(reply("Sent to Oumu", format!("{what}{} · translating", if cut { " · the first part" } else { "" }), vec![], &files))
        }),
        other => refuse!("Unknown action '{other}'"),
    })
}

/// Images anywhere under `folder`, skipping hidden folders and links.
fn images_in(folder: &Path) -> Vec<PathBuf> {
    let mut found = vec![];
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut kids: Vec<PathBuf> = std::fs::read_dir(&dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        kids.sort();
        let mut subs = vec![];
        for k in kids {
            let hidden = k.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(true);
            let Ok(m) = std::fs::symlink_metadata(&k) else { continue };
            if hidden || m.file_type().is_symlink() {
                continue;
            }
            if m.is_dir() {
                subs.push(k);
            } else if crate::kinds::category(&k) == Some(Kind::Image) {
                found.push(k);
            }
        }
        stack.extend(subs.into_iter().rev());
    }
    found
}

/// Every image under each folder -> a new "<folder> <mode>" folder beside it, keeping
/// the subfolder layout. The originals are only read.
pub fn batch(folders: &[PathBuf], mode: &str) -> Result<Report> {
    let mut outs = vec![];
    let (mut done, mut failed, mut first_err) = (0usize, 0usize, String::new());
    for folder in folders {
        let mut images = images_in(folder);
        if mode != "Compressed" {
            images.retain(|p| imgio::targets_for(p).contains(&mode));
        }
        if images.is_empty() {
            continue;
        }
        if images.len() > crate::kinds::MAX_FILES {
            refuse!("{} has {} images; the limit is {}", name(folder), images.len(), crate::kinds::MAX_FILES);
        }
        let root = crate::out::unique_path(&crate::out::output_dir(folder).join(format!("{} {mode}", name(folder))));
        std::fs::create_dir_all(&root)?;
        crate::out::remember_dir(&root);
        outs.push(root.clone());
        let (base, r2) = (folder.clone(), root.clone());
        crate::out::with_redirect(
            move |src| {
                let rel = src.parent().and_then(|p| p.strip_prefix(&base).ok()).map(Path::to_path_buf).unwrap_or_default();
                let d = r2.join(rel);
                let _ = std::fs::create_dir_all(&d);
                d
            },
            || {
                for img in &images {
                    let r = if mode == "Compressed" { crate::imaging::compress_image(img, 72, None, false) } else { imgio::image_to(img, mode) };
                    match r {
                        Ok(_) => done += 1,
                        Err(e) => {
                            failed += 1;
                            if first_err.is_empty() {
                                first_err = format!("{}: {}", name(img), crate::friendly(&e));
                            }
                        }
                    }
                }
            },
        );
    }
    if outs.is_empty() {
        refuse!("{}", if mode != "Compressed" { format!("No images to convert in {}", name(&folders[0])) } else { format!("No images in {}", name(&folders[0])) });
    }
    if done == 0 {
        for r in &outs {
            let _ = std::fs::remove_dir_all(r);
        }
        refuse!("Every image failed: {first_err}");
    }
    let title = format!("{done} image{} → {}", if done != 1 { "s" } else { "" }, if mode == "Compressed" { "compressed" } else { mode });
    let mut detail = format!("{} · {}", name(&outs[0]), human_size(tree_size(&outs)));
    if failed > 0 {
        detail.push_str(&format!(" · {failed} failed ({first_err})"));
    }
    Ok(Report::new(title, detail, outs, folders.to_vec()))
}

/// Run a job plan right here (tests, the CLI).
pub fn run_now(key: &str, paths: &[PathBuf]) -> Result<Report> {
    match plan(key, paths)? {
        Plan::Job { run, .. } => run(),
        Plan::Window { tool, .. } => refuse!("{tool:?} opens a window"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{photo, tmpdir};

    fn labels(r: &[(String, Vec<Slice>)]) -> Vec<(String, Vec<String>)> {
        r.iter().map(|(n, s)| (n.clone(), s.iter().map(|x| x.label.clone()).collect())).collect()
    }

    #[test]
    fn rings_per_kind() {
        let d = tmpdir("rings");
        let png = d.join("a.png");
        let pdf = d.join("b.pdf");
        let pdf2 = d.join("c.pdf");
        photo(10, 10).save(&png).unwrap();
        crate::pdf::tests::sample(&pdf, 1, 0);
        crate::pdf::tests::sample(&pdf2, 1, 0);
        let r = labels(&rings_for(&sort_drop(std::slice::from_ref(&png)), true, true));
        assert_eq!(r[0].0, "Convert");
        assert!(!r[0].1.contains(&"PNG".to_string()));
        assert_eq!(r[1], ("Tools".into(), IMAGE_TOOLS.iter().map(|s| s.to_string()).collect()));
        assert_eq!(r[2], ("More".into(), vec!["Copy".into(), "Text".into(), "Translate".into()]));
        assert_eq!(r[3].0, "Pack");
        let r = labels(&rings_for(&sort_drop(&[pdf.clone(), pdf2.clone()]), true, false));
        assert_eq!(r[1].1, vec!["Compress", "Split", "Rotate", "Merge", "Protect", "Extract", "Metadata"]);
        // 9 extra PDF slices spread over two rings, 5 + 4
        assert_eq!(r[2].1.len(), 5);
        assert_eq!(r[3].0, "More 2");
        assert_eq!(r[3].1.len(), 4);
        let folder = labels(&rings_for(&sort_drop(std::slice::from_ref(&d)), false, false));
        assert_eq!(folder[0], ("More".into(), vec!["→ WEBP".into(), "→ JPG".into(), "Compress".into()]));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn jobs_run_end_to_end() {
        let d = tmpdir("jobs");
        let png = d.join("a.png");
        photo(40, 30).save(&png).unwrap();
        let r = run_now("convert:JPG", std::slice::from_ref(&png)).unwrap();
        assert_eq!(r.title, "PNG → JPG");
        assert!(d.join("a.jpg").exists());
        let r = run_now("x:copy", std::slice::from_ref(&png)).unwrap();
        assert!(matches!(r.clip, Some(Clip::Image(_))));
        let r = run_now("x:copy-path", std::slice::from_ref(&png)).unwrap();
        assert!(matches!(&r.clip, Some(Clip::Text(t)) if t.ends_with("a.png")));
        let r = run_now("x:zip", std::slice::from_ref(&png)).unwrap();
        assert_eq!(r.outs[0].file_name().unwrap(), "a.zip");
        let r = run_now("x:extract", &r.outs).unwrap();
        assert!(r.outs[0].join("a.png").exists());
        let txt = d.join("n.txt");
        std::fs::write(&txt, "hello\nworld").unwrap();
        let r = run_now("convert:DOCX", std::slice::from_ref(&txt)).unwrap();
        let back = run_now("convert:PDF", &r.outs).unwrap();
        assert!(crate::pdf::extract_text(&back.outs[0]).unwrap().contains("world"));
        assert!(matches!(plan("tool:Crop", std::slice::from_ref(&png)).unwrap(), Plan::Window { tool: Tool::Crop, .. }));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn batch_mirrors_subfolders() {
        let d = tmpdir("batch");
        let f = d.join("pics");
        std::fs::create_dir_all(f.join("sub")).unwrap();
        photo(20, 20).save(f.join("a.png")).unwrap();
        photo(20, 20).save(f.join("sub/b.png")).unwrap();
        std::fs::write(f.join("notes.txt"), "x").unwrap();
        let r = batch(std::slice::from_ref(&f), "WEBP").unwrap();
        assert_eq!(r.title, "2 images → WEBP");
        let root = d.join("pics WEBP");
        assert!(root.join("a.webp").exists() && root.join("sub/b.webp").exists());
        assert!(!f.join("a.webp").exists(), "originals' folder untouched");
        let _ = std::fs::remove_dir_all(&d);
    }
}
