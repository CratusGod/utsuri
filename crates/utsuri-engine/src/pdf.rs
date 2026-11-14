//! PDF tools on lopdf (structure) + pdf_oxide (text) + Windows.Data.Pdf (rendering).
//! Ported from imaging.py (split, merge, rotate, protect, extract images, compress,
//! metadata) and pdfkit.py (numbers, grayscale, flatten, halves, chapters, searchable OCR,
//! organize, watermark, stamp, unlock).

use crate::out::{atomic_write, sibling};
use crate::refuse;
use anyhow::{Context as _, Result};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

pub const OCR_DPI: u32 = 300;
pub const NUMBER_PT: f32 = 12.0;
pub const CORNERS: &[&str] = &["center", "top-left", "top-right", "bottom-left", "bottom-right"];

fn name_of(p: &Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

fn out(src: &Path, label: &str) -> PathBuf {
    sibling(src, label, Some("pdf"))
}

// ---------------------------------------------------------------- open / save

/// Open, refusing password-protected files (the password tools use `open_raw`).
pub fn open(src: &Path) -> Result<Document> {
    open_with(src, None)
}

/// Open with a password (None = no password). An owner-only password (no user password)
/// opens fine.
pub fn open_with(src: &Path, password: Option<&str>) -> Result<Document> {
    let bytes = std::fs::read(src)?;
    let r = match password {
        Some(pw) => load_pw(&bytes, pw),
        None => Document::load_mem(&bytes),
    };
    match r {
        Ok(d) if d.is_encrypted() => refuse!("This PDF is password-protected. Remove the password first (More → Unlock)."),
        Ok(d) => Ok(d),
        Err(e) if is_password_error(&e) => {
            if password.is_some() {
                refuse!("Wrong password");
            }
            refuse!("This PDF is password-protected. Remove the password first (More → Unlock).")
        }
        Err(_) => Err(bad(src)),
    }
}

fn is_password_error(e: &lopdf::Error) -> bool {
    matches!(e, lopdf::Error::Decryption(_) | lopdf::Error::InvalidPassword) || format!("{e:?}").to_lowercase().contains("password")
}

/// Open without needing the password to be known up front (structure only when a user
/// password is set is not possible with lopdf, so this is `open`).
pub fn open_raw(src: &Path) -> Result<Document> {
    open(src)
}

fn bad(src: &Path) -> anyhow::Error {
    crate::UserError(format!("Can't read {}: it isn't a valid PDF", name_of(src))).into()
}

/// Save compressed, atomically.
pub fn save(doc: &mut Document, out: &Path) -> Result<PathBuf> {
    doc.prune_objects();
    doc.compress();
    atomic_write(out, |tmp| {
        let mut f = std::io::BufWriter::new(std::fs::File::create(tmp)?);
        let opts = lopdf::SaveOptions::builder().use_object_streams(true).use_xref_streams(true).build();
        doc.save_with_options(&mut f, opts).map_err(std::io::Error::other)?;
        use std::io::Write;
        f.flush()
    })
    .with_context(|| format!("couldn't save {}", out.display()))?;
    Ok(out.to_path_buf())
}

pub fn page_count(src: &Path) -> Result<usize> {
    Ok(open_raw(src)?.get_pages().len())
}

// ---------------------------------------------------------------- text (pdf_oxide)

fn oxide(src: &Path) -> Result<pdf_oxide::PdfDocument> {
    pdf_oxide::PdfDocument::open(src).map_err(|_| bad(src))
}

/// Each page's text.
pub fn page_texts(src: &Path) -> Result<Vec<String>> {
    let d = oxide(src)?;
    let n = d.page_count().map_err(|_| bad(src))?;
    Ok((0..n).map(|i| d.extract_text(i).unwrap_or_default()).collect())
}

pub fn extract_text(src: &Path) -> Result<String> {
    Ok(page_texts(src)?.join("\n\u{c}\n"))
}

/// A line of text with its position (top-left origin), size and weight.
#[derive(Clone, Debug)]
pub struct Span {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub size: f32,
    pub bold: bool,
}

pub fn page_spans(src: &Path, page: usize) -> Result<Vec<Span>> {
    let d = oxide(src)?;
    Ok(d.extract_spans(page)
        .unwrap_or_default()
        .into_iter()
        .map(|s| Span {
            bold: (s.font_weight as u32) >= 600 || s.font_name.contains("Bold"),
            text: s.text,
            x: s.bbox.x,
            y: s.bbox.y,
            w: s.bbox.width,
            h: s.bbox.height,
            size: s.font_size,
        })
        .collect())
}

// ---------------------------------------------------------------- page geometry

/// A page as the reader sees it: crop box, rotation, size after rotation.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub crop: [f32; 4],
    pub rot: i64,
    pub w: f32,
    pub h: f32,
}

fn num(o: &Object) -> f32 {
    o.as_float().or_else(|_| o.as_i64().map(|i| i as f32)).unwrap_or(0.0)
}

fn inherited(doc: &Document, page: ObjectId, key: &[u8]) -> Option<Object> {
    let mut cur = page;
    let mut seen = HashSet::new();
    while seen.insert(cur) {
        let d = doc.get_dictionary(cur).ok()?;
        if let Ok(v) = d.get(key) {
            if !matches!(v, Object::Null) {
                return Some(v.clone());
            }
        }
        cur = d.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn rect_of(doc: &Document, o: Option<Object>) -> Option<[f32; 4]> {
    let o = o?;
    let arr = match o {
        Object::Reference(r) => doc.get_object(r).ok()?.as_array().ok()?.clone(),
        Object::Array(a) => a,
        _ => return None,
    };
    if arr.len() != 4 {
        return None;
    }
    let v: Vec<f32> = arr.iter().map(num).collect();
    Some([v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
}

pub fn view(doc: &Document, page: ObjectId) -> View {
    let media = rect_of(doc, inherited(doc, page, b"MediaBox")).unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let crop = rect_of(doc, inherited(doc, page, b"CropBox")).unwrap_or(media);
    let rot = inherited(doc, page, b"Rotate").and_then(|o| o.as_i64().ok()).unwrap_or(0).rem_euclid(360) / 90 * 90;
    let (cw, ch) = (crop[2] - crop[0], crop[3] - crop[1]);
    let (w, h) = if rot % 180 == 0 { (cw, ch) } else { (ch, cw) };
    View { crop, rot, w, h }
}

impl View {
    /// Reader point (top-left origin) -> PDF user space.
    pub fn to_pdf(&self, u: f32, v: f32) -> (f32, f32) {
        let [x0, y0, x1, y1] = self.crop;
        match self.rot {
            90 => (x0 + v, y0 + u),
            180 => (x1 - u, y0 + v),
            270 => (x1 - v, y1 - u),
            _ => (x0 + u, y1 - v),
        }
    }

    /// `cm` matrix that maps the unit square (y up) upright onto reader rect (u, v, w, h).
    pub fn matrix_for(&self, u: f32, v: f32, w: f32, h: f32) -> [f32; 6] {
        let p0 = self.to_pdf(u, v + h); // image bottom-left
        let p1 = self.to_pdf(u + w, v + h); // bottom-right
        let p2 = self.to_pdf(u, v); // top-left
        [p1.0 - p0.0, p1.1 - p0.1, p2.0 - p0.0, p2.1 - p0.1, p0.0, p0.1]
    }
}

/// Where a w×h box goes on the reader's page.
pub fn anchor(v: &View, w: f32, h: f32, where_: &str, margin: f32) -> (f32, f32) {
    match where_ {
        "top-left" => (margin, margin),
        "top-right" => (v.w - margin - w, margin),
        "bottom-left" => (margin, v.h - margin - h),
        "bottom-right" => (v.w - margin - w, v.h - margin - h),
        _ => ((v.w - w) / 2.0, (v.h - h) / 2.0),
    }
}

// ---------------------------------------------------------------- page ranges

/// "1-3, 7, 10-" -> zero-based indexes, sorted, no repeats. Empty or "all" = every page.
pub fn parse_ranges(text: &str, count: usize) -> Result<Vec<usize>> {
    let t = text.trim().to_lowercase();
    if t.is_empty() || t == "all" {
        return Ok((0..count).collect());
    }
    let mut pages = std::collections::BTreeSet::new();
    for part in t.split(',') {
        let p = part.trim();
        let (a, dash, b) = match p.split_once('-') {
            Some((a, b)) => (a.trim(), true, b.trim()),
            None => (p, false, ""),
        };
        let ok = |s: &str| s.is_empty() || s.chars().all(|c| c.is_ascii_digit());
        if !ok(a) || !ok(b) || (a.is_empty() && b.is_empty()) {
            refuse!("'{p}' isn't a page range (use e.g. 1-3, 7, 10-)");
        }
        let a1: usize = if a.is_empty() { 1 } else { a.parse()? };
        let b1: usize = if dash { if b.is_empty() { count } else { b.parse()? } } else { a1 };
        if a1 < 1 || b1 > count || a1 > b1 {
            refuse!("Pages {p} are outside 1-{count}");
        }
        pages.extend(a1 - 1..b1);
    }
    Ok(pages.into_iter().collect())
}

/// [0, 1, 2, 6] -> "pages 1-3, 7" (for file names).
pub fn label_for(pages: &[usize]) -> String {
    let mut runs = vec![];
    let mut start = None;
    for (i, &p) in pages.iter().enumerate() {
        let s = *start.get_or_insert(p);
        if i + 1 == pages.len() || pages[i + 1] != p + 1 {
            runs.push(if s == p { format!("{}", s + 1) } else { format!("{}-{}", s + 1, p + 1) });
            start = None;
        }
    }
    let s = runs.join(", ");
    let s = if s.chars().count() <= 40 { s } else { format!("{}…", s.chars().take(37).collect::<String>()) };
    format!("{}{s}", if pages.len() == 1 { "page " } else { "pages " })
}

// ---------------------------------------------------------------- assembling new documents

/// A page in a new document: (source doc index, page index or None = blank, extra rotation).
pub type PageRef = (usize, Option<usize>, i64);

/// Build a new document from pages of several sources, in order. Pages may repeat.
pub fn assemble(sources: &[Document], items: &[PageRef]) -> Result<Document> {
    if items.is_empty() {
        refuse!("No pages left to save");
    }
    let mut out = Document::with_version("1.7");
    let pages_root = out.new_object_id();
    let mut kids = vec![];
    // per source: renumbered copy and its page ids
    let mut copies: Vec<Option<(Document, Vec<ObjectId>)>> = (0..sources.len()).map(|_| None).collect();
    let mut last_size = (595.0f32, 842.0f32);
    let mut used: HashSet<ObjectId> = HashSet::new();
    for &(si, pno, rot) in items {
        let page_id = match pno {
            None => {
                let content = out.add_object(Stream::new(dictionary! {}, vec![]));
                out.add_object(dictionary! {
                    "Type" => "Page",
                    "MediaBox" => vec![0.into(), 0.into(), last_size.0.into(), last_size.1.into()],
                    "Resources" => dictionary! {},
                    "Contents" => content,
                })
            }
            Some(p) => {
                if copies[si].is_none() {
                    let mut d = sources[si].clone();
                    // bake inherited attributes into each page before cutting it loose
                    for pid in d.get_pages().into_values() {
                        let mut pd = d.get_dictionary(pid)?.clone();
                        for key in [&b"MediaBox"[..], b"CropBox", b"Resources", b"Rotate"] {
                            if pd.get(key).is_err() {
                                if let Some(v) = inherited(&d, pid, key) {
                                    pd.set(key, v);
                                }
                            }
                        }
                        d.objects.insert(pid, Object::Dictionary(pd));
                    }
                    d.renumber_objects_with(out.max_id + 1);
                    let ids: Vec<ObjectId> = d.get_pages().into_values().collect();
                    out.max_id = d.max_id;
                    for (id, obj) in &d.objects {
                        out.objects.insert(*id, obj.clone());
                    }
                    copies[si] = Some((d, ids));
                }
                let (_, ids) = copies[si].as_ref().unwrap();
                let Some(&pid) = ids.get(p) else { refuse!("Page {} doesn't exist", p + 1) };
                let v = view(&out, pid);
                last_size = (v.crop[2] - v.crop[0], v.crop[3] - v.crop[1]);
                if used.insert(pid) {
                    pid
                } else {
                    // the same page twice: a new page object sharing its content
                    let d = out.get_dictionary(pid)?.clone();
                    out.add_object(d)
                }
            }
        };
        let pd = out.get_object_mut(page_id)?.as_dict_mut()?;
        pd.set("Parent", pages_root);
        pd.remove(b"Annots").filter(|_| false); // keep annotations
        if rot != 0 {
            let cur = pd.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0);
            pd.set("Rotate", (cur + rot).rem_euclid(360));
        }
        kids.push(Object::Reference(page_id));
    }
    let count = kids.len() as i64;
    out.objects.insert(pages_root, Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => kids, "Count" => count}));
    let catalog = out.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_root});
    out.trailer.set("Root", catalog);
    // drop the sources' own catalogs, outlines and page trees
    out.prune_objects();
    Ok(out)
}

fn extract_to(doc: &Document, pages: &[usize], dest: &Path) -> Result<PathBuf> {
    let items: Vec<PageRef> = pages.iter().map(|&p| (0, Some(p), 0)).collect();
    let mut nd = assemble(std::slice::from_ref(doc), &items)?;
    save(&mut nd, dest)
}

// ---------------------------------------------------------------- one-click tools

pub fn split_pdf(src: &Path) -> Result<Vec<PathBuf>> {
    let doc = open(src)?;
    let n = doc.get_pages().len();
    (0..n).map(|i| extract_to(&doc, &[i], &out(src, &format!("page {}", i + 1)))).collect()
}

pub fn merge_pdfs(srcs: &[PathBuf]) -> Result<PathBuf> {
    let docs = srcs.iter().map(|s| open(s)).collect::<Result<Vec<_>>>()?;
    let items: Vec<PageRef> = docs.iter().enumerate().flat_map(|(i, d)| (0..d.get_pages().len()).map(move |p| (i, Some(p), 0))).collect();
    let mut nd = assemble(&docs, &items)?;
    save(&mut nd, &out(&srcs[0], "Merged"))
}

pub fn rotate_pdf(src: &Path, degrees: i64) -> Result<PathBuf> {
    let mut doc = open(src)?;
    for pid in doc.get_pages().into_values() {
        let cur = inherited(&doc, pid, b"Rotate").and_then(|o| o.as_i64().ok()).unwrap_or(0);
        doc.get_object_mut(pid)?.as_dict_mut()?.set("Rotate", (cur + degrees).rem_euclid(360));
    }
    save(&mut doc, &out(src, "Rotated"))
}

pub fn split_halves(src: &Path) -> Result<Vec<PathBuf>> {
    let doc = open(src)?;
    let n = doc.get_pages().len();
    if n < 2 {
        refuse!("A one-page PDF can't be split in half");
    }
    let half = n.div_ceil(2);
    Ok(vec![
        extract_to(&doc, &(0..half).collect::<Vec<_>>(), &out(src, "part 1"))?,
        extract_to(&doc, &(half..n).collect::<Vec<_>>(), &out(src, "part 2"))?,
    ])
}

fn safe_title(t: &str) -> String {
    let cleaned: String = t.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { ' ' } else { c }).collect();
    let s = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim_matches(|c| c == ' ' || c == '.').chars().take(60).collect::<String>();
    if s.is_empty() { "chapter".into() } else { s }
}

pub fn split_chapters(src: &Path) -> Result<Vec<PathBuf>> {
    let doc = open(src)?;
    let n = doc.get_pages().len();
    let toc = doc.get_toc().map(|t| t.toc).unwrap_or_default();
    let mut tops: Vec<(String, usize)> = toc.iter().filter(|t| t.level == 1 && t.page >= 1).map(|t| (t.title.clone(), t.page.max(1))).collect();
    if tops.is_empty() {
        refuse!("This PDF has no bookmarks to split by");
    }
    tops.sort_by_key(|t| t.1);
    if tops[0].1 > 1 {
        tops.insert(0, ("Front matter".into(), 1));
    }
    let mut outs = vec![];
    let mut seen = HashSet::new();
    for (i, (title, start)) in tops.iter().enumerate() {
        if !seen.insert(*start) {
            continue; // two bookmarks on one page: the first names it
        }
        let next = tops[i + 1..].iter().map(|t| t.1).find(|p| p > start).unwrap_or(n + 1);
        let pages: Vec<usize> = (start - 1..(next - 1).min(n)).collect();
        if pages.is_empty() {
            continue;
        }
        outs.push(extract_to(&doc, &pages, &out(src, &format!("{:02} {}", outs.len() + 1, safe_title(title))))?);
    }
    Ok(outs)
}

/// Build PDFs from organize items. `cuts` are item positions where a new file starts.
pub fn organize(srcs: &[PathBuf], items: &[PageRef], label: &str, cuts: &[usize]) -> Result<Vec<PathBuf>> {
    if items.is_empty() {
        refuse!("No pages left to save");
    }
    let docs = srcs.iter().map(|s| open(s)).collect::<Result<Vec<_>>>()?;
    let mut bounds: Vec<usize> = cuts.iter().copied().filter(|c| *c > 0 && *c < items.len()).collect();
    bounds.extend([0, items.len()]);
    bounds.sort();
    bounds.dedup();
    let groups: Vec<&[PageRef]> = bounds.windows(2).map(|w| &items[w[0]..w[1]]).collect();
    let mut outs = vec![];
    for (k, g) in groups.iter().enumerate() {
        let mut nd = assemble(&docs, g)?;
        let name = if groups.len() > 1 { format!("{label} {}", k + 1) } else { label.to_string() };
        outs.push(save(&mut nd, &out(&srcs[0], &name))?);
    }
    Ok(outs)
}

// ---------------------------------------------------------------- password

pub fn is_encrypted(src: &Path) -> Result<bool> {
    let b = std::fs::read(src)?;
    Ok(match Document::load_mem(&b) {
        Ok(d) => d.is_encrypted() || d.was_encrypted(),
        Err(e) => is_password_error(&e) || find(&b, b"/Encrypt"),
    })
}

fn find(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

pub fn needs_password(src: &Path) -> bool {
    match std::fs::read(src).map(|b| Document::load_mem(&b)) {
        Ok(Ok(d)) => d.is_encrypted(),
        Ok(Err(e)) => is_password_error(&e),
        Err(_) => false,
    }
}

pub fn password_ok(src: &Path, pw: &str) -> bool {
    std::fs::read(src).ok().is_some_and(|b| load_pw(&b, pw).is_ok())
}

/// AES-256 with the same password for opening and owning; printing and copying allowed.
pub fn protect_pdf(src: &Path, password: &str) -> Result<PathBuf> {
    if password.is_empty() {
        refuse!("Type a password first");
    }
    let mut doc = open(src)?;
    doc.prune_objects();
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
    let aes = std::sync::Arc::new(lopdf::encryption::crypt_filters::Aes256CryptFilter) as std::sync::Arc<dyn lopdf::encryption::crypt_filters::CryptFilter>;
    let perms = lopdf::Permissions::PRINTABLE | lopdf::Permissions::COPYABLE | lopdf::Permissions::COPYABLE_FOR_ACCESSIBILITY | lopdf::Permissions::PRINTABLE_IN_HIGH_QUALITY;
    let version = lopdf::EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), aes)]),
        file_encryption_key: &key,
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: password,
        user_password: password,
        permissions: perms,
    };
    let state = lopdf::EncryptionState::try_from(version)?;
    doc.compress();
    doc.encrypt(&state)?;
    let dest = out(src, "Protected");
    atomic_write(&dest, |tmp| doc.save(tmp).map(|_| ()).map_err(std::io::Error::other))?;
    Ok(dest)
}

pub fn unlock(src: &Path, password: &str) -> Result<PathBuf> {
    if !is_encrypted(src)? {
        refuse!("This PDF has no password");
    }
    let bytes = std::fs::read(src)?;
    let mut doc = match load_pw(&bytes, password) {
        Ok(d) => d,
        Err(_) if !needs_password(src) => Document::load_mem(&bytes).map_err(|_| bad(src))?,
        Err(_) => refuse!("Wrong password"),
    };
    doc.trailer.remove(b"Encrypt");
    doc.encryption_state = None;
    save(&mut doc, &out(src, "Unlocked"))
}

// ---------------------------------------------------------------- metadata

pub fn read_pdf_metadata(src: &Path) -> Result<Vec<(String, String)>> {
    let doc = open(src)?;
    let mut out = vec![];
    if let Ok(info) = doc.trailer.get(b"Info").and_then(|o| doc.dereference(o)).and_then(|(_, o)| o.as_dict().cloned()) {
        for (k, v) in info.iter() {
            let val = match v {
                Object::String(s, _) => lopdf::decode_text_string(v).unwrap_or_else(|_| String::from_utf8_lossy(s).into_owned()),
                other => format!("{other:?}"),
            };
            if !val.is_empty() {
                out.push((String::from_utf8_lossy(k).into_owned(), val));
            }
        }
    }
    out.push(("Pages".into(), doc.get_pages().len().to_string()));
    if doc.catalog().map(|c| c.has(b"Metadata")).unwrap_or(false) {
        out.push(("XMP".into(), "present".into()));
    }
    Ok(out)
}

pub fn strip_pdf_metadata(src: &Path) -> Result<PathBuf> {
    let mut doc = open(src)?;
    doc.trailer.remove(b"Info");
    if let Ok(cat) = doc.catalog_mut() {
        cat.remove(b"Metadata");
    }
    save(&mut doc, &out(src, "Clean"))
}

// ---------------------------------------------------------------- drawing on pages

/// The page's resource dictionary, made local and mutable.
fn resources_mut(doc: &mut Document, page: ObjectId) -> Result<&mut Dictionary> {
    let res = doc.get_dictionary(page)?.get(b"Resources").ok().cloned().or_else(|| inherited(doc, page, b"Resources"));
    let id = match res {
        Some(Object::Reference(r)) => r,
        Some(Object::Dictionary(d)) => {
            let id = doc.add_object(d);
            doc.get_object_mut(page)?.as_dict_mut()?.set("Resources", id);
            id
        }
        _ => {
            let id = doc.add_object(Dictionary::new());
            doc.get_object_mut(page)?.as_dict_mut()?.set("Resources", id);
            id
        }
    };
    Ok(doc.get_object_mut(id)?.as_dict_mut()?)
}

/// Add `name -> obj` to the page's /XObject (or /Font, /ExtGState) sub-dictionary.
fn add_resource(doc: &mut Document, page: ObjectId, kind: &str, name: &str, obj: ObjectId) -> Result<()> {
    let sub = resources_mut(doc, page)?.get(kind.as_bytes()).ok().cloned();
    match sub {
        Some(Object::Reference(r)) => {
            doc.get_object_mut(r)?.as_dict_mut()?.set(name, obj);
        }
        Some(Object::Dictionary(mut d)) => {
            d.set(name, obj);
            resources_mut(doc, page)?.set(kind, d);
        }
        _ => {
            resources_mut(doc, page)?.set(kind, dictionary! { name => obj });
        }
    }
    Ok(())
}

/// Draw `ops` over the page, isolated from its own graphics state.
fn overlay(doc: &mut Document, page: ObjectId, ops: Vec<Operation>) -> Result<()> {
    let mut all = vec![Operation::new("Q", vec![]), Operation::new("q", vec![])];
    all.extend(ops);
    all.push(Operation::new("Q", vec![]));
    let before = doc.add_object(Stream::new(dictionary! {}, b"q\n".to_vec()));
    let after = doc.add_object(Stream::new(dictionary! {}, Content { operations: all }.encode()?));
    let pd = doc.get_object_mut(page)?.as_dict_mut()?;
    let mut contents: Vec<Object> = match pd.get(b"Contents") {
        Ok(Object::Array(a)) => a.clone(),
        Ok(o @ Object::Reference(_)) => vec![o.clone()],
        _ => vec![],
    };
    contents.insert(0, Object::Reference(before));
    contents.push(Object::Reference(after));
    pd.set("Contents", contents);
    Ok(())
}

fn f(v: f32) -> Object {
    Object::Real(v)
}

fn cm(m: [f32; 6]) -> Operation {
    Operation::new("cm", m.iter().map(|v| f(*v)).collect())
}

/// An RGBA image as a PDF image XObject (with a soft mask when it has transparency).
pub fn image_xobject(doc: &mut Document, im: &image::RgbaImage) -> ObjectId {
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;
    let z = |data: &[u8]| {
        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    };
    let (w, h) = im.dimensions();
    let rgb: Vec<u8> = im.pixels().flat_map(|p| [p.0[0], p.0[1], p.0[2]]).collect();
    let mut dict = dictionary! {
        "Type" => "XObject", "Subtype" => "Image", "Width" => w as i64, "Height" => h as i64,
        "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "FlateDecode",
    };
    if im.pixels().any(|p| p.0[3] < 255) {
        let alpha: Vec<u8> = im.pixels().map(|p| p.0[3]).collect();
        let smask = doc.add_object(Stream::new(
            dictionary! {"Type" => "XObject", "Subtype" => "Image", "Width" => w as i64, "Height" => h as i64, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8, "Filter" => "FlateDecode"},
            z(&alpha),
        ));
        dict.set("SMask", smask);
    }
    let mut s = Stream::new(dict, z(&rgb));
    s.allows_compression = false;
    doc.add_object(s)
}

/// Place an image XObject upright at reader rect (u, v, w, h).
fn draw_image(doc: &mut Document, page: ObjectId, xobj: ObjectId, rect: (f32, f32, f32, f32), name: &str) -> Result<()> {
    let v = view(doc, page);
    add_resource(doc, page, "XObject", name, xobj)?;
    overlay(doc, page, vec![cm(v.matrix_for(rect.0, rect.1, rect.2, rect.3)), Operation::new("Do", vec![Object::Name(name.as_bytes().to_vec())])])
}

/// Helvetica widths (per 1000) for page-number text.
fn helv_width(s: &str, size: f32) -> f32 {
    s.chars().map(|c| match c {
        '0'..='9' => 556.0,
        ' ' | '/' => 278.0,
        _ => 556.0,
    }).sum::<f32>() * size / 1000.0
}

pub fn number_pages(src: &Path) -> Result<PathBuf> {
    let mut doc = open(src)?;
    let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
    let n = pages.len();
    let font = doc.add_object(dictionary! {"Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding"});
    for (i, pid) in pages.into_iter().enumerate() {
        let text = format!("{} / {}", i + 1, n);
        let v = view(&doc, pid);
        let tw = helv_width(&text, NUMBER_PT);
        // baseline 22 pt above the bottom edge, centred, upright for the reader
        let (u, base) = ((v.w - tw) / 2.0, v.h - 22.0);
        let m = v.matrix_for(u, base - 1.0, 1.0, 1.0);
        add_resource(&mut doc, pid, "Font", "UtsuriNum", font)?;
        overlay(&mut doc, pid, vec![
            cm(m),
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name(b"UtsuriNum".to_vec()), f(NUMBER_PT)]),
            Operation::new("rg", vec![f(0.1), f(0.1), f(0.1)]),
            Operation::new("Td", vec![f(0.0), f(0.0)]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
        ])?;
    }
    save(&mut doc, &out(src, "Numbered"))
}

fn luma(r: f32, g: f32, b: f32) -> f32 {
    0.299 * r + 0.587 * g + 0.114 * b
}

fn cmyk_gray(c: f32, m: f32, y: f32, k: f32) -> f32 {
    1.0 - (0.3 * c + 0.59 * m + 0.11 * y + k).min(1.0)
}

/// Rewrite colour operators as grey.
fn gray_ops(content: &[u8]) -> Result<Vec<u8>> {
    let c = Content::decode(content)?;
    let nums = |ops: &[Object]| ops.iter().map(num).collect::<Vec<f32>>();
    let ops: Vec<Operation> = c
        .operations
        .into_iter()
        .map(|op| {
            let n = nums(&op.operands);
            match (op.operator.as_str(), n.len()) {
                ("rg", 3) => Operation::new("g", vec![f(luma(n[0], n[1], n[2]))]),
                ("RG", 3) => Operation::new("G", vec![f(luma(n[0], n[1], n[2]))]),
                ("k", 4) => Operation::new("g", vec![f(cmyk_gray(n[0], n[1], n[2], n[3]))]),
                ("K", 4) => Operation::new("G", vec![f(cmyk_gray(n[0], n[1], n[2], n[3]))]),
                ("sc" | "scn", 3) if op.operands.iter().all(|o| !matches!(o, Object::Name(_))) => Operation::new("g", vec![f(luma(n[0], n[1], n[2]))]),
                ("SC" | "SCN", 3) if op.operands.iter().all(|o| !matches!(o, Object::Name(_))) => Operation::new("G", vec![f(luma(n[0], n[1], n[2]))]),
                ("sc" | "scn", 4) if op.operands.iter().all(|o| !matches!(o, Object::Name(_))) => Operation::new("g", vec![f(cmyk_gray(n[0], n[1], n[2], n[3]))]),
                ("SC" | "SCN", 4) if op.operands.iter().all(|o| !matches!(o, Object::Name(_))) => Operation::new("G", vec![f(cmyk_gray(n[0], n[1], n[2], n[3]))]),
                _ => op,
            }
        })
        .collect();
    Ok(Content { operations: ops }.encode()?)
}

/// Decode an image XObject to RGBA (DCT and 8-bit Flate RGB/Gray/CMYK only).
fn decode_image(doc: &Document, s: &Stream) -> Option<image::DynamicImage> {
    let d = &s.dict;
    let w = d.get(b"Width").ok()?.as_i64().ok()? as u32;
    let h = d.get(b"Height").ok()?.as_i64().ok()? as u32;
    let filters: Vec<Vec<u8>> = match d.get(b"Filter").ok() {
        Some(Object::Name(n)) => vec![n.clone()],
        Some(Object::Array(a)) => a.iter().filter_map(|o| o.as_name().ok().map(<[u8]>::to_vec)).collect(),
        _ => vec![],
    };
    let cs = d.get(b"ColorSpace").ok().map(|o| doc.dereference(o).map(|(_, o)| o.clone()).unwrap_or(o.clone()));
    let cs_name = match &cs {
        Some(Object::Name(n)) => String::from_utf8_lossy(n).into_owned(),
        Some(Object::Array(a)) => a.first().and_then(|o| o.as_name().ok()).map(|n| String::from_utf8_lossy(n).into_owned()).unwrap_or_default(),
        _ => String::new(),
    };
    if filters.last().map(|f| f.as_slice()) == Some(b"DCTDecode") {
        if filters.len() == 1 {
            return image::load_from_memory_with_format(&s.content, image::ImageFormat::Jpeg).ok();
        }
        return None;
    }
    if d.get(b"BitsPerComponent").ok().and_then(|o| o.as_i64().ok()) != Some(8) || d.has(b"Decode") {
        return None;
    }
    let raw = if filters.is_empty() { s.content.clone() } else if filters == [b"FlateDecode".to_vec()] { s.decompressed_content().ok()? } else { return None };
    let comps = match cs_name.as_str() {
        "DeviceRGB" | "CalRGB" => 3,
        "DeviceGray" | "CalGray" => 1,
        "DeviceCMYK" => 4,
        "ICCBased" => match &cs {
            Some(Object::Array(a)) => a.get(1).and_then(|o| o.as_reference().ok()).and_then(|r| doc.get_object(r).ok()).and_then(|o| o.as_stream().ok()).and_then(|s| s.dict.get(b"N").ok()?.as_i64().ok()).unwrap_or(3) as usize,
            _ => 3,
        },
        _ => return None,
    };
    if raw.len() < (w * h) as usize * comps {
        return None;
    }
    Some(match comps {
        1 => image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, raw[..(w * h) as usize].to_vec())?),
        3 => image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, raw[..(w * h * 3) as usize].to_vec())?),
        4 => image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| {
            let i = ((y * w + x) * 4) as usize;
            let [c, m, yy, k] = [raw[i], raw[i + 1], raw[i + 2], raw[i + 3]].map(|v| v as f32 / 255.0);
            image::Rgb([(1.0 - c.min(1.0 - k) - k).max(0.0), (1.0 - m.min(1.0 - k) - k).max(0.0), (1.0 - yy.min(1.0 - k) - k).max(0.0)].map(|v| (v * 255.0) as u8))
        })),
        _ => return None,
    })
}

/// All image XObject ids, page by page (each once), with their page number.
fn image_ids(doc: &Document) -> Vec<(usize, ObjectId)> {
    let mut seen = HashSet::new();
    let mut out = vec![];
    for (pno, pid) in doc.get_pages().into_values().enumerate() {
        if let Ok(imgs) = doc.get_page_images(pid) {
            for im in imgs {
                if seen.insert(im.id) {
                    out.push((pno, im.id));
                }
            }
        }
    }
    out
}

pub fn grayscale(src: &Path) -> Result<PathBuf> {
    let mut doc = open(src)?;
    // every content stream: pages and form XObjects
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in ids {
        let Ok(Object::Stream(s)) = doc.get_object(id) else { continue };
        let is_form = s.dict.get(b"Subtype").and_then(Object::as_name).map(|n| n == b"Form").unwrap_or(false);
        let is_page_content = !s.dict.has(b"Subtype") && !s.dict.has(b"Type") && !s.dict.has(b"Length1") && !s.dict.has(b"N");
        if !(is_form || is_page_content) {
            continue;
        }
        let Ok(data) = s.decompressed_content().or_else(|_| Ok::<_, lopdf::Error>(s.content.clone())) else { continue };
        if let Ok(new) = gray_ops(&data) {
            if let Ok(Object::Stream(s)) = doc.get_object_mut(id) {
                s.set_plain_content(new);
                let _ = s.compress();
            }
        }
    }
    for (_, id) in image_ids(&doc) {
        let Ok(Object::Stream(s)) = doc.get_object(id) else { continue };
        let Some(im) = decode_image(&doc, s) else { continue };
        let g = im.to_luma8();
        let mut jpg = Vec::new();
        if image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 85).encode_image(&g).is_err() {
            continue;
        }
        if let Ok(Object::Stream(s)) = doc.get_object_mut(id) {
            s.dict.set("ColorSpace", "DeviceGray");
            s.dict.set("Filter", "DCTDecode");
            s.dict.set("BitsPerComponent", 8);
            s.dict.remove(b"DecodeParms");
            s.set_content(jpg);
            s.allows_compression = false;
        }
    }
    save(&mut doc, &out(src, "Gray"))
}

/// Bake annotation and form-field appearances into the page and drop them.
pub fn flatten(src: &Path) -> Result<PathBuf> {
    let mut doc = open(src)?;
    let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
    let mut any = false;
    for pid in pages {
        let annots = match doc.get_dictionary(pid)?.get(b"Annots") {
            Ok(Object::Array(a)) => a.clone(),
            Ok(Object::Reference(r)) => doc.get_object(*r).and_then(Object::as_array).cloned().unwrap_or_default(),
            _ => continue,
        };
        let mut keep = vec![];
        let mut ops = vec![];
        for (i, a) in annots.iter().enumerate() {
            let Ok(aid) = a.as_reference() else { continue };
            let Ok(ad) = doc.get_dictionary(aid) else { continue };
            let sub = ad.get(b"Subtype").and_then(Object::as_name).map(<[u8]>::to_vec).unwrap_or_default();
            if sub == b"Link" || sub == b"Popup" {
                keep.push(a.clone());
                continue;
            }
            any = true;
            let hidden = ad.get(b"F").and_then(Object::as_i64).map(|f| f & 2 != 0).unwrap_or(false);
            let rect = rect_of(&doc, ad.get(b"Rect").ok().cloned());
            // the normal appearance (for a checkbox: the state in /AS)
            let ap = ad.get(b"AP").ok().and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_dict().ok().cloned());
            let n = ap.and_then(|ap| ap.get(b"N").ok().cloned());
            let stream_id = match n {
                Some(Object::Reference(r)) => match doc.get_object(r) {
                    Ok(Object::Stream(_)) => Some(r),
                    Ok(Object::Dictionary(states)) => ad.get(b"AS").and_then(Object::as_name).ok().and_then(|s| states.get(s).ok()).and_then(|o| o.as_reference().ok()),
                    _ => None,
                },
                Some(Object::Dictionary(states)) => ad.get(b"AS").and_then(Object::as_name).ok().and_then(|s| states.get(s).ok()).and_then(|o| o.as_reference().ok()),
                _ => None,
            };
            let (Some(sid), Some(rect), false) = (stream_id, rect, hidden) else { continue };
            let Ok(Object::Stream(s)) = doc.get_object(sid) else { continue };
            let bbox = rect_of(&doc, s.dict.get(b"BBox").ok().cloned()).unwrap_or([0.0, 0.0, 1.0, 1.0]);
            let m: Vec<f32> = s.dict.get(b"Matrix").and_then(Object::as_array).map(|a| a.iter().map(num).collect()).unwrap_or_else(|_| vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
            // transformed bbox -> rect (PDF 32000 §12.5.5)
            let pts = [(bbox[0], bbox[1]), (bbox[2], bbox[1]), (bbox[0], bbox[3]), (bbox[2], bbox[3])].map(|(x, y)| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]));
            let (tx0, ty0) = pts.iter().fold((f32::MAX, f32::MAX), |a, p| (a.0.min(p.0), a.1.min(p.1)));
            let (tx1, ty1) = pts.iter().fold((f32::MIN, f32::MIN), |a, p| (a.0.max(p.0), a.1.max(p.1)));
            let sx = (rect[2] - rect[0]) / (tx1 - tx0).max(1e-3);
            let sy = (rect[3] - rect[1]) / (ty1 - ty0).max(1e-3);
            let name = format!("UtsuriFlat{i}");
            if let Ok(Object::Stream(s)) = doc.get_object_mut(sid) {
                s.dict.set("Type", "XObject");
                s.dict.set("Subtype", "Form");
            }
            add_resource(&mut doc, pid, "XObject", &name, sid)?;
            ops.push(Operation::new("q", vec![]));
            ops.push(cm([sx, 0.0, 0.0, sy, rect[0] - tx0 * sx, rect[1] - ty0 * sy]));
            ops.push(Operation::new("Do", vec![Object::Name(name.into_bytes())]));
            ops.push(Operation::new("Q", vec![]));
        }
        if !ops.is_empty() {
            overlay(&mut doc, pid, ops)?;
        }
        let pd = doc.get_object_mut(pid)?.as_dict_mut()?;
        if keep.is_empty() {
            pd.remove(b"Annots");
        } else {
            pd.set("Annots", keep);
        }
    }
    let has_form = doc.catalog().map(|c| c.has(b"AcroForm")).unwrap_or(false);
    if !any && !has_form {
        refuse!("Nothing to flatten: this PDF has no form fields or annotations");
    }
    if let Ok(c) = doc.catalog_mut() {
        c.remove(b"AcroForm");
    }
    save(&mut doc, &out(src, "Flattened"))
}

/// Save every embedded image into "<name> images".
pub fn extract_images(src: &Path) -> Result<Vec<PathBuf>> {
    let doc = open(src)?;
    let stem = src.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let folder = crate::out::unique_path(&crate::out::output_dir(src).join(format!("{stem} images")));
    let mut outs = vec![];
    for (pno, id) in image_ids(&doc) {
        let Ok(Object::Stream(s)) = doc.get_object(id) else { continue };
        if s.dict.get(b"Width").and_then(Object::as_i64).unwrap_or(0) < 16 {
            continue;
        }
        let is_jpeg = matches!(s.dict.get(b"Filter"), Ok(Object::Name(n)) if n == b"DCTDecode");
        let n = outs.len() + 1;
        if is_jpeg {
            std::fs::create_dir_all(&folder)?;
            let p = folder.join(format!("page {} image {n}.jpg", pno + 1));
            std::fs::write(&p, &s.content)?;
            outs.push(p);
        } else if let Some(im) = decode_image(&doc, s) {
            std::fs::create_dir_all(&folder)?;
            let p = folder.join(format!("page {} image {n}.png", pno + 1));
            im.save(&p)?;
            outs.push(p);
        }
    }
    if outs.is_empty() {
        refuse!("No images found in this PDF.");
    }
    crate::out::remember_dir(&folder);
    Ok(outs)
}

/// Downsample big images to about `dpi` for the page size and re-encode them as JPEG.
pub fn compress_pdf(src: &Path, dpi: u32, quality: u8) -> Result<PathBuf> {
    let mut doc = open(src)?;
    let max_page = doc.get_pages().into_values().map(|p| {
        let v = view(&doc, p);
        v.w.max(v.h)
    }).fold(0.0f32, f32::max).max(72.0);
    let max_px = (max_page / 72.0 * dpi as f32) as u32;
    for (_, id) in image_ids(&doc) {
        let Ok(Object::Stream(s)) = doc.get_object(id) else { continue };
        if s.dict.has(b"SMask") || s.dict.has(b"Mask") || s.dict.get(b"ImageMask").and_then(Object::as_bool).unwrap_or(false) {
            continue;
        }
        let before = s.content.len();
        let Some(im) = decode_image(&doc, s) else { continue };
        let im = if im.width().max(im.height()) > max_px { im.resize(max_px, max_px, image::imageops::FilterType::Lanczos3) } else { im };
        let gray = matches!(im, image::DynamicImage::ImageLuma8(_));
        let mut jpg = Vec::new();
        let ok = if gray {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, quality).encode_image(&im.to_luma8()).is_ok()
        } else {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, quality).encode_image(&im.to_rgb8()).is_ok()
        };
        if !ok || jpg.len() >= before {
            continue;
        }
        let (w, h) = (im.width() as i64, im.height() as i64);
        if let Ok(Object::Stream(s)) = doc.get_object_mut(id) {
            s.dict.set("Width", w);
            s.dict.set("Height", h);
            s.dict.set("ColorSpace", if gray { "DeviceGray" } else { "DeviceRGB" });
            s.dict.set("Filter", "DCTDecode");
            s.dict.set("BitsPerComponent", 8);
            s.dict.remove(b"DecodeParms");
            s.set_content(jpg);
            s.allows_compression = false;
        }
    }
    save(&mut doc, &out(src, "Compressed"))
}

// ---------------------------------------------------------------- watermark / stamp

pub const WM_MAX_CHARS: usize = 200;

fn check_where(w: &str) -> Result<()> {
    if !CORNERS.contains(&w) {
        refuse!("Unknown position '{w}'");
    }
    Ok(())
}

/// Scale every alpha by `opacity`.
pub fn faded(im: &image::RgbaImage, opacity: f32) -> image::RgbaImage {
    let o = opacity.clamp(0.0, 1.0);
    let mut out = im.clone();
    for p in out.pixels_mut() {
        p.0[3] = (p.0[3] as f32 * o).round() as u8;
    }
    out
}

/// The text mark as a transparent image, rotated `angle` degrees (rising left to right),
/// plus its size in points.
pub fn text_mark(text: &str, size: f32, opacity: f32, angle: f32, color: [u8; 3]) -> Result<(image::RgbaImage, f32, f32)> {
    if text.trim().is_empty() {
        refuse!("Type the watermark text first");
    }
    if text.chars().count() > WM_MAX_CHARS {
        refuse!("Watermark text is too long (at most {WM_MAX_CHARS} characters)");
    }
    let flat = crate::pdfgen::text_stamp_image(text, size, [color[0], color[1], color[2], 255])?;
    // text_stamp_image renders at 4 px per point
    let rotated = rotate_rgba(&flat, angle);
    let im = faded(&rotated, opacity);
    Ok((im.clone(), im.width() as f32 / 4.0, im.height() as f32 / 4.0))
}

/// Rotate counter-clockwise by `deg`, expanding the canvas.
fn rotate_rgba(im: &image::RgbaImage, deg: f32) -> image::RgbaImage {
    if deg.abs() < 0.01 {
        return im.clone();
    }
    let a = deg.to_radians();
    let (s, c) = (a.sin(), a.cos());
    let (w, h) = (im.width() as f32, im.height() as f32);
    let nw = (w * c.abs() + h * s.abs()).ceil() as u32;
    let nh = (w * s.abs() + h * c.abs()).ceil() as u32;
    let (cx, cy, ncx, ncy) = (w / 2.0, h / 2.0, nw as f32 / 2.0, nh as f32 / 2.0);
    image::RgbaImage::from_fn(nw, nh, |x, y| {
        let (dx, dy) = (x as f32 + 0.5 - ncx, y as f32 + 0.5 - ncy);
        // inverse rotation (screen y is down, so CCW on screen is +angle with -y)
        let sx = c * dx - s * dy + cx;
        let sy = s * dx + c * dy + cy;
        bilinear(im, sx - 0.5, sy - 0.5)
    })
}

fn bilinear(im: &image::RgbaImage, x: f32, y: f32) -> image::Rgba<u8> {
    let (w, h) = (im.width() as i32, im.height() as i32);
    let (x0, y0) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let px = |xx: i32, yy: i32| if xx < 0 || yy < 0 || xx >= w || yy >= h { [0f32; 4] } else { im.get_pixel(xx as u32, yy as u32).0.map(|v| v as f32) };
    let (a, b, c, d) = (px(x0, y0), px(x0 + 1, y0), px(x0, y0 + 1), px(x0 + 1, y0 + 1));
    image::Rgba([0, 1, 2, 3].map(|k| (a[k] * (1.0 - fx) * (1.0 - fy) + b[k] * fx * (1.0 - fy) + c[k] * (1.0 - fx) * fy + d[k] * fx * fy).round() as u8))
}

/// Put a prepared mark on the pages, shrunk to fit if it's bigger than a page.
fn place_mark(doc: &mut Document, pages: &[usize], img: &image::RgbaImage, w: f32, h: f32, where_: &str, scale_to_width: Option<f32>) -> Result<()> {
    let ids: Vec<ObjectId> = doc.get_pages().into_values().collect();
    let xobj = image_xobject(doc, img);
    for &i in pages {
        let pid = ids[i];
        let v = view(doc, pid);
        let (w, h) = match scale_to_width {
            Some(frac) => {
                let ww = v.w * frac.clamp(0.05, 1.0);
                (ww, ww * h / w)
            }
            None => (w, h),
        };
        let k = 1f32.min((v.w - 8.0) / w).min((v.h - 8.0) / h);
        let (ww, hh) = (w * k, h * k);
        let margin = 28f32.min((v.w - ww) / 2.0).min((v.h - hh) / 2.0);
        let (u, vv) = anchor(&v, ww, hh, where_, margin);
        draw_image(doc, pid, xobj, (u, vv, ww, hh), "UtsuriMark")?;
    }
    Ok(())
}

pub struct TextMark<'a> {
    pub text: &'a str,
    pub size: f32,
    pub opacity: f32,
    pub angle: f32,
    pub color: [u8; 3],
    pub where_: &'a str,
    pub pages: &'a str,
}

/// Only the centre position is tilted; corner marks stay level.
pub fn watermark_text(src: &Path, m: &TextMark) -> Result<PathBuf> {
    check_where(m.where_)?;
    let angle = if m.where_ == "center" { m.angle } else { 0.0 };
    let (img, w, h) = text_mark(m.text, m.size, m.opacity, angle, m.color)?;
    let mut doc = open(src)?;
    let pages = parse_ranges(m.pages, doc.get_pages().len())?;
    place_mark(&mut doc, &pages, &img, w, h, m.where_, None)?;
    save(&mut doc, &out(src, "Watermarked"))
}

/// Stamp an image, `scale` of the page width, at a corner or the centre.
pub fn stamp_image(src: &Path, image_path: &Path, scale: f32, where_: &str, pages: &str, opacity: f32) -> Result<PathBuf> {
    check_where(where_)?;
    if !image_path.is_file() {
        refuse!("Pick an image to stamp");
    }
    let img = crate::imgio::open_image(image_path).map_err(|_| crate::UserError(format!("{} isn't an image Utsuri can read", name_of(image_path))))?.to_rgba8();
    let img = if img.width().max(img.height()) > 3000 { image::imageops::resize(&img, 3000.min(img.width()), (3000.0 * img.height() as f32 / img.width() as f32) as u32, image::imageops::FilterType::Lanczos3) } else { img };
    let img = faded(&img, opacity);
    let mut doc = open(src)?;
    let pages = parse_ranges(pages, doc.get_pages().len())?;
    let (w, h) = (img.width() as f32, img.height() as f32);
    place_mark(&mut doc, &pages, &img, w, h, where_, Some(scale))?;
    save(&mut doc, &out(src, "Stamped"))
}

// ---------------------------------------------------------------- rendering + OCR

/// Render a page to an image (Windows.Data.Pdf), `dpi` relative to the page size.
pub fn render_page(src: &Path, password: Option<&str>, index: usize, dpi: u32) -> Result<image::DynamicImage> {
    let doc = open_with(src, password)?;
    let pid = *doc.get_pages().values().nth(index).context("no such page")?;
    let v = view(&doc, pid);
    let width = ((v.w / 72.0) * dpi as f32).round().max(1.0) as u32;
    render_width(src, password, index, width)
}

/// Windows.Data.Pdf can't open AES-256 (PDF 2.0) files, so a protected file is decrypted
/// into a temporary copy first.
pub fn render_width(src: &Path, password: Option<&str>, index: usize, width: u32) -> Result<image::DynamicImage> {
    let Some(pw) = password else {
        let png = crate::winrt::render_pdf_page_png(src, None, index as u32, width)?;
        return Ok(image::load_from_memory(&png)?);
    };
    let mut doc = open_with(src, Some(pw))?;
    doc.trailer.remove(b"Encrypt");
    doc.encryption_state = None;
    let tmp = crate::out::temp_part(&std::env::temp_dir(), ".pdf");
    let r = (|| {
        doc.save(&tmp)?;
        let png = crate::winrt::render_pdf_page_png(&tmp, None, index as u32, width)?;
        Ok(image::load_from_memory(&png)?)
    })();
    let _ = std::fs::remove_file(&tmp);
    r
}

pub fn pdf_to_images(src: &Path, target: &str, dpi: Option<u32>) -> Result<Vec<PathBuf>> {
    open(src)?; // refuse passwords with the friendly message
    let dpi = dpi.unwrap_or(crate::options().pdf_dpi);
    let n = page_count(src)?;
    let mut outs = vec![];
    for i in 0..n {
        let im = image::DynamicImage::ImageRgb8(render_page(src, None, i, dpi)?.to_rgb8());
        let label = if n > 1 { format!("page {}", i + 1) } else { String::new() };
        let o = sibling(src, &label, Some(crate::imgio::ext_of_target(target)));
        crate::imgio::save_image(&im, &o, target, None, None)?;
        outs.push(o);
    }
    Ok(outs)
}

pub fn pdf_to_txt(src: &Path) -> Result<PathBuf> {
    open(src)?;
    let text = extract_text(src)?;
    let o = sibling(src, "", Some("txt"));
    atomic_write(&o, |tmp| std::fs::write(tmp, text.as_bytes()))?;
    Ok(o)
}

/// Text blocks become paragraphs (big text = headings), embedded images are kept,
/// one Word page per PDF page.
pub fn pdf_to_docx(src: &Path) -> Result<PathBuf> {
    use crate::docx::{Item, Run, Style};
    let doc = open(src)?;
    let n = doc.get_pages().len();
    let page_ids: Vec<ObjectId> = doc.get_pages().into_values().collect();
    let mut items = vec![];
    for p in 0..n {
        if p > 0 {
            items.push(Item::PageBreak);
        }
        let v = view(&doc, page_ids[p]);
        let mut entries: Vec<(f32, Item)> = vec![];
        // lines -> paragraphs: join lines whose gap is small
        let mut spans = page_spans(src, p)?;
        spans.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal).then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal)));
        let mut cur: Option<(f32, f32, f32, bool, String)> = None; // (top, bottom, size, bold, text)
        let flush = |cur: &mut Option<(f32, f32, f32, bool, String)>, entries: &mut Vec<(f32, Item)>| {
            if let Some((top, _, size, bold, text)) = cur.take() {
                let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if t.is_empty() {
                    return;
                }
                let style = if size >= 20.0 { Style::Heading(1) } else if size >= 16.0 { Style::Heading(2) } else { Style::Normal };
                let bold = bold && t.chars().count() < 120 && style == Style::Normal;
                entries.push((top, Item::Para { style, runs: vec![Run { text: t, bold, ..Default::default() }] }));
            }
        };
        for s in spans {
            if s.text.trim().is_empty() {
                continue;
            }
            match &mut cur {
                Some((_, bottom, size, bold, text)) if (s.y - *bottom) < s.size.max(*size) * 0.8 && (s.size - *size).abs() < 1.5 => {
                    let same_line = (s.y - (*bottom - s.h)).abs() < s.h * 0.5;
                    text.push(if same_line { ' ' } else { ' ' });
                    text.push_str(&s.text);
                    *bottom = bottom.max(s.y + s.h);
                    *bold = *bold && s.bold;
                }
                _ => {
                    flush(&mut cur, &mut entries);
                    cur = Some((s.y, s.y + s.h, s.size, s.bold, s.text.clone()));
                }
            }
        }
        flush(&mut cur, &mut entries);
        // images: placed at their top (from the content stream's cm is complex; put them first)
        for (pno, id) in image_ids(&doc) {
            if pno != p {
                continue;
            }
            let Ok(Object::Stream(s)) = doc.get_object(id) else { continue };
            let Some(im) = decode_image(&doc, s) else { continue };
            if im.width() < 16 {
                continue;
            }
            let png = crate::imaging::png_bytes(&im)?;
            let w_pt = (im.width() as f32 * 72.0 / 150.0).min(v.w);
            let h_pt = w_pt * im.height() as f32 / im.width() as f32;
            entries.push((-1.0, Item::Image { png, width_pt: w_pt, height_pt: h_pt }));
        }
        entries.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        items.extend(entries.into_iter().map(|e| e.1));
    }
    let o = sibling(src, "", Some("docx"));
    crate::docx::write_docx(&o, &items)?;
    Ok(o)
}

/// Refuse a PDF whose every page already has text.
pub fn check_needs_ocr(src: &Path) -> Result<()> {
    open(src)?;
    if page_texts(src)?.iter().all(|t| !t.trim().is_empty()) {
        refuse!("This PDF already has selectable text");
    }
    Ok(())
}

/// OCR the pages without text (Tesseract makes a one-page PDF for each); pages with text
/// are copied untouched. Returns (output, pages recognised).
pub fn make_searchable(src: &Path, lang: &str) -> Result<(PathBuf, usize)> {
    let doc = open(src)?;
    let texts = page_texts(src)?;
    let todo: Vec<usize> = texts.iter().enumerate().filter(|(_, t)| t.trim().is_empty()).map(|(i, _)| i).collect();
    if todo.is_empty() {
        refuse!("This PDF already has selectable text");
    }
    let tmp = std::env::temp_dir().join(format!("utsuri-ocr-{}-{}", std::process::id(), crate::out::file_size(src)));
    std::fs::create_dir_all(&tmp)?;
    let result = (|| -> Result<(PathBuf, usize)> {
        let mut sources = vec![doc.clone()];
        let mut items: Vec<PageRef> = vec![];
        for i in 0..texts.len() {
            if !todo.contains(&i) {
                items.push((0, Some(i), 0));
                continue;
            }
            let im = render_page(src, None, i, OCR_DPI)?;
            let png = tmp.join(format!("p{i}.png"));
            im.save(&png)?;
            let base = tmp.join(format!("p{i}"));
            crate::ocr::tesseract_pdf(&png, &base, lang).map_err(|e| crate::UserError(format!("Text recognition failed on page {}: {}", i + 1, crate::friendly(&e))))?;
            let one = Document::load(base.with_extension("pdf")).context("tesseract wrote no PDF")?;
            // keep the original page size: tesseract's page is the image at its dpi
            sources.push(one);
            items.push((sources.len() - 1, Some(0), 0));
        }
        let mut nd = assemble(&sources, &items)?;
        Ok((save(&mut nd, &out(src, "Searchable"))?, todo.len()))
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testutil::tmpdir;

    /// A test PDF: `n` pages, each with "Page N" in Helvetica and a red box.
    pub fn sample(path: &Path, n: usize, rotate: i64) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font = doc.add_object(dictionary! {"Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica"});
        let res = doc.add_object(dictionary! {"Font" => dictionary! {"F1" => font}});
        let mut kids = vec![];
        for i in 0..n {
            let content = Content {
                operations: vec![
                    Operation::new("1 0 0 rg", vec![]),
                    Operation::new("re", vec![100.into(), 500.into(), 200.into(), 100.into()]),
                    Operation::new("f", vec![]),
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec!["F1".into(), 24.into()]),
                    Operation::new("Td", vec![100.into(), 700.into()]),
                    Operation::new("Tj", vec![Object::string_literal(format!("Page {}", i + 1))]),
                    Operation::new("ET", vec![]),
                ],
            };
            let mut bytes = content.encode().unwrap();
            // "1 0 0 rg" with no operands encodes oddly; write it plainly
            bytes = String::from_utf8_lossy(&bytes).replace("1 0 0 rg", "1 0 0 rg").into_bytes();
            let cid = doc.add_object(Stream::new(dictionary! {}, bytes));
            kids.push(Object::Reference(doc.add_object(dictionary! {"Type" => "Page", "Parent" => pages_id, "Contents" => cid})));
        }
        let count = kids.len() as i64;
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => kids, "Count" => count, "Resources" => res,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Rotate" => rotate,
        }));
        let cat = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_id});
        doc.trailer.set("Root", cat);
        doc.save(path).unwrap();
    }

    fn texts(p: &Path) -> Vec<String> {
        page_texts(p).unwrap().into_iter().map(|t| t.trim().to_string()).collect()
    }

    #[test]
    fn split_merge_halves_rotate() {
        let d = tmpdir("pdfops");
        let src = d.join("doc.pdf");
        sample(&src, 5, 0);
        assert_eq!(texts(&src)[2], "Page 3");
        let parts = split_pdf(&src).unwrap();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[3].file_name().unwrap(), "doc page 4.pdf");
        assert_eq!(texts(&parts[3]), vec!["Page 4"]);
        let merged = merge_pdfs(&[parts[4].clone(), parts[0].clone()]).unwrap();
        assert_eq!(texts(&merged), vec!["Page 5", "Page 1"]);
        let halves = split_halves(&src).unwrap();
        assert_eq!(page_count(&halves[0]).unwrap(), 3);
        assert_eq!(page_count(&halves[1]).unwrap(), 2);
        let rot = rotate_pdf(&src, 90).unwrap();
        let doc = open(&rot).unwrap();
        let pid = *doc.get_pages().values().next().unwrap();
        assert_eq!(view(&doc, pid).rot, 90);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn organize_with_blank_duplicate_rotation_and_cut() {
        let d = tmpdir("organize");
        let a = d.join("a.pdf");
        let b = d.join("b.pdf");
        sample(&a, 3, 0);
        sample(&b, 2, 0);
        let items: Vec<PageRef> = vec![(0, Some(2), 0), (1, Some(0), 90), (0, None, 0), (0, Some(2), 0), (1, Some(1), 0)];
        let outs = organize(&[a.clone(), b.clone()], &items, "Organized", &[3]).unwrap();
        assert_eq!(outs.len(), 2);
        assert_eq!(outs[0].file_name().unwrap(), "a Organized 1.pdf");
        assert_eq!(texts(&outs[0]), vec!["Page 3", "Page 1", ""]);
        assert_eq!(texts(&outs[1]), vec!["Page 3", "Page 2"]);
        let doc = open(&outs[0]).unwrap();
        let pid = *doc.get_pages().values().nth(1).unwrap();
        assert_eq!(view(&doc, pid).rot, 90);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn protect_then_unlock() {
        let d = tmpdir("protect");
        let src = d.join("s.pdf");
        sample(&src, 2, 0);
        let p = protect_pdf(&src, "hunter2").unwrap();
        assert!(is_encrypted(&p).unwrap());
        assert!(needs_password(&p));
        assert!(password_ok(&p, "hunter2"));
        assert!(!password_ok(&p, "nope"));
        let e = open(&p).unwrap_err();
        assert!(crate::friendly(&e).contains("password-protected"));
        assert!(crate::friendly(&unlock(&p, "nope").unwrap_err()).contains("Wrong password"));
        let u = unlock(&p, "hunter2").unwrap();
        assert!(!is_encrypted(&u).unwrap());
        assert_eq!(texts(&u), vec!["Page 1", "Page 2"]);
        // Windows renders the protected file with the password too
        let im = render_page(&p, Some("hunter2"), 0, 72).unwrap();
        assert_eq!(im.width(), 612);
        assert!(crate::friendly(&unlock(&src, "x").unwrap_err()).contains("no password"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn numbers_gray_metadata_and_rendering() {
        let d = tmpdir("pdftools");
        let src = d.join("n.pdf");
        sample(&src, 3, 90);
        let num = number_pages(&src).unwrap();
        let t = texts(&num);
        assert!(t[1].contains("2 / 3"), "{t:?}");
        let gray = grayscale(&src).unwrap();
        let im = render_page(&gray, None, 0, 36).unwrap().to_rgb8();
        assert!(im.pixels().all(|p| (p.0[0] as i32 - p.0[2] as i32).abs() < 8), "still colour");
        let col = render_page(&src, None, 0, 36).unwrap().to_rgb8();
        assert!(col.pixels().any(|p| p.0[0] > 200 && p.0[2] < 60), "the sample should have red");
        let clean = strip_pdf_metadata(&src).unwrap();
        assert!(read_pdf_metadata(&clean).unwrap().iter().any(|(k, v)| k == "Pages" && v == "3"));
        let imgs = pdf_to_images(&src, "PNG", Some(36)).unwrap();
        assert_eq!(imgs.len(), 3);
        // rotated 90: the image is landscape
        let (w, h) = image::image_dimensions(&imgs[0]).unwrap();
        assert!(w > h, "{w}x{h}");
        let txt = pdf_to_txt(&src).unwrap();
        assert!(std::fs::read_to_string(&txt).unwrap().contains("Page 2"));
        let docx = pdf_to_docx(&src).unwrap();
        assert!(crate::docx::read_text(&docx).unwrap().contains("Page 3"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn watermark_and_stamp_land_on_the_page() {
        let d = tmpdir("wm");
        let src = d.join("w.pdf");
        sample(&src, 2, 0);
        let wm = watermark_text(&src, &TextMark { text: "DRAFT", size: 60.0, opacity: 0.5, angle: 45.0, color: [0, 0, 255], where_: "center", pages: "2" }).unwrap();
        let p1 = render_page(&wm, None, 0, 36).unwrap().to_rgb8();
        let p2 = render_page(&wm, None, 1, 36).unwrap().to_rgb8();
        let blue = |im: &image::RgbImage| im.pixels().filter(|p| p.0[2] as i32 > p.0[0] as i32 + 40 && p.0[2] as i32 > p.0[1] as i32 + 40).count();
        assert_eq!(blue(&p1), 0, "page 1 wasn't in the range");
        assert!(blue(&p2) > 20, "page 2 has the mark");
        let logo = d.join("logo.png");
        image::RgbaImage::from_pixel(40, 20, image::Rgba([0, 200, 0, 255])).save(&logo).unwrap();
        let st = stamp_image(&src, &logo, 0.25, "bottom-right", "", 1.0).unwrap();
        let im = render_page(&st, None, 0, 36).unwrap().to_rgb8();
        let (w, h) = im.dimensions();
        // bottom-right quadrant is green, top-left isn't
        assert!(im.get_pixel(w - 20, h - 20).0[1] > 150 || (w - 40..w - 10).any(|x| im.get_pixel(x, h - 18).0[1] > 150));
        assert!(im.get_pixel(10, 10).0[1] > 200 && im.get_pixel(10, 10).0[0] > 200, "top-left stays white");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_ranges("", 4).unwrap(), vec![0, 1, 2, 3]);
        assert_eq!(parse_ranges("1-2, 4", 4).unwrap(), vec![0, 1, 3]);
        assert_eq!(parse_ranges("3-", 4).unwrap(), vec![2, 3]);
        assert!(parse_ranges("5", 4).is_err());
        assert!(parse_ranges("x", 4).is_err());
        assert_eq!(label_for(&[0, 1, 2, 6]), "pages 1-3, 7");
        assert_eq!(label_for(&[4]), "page 5");
    }

    #[test]
    fn compress_shrinks_a_photo_pdf_and_extract_finds_it() {
        let d = tmpdir("pdfimg");
        let png = d.join("p.png");
        crate::testutil::photo(1800, 1200).save(&png).unwrap();
        let pdf = crate::imgio::images_to_single_pdf(&[png]).unwrap();
        let small = compress_pdf(&pdf, 72, 50).unwrap();
        assert!(crate::out::file_size(&small) < crate::out::file_size(&pdf), "{} vs {}", crate::out::file_size(&small), crate::out::file_size(&pdf));
        let imgs = extract_images(&pdf).unwrap();
        assert_eq!(imgs.len(), 1);
        assert!(imgs[0].parent().unwrap().file_name().unwrap().to_string_lossy().ends_with("images"));
        let _ = std::fs::remove_dir_all(&d);
    }
}

fn load_pw(bytes: &[u8], pw: &str) -> lopdf::Result<Document> {
    let mut o = lopdf::LoadOptions::default();
    o.password = Some(pw.to_string());
    Document::load_mem_with_options(bytes, o)
}

