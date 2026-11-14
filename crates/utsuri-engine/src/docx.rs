//! Word documents: read paragraphs/runs/tables, write simple documents.
//! Ported from convert.py (docx_to_html, docx_to_txt, text_to_docx, pdf_to_docx).

use anyhow::{Context as _, Result};
use quick_xml::Reader;
use quick_xml::events::Event;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Normal,
    Title,
    Heading(u8),
    List,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Para { style: Style, runs: Vec<Run> },
    Table(Vec<Vec<String>>),
}

impl Block {
    pub fn text(&self) -> String {
        match self {
            Block::Para { runs, .. } => runs.iter().map(|r| r.text.as_str()).collect(),
            Block::Table(rows) => rows.iter().map(|r| r.join("\t")).collect::<Vec<_>>().join("\n"),
        }
    }
}

fn read_part(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    let mut f = zip.by_name(name).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    Some(s)
}

fn local(name: &[u8]) -> &[u8] {
    name.rsplit(|c| *c == b':').next().unwrap_or(name)
}

fn attr(e: &quick_xml::events::BytesStart, key: &[u8]) -> Option<String> {
    e.attributes().flatten().find(|a| local(a.key.as_ref().as_bytes()) == key).map(|a| a.value.to_string())
}

/// styleId -> lowercased style name (from styles.xml).
fn style_names(xml: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut r = Reader::from_str(xml);
    let mut cur: Option<String> = None;
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match local(e.name().as_ref().as_bytes()) {
                b"style" => cur = attr(&e, b"styleId"),
                b"name" => {
                    if let (Some(id), Some(n)) = (cur.clone(), attr(&e, b"val")) {
                        out.insert(id, n.to_lowercase());
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

fn style_of(name: &str, numbered: bool) -> Style {
    if let Some(n) = name.strip_prefix("heading").map(str::trim) {
        return Style::Heading(n.parse::<u8>().unwrap_or(1).clamp(1, 6));
    }
    if name == "title" {
        return Style::Title;
    }
    if name.contains("list") || numbered {
        return Style::List;
    }
    Style::Normal
}

/// The body of a .docx as blocks, in document order.
pub fn read_blocks(src: &Path) -> Result<Vec<Block>> {
    let f = std::fs::File::open(src)?;
    let mut zip = zip::ZipArchive::new(f).map_err(|_| crate::UserError(format!("{} isn't a readable Word file", src.file_name().unwrap_or_default().to_string_lossy())))?;
    let doc = read_part(&mut zip, "word/document.xml").context("no word/document.xml in this file")?;
    let names = read_part(&mut zip, "word/styles.xml").map(|s| style_names(&s)).unwrap_or_default();
    let mut r = Reader::from_str(&doc);
    let mut blocks = vec![];
    // state
    let mut table_depth = 0usize;
    let mut rows: Vec<Vec<String>> = vec![];
    let mut cell = String::new();
    let mut runs: Vec<Run> = vec![];
    let mut run = Run::default();
    let mut style_id = String::new();
    let mut numbered = false;
    let mut in_rpr = false;
    let mut in_text = false;
    loop {
        let ev = match r.read_event() {
            Ok(e) => e,
            Err(e) => return Err(anyhow::anyhow!("bad document.xml: {e}")),
        };
        // an empty <w:p/> is a blank line: treat it as a start and an end
        let ev = match ev {
            Event::Empty(e) if local(e.name().as_ref().as_bytes()) == b"p" => {
                if table_depth == 0 {
                    blocks.push(Block::Para { style: Style::Normal, runs: vec![] });
                }
                continue;
            }
            ev => ev,
        };
        match ev {
            Event::Start(e) | Event::Empty(e) => {
                match local(e.name().as_ref().as_bytes()) {
                    b"tbl" => {
                        table_depth += 1;
                        if table_depth == 1 {
                            rows.clear();
                        }
                    }
                    b"tr" if table_depth == 1 => rows.push(vec![]),
                    b"tc" if table_depth == 1 => cell.clear(),
                    b"p" => {
                        runs.clear();
                        style_id.clear();
                        numbered = false;
                    }
                    b"pStyle" => style_id = attr(&e, b"val").unwrap_or_default(),
                    b"numPr" => numbered = true,
                    b"r" => run = Run::default(),
                    b"rPr" => in_rpr = true,
                    b"b" if in_rpr => run.bold = attr(&e, b"val").is_none_or(|v| v != "0" && v != "false"),
                    b"i" if in_rpr => run.italic = attr(&e, b"val").is_none_or(|v| v != "0" && v != "false"),
                    b"u" if in_rpr => run.underline = attr(&e, b"val").is_none_or(|v| v != "none"),
                    b"t" => in_text = true,
                    b"tab" if !in_rpr => run.text.push('\t'),
                    b"br" | b"cr" => run.text.push('\n'),
                    _ => {}
                }
            }
            Event::Text(t) if in_text => {
                let s = t.into_inner().into_owned();
                run.text.push_str(&quick_xml::escape::unescape(&s).map(|c| c.into_owned()).unwrap_or(s));
            }
            Event::GeneralRef(g) if in_text => {
                let name = g.into_inner().into_owned();
                let s = match name.as_str() {
                    "amp" => "&".to_string(),
                    "lt" => "<".to_string(),
                    "gt" => ">".to_string(),
                    "quot" => "\"".to_string(),
                    "apos" => "'".to_string(),
                    n if n.starts_with("#x") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32).map(String::from).unwrap_or_default(),
                    n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32).map(String::from).unwrap_or_default(),
                    _ => String::new(),
                };
                run.text.push_str(&s);
            }
            Event::End(e) => match local(e.name().as_ref().as_bytes()) {
                b"t" => in_text = false,
                b"rPr" => in_rpr = false,
                b"r" => {
                    if !run.text.is_empty() {
                        runs.push(std::mem::take(&mut run));
                    }
                }
                b"p" => {
                    if table_depth > 0 {
                        if !cell.is_empty() {
                            cell.push('\n');
                        }
                        cell.push_str(&runs.iter().map(|r| r.text.as_str()).collect::<String>());
                    } else {
                        let name = names.get(&style_id).cloned().unwrap_or_else(|| style_id.to_lowercase());
                        let name = if name.starts_with("heading") || name == "title" { name } else { name.replace("heading", "heading ") };
                        blocks.push(Block::Para { style: style_of(&name, numbered), runs: std::mem::take(&mut runs) });
                    }
                }
                b"tc" if table_depth == 1 => {
                    if let Some(row) = rows.last_mut() {
                        row.push(std::mem::take(&mut cell));
                    }
                }
                b"tbl" => {
                    table_depth -= 1;
                    if table_depth == 0 {
                        blocks.push(Block::Table(std::mem::take(&mut rows)));
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(blocks)
}

pub fn read_text(src: &Path) -> Result<String> {
    Ok(read_blocks(src)?.iter().map(Block::text).collect::<Vec<_>>().join("\n"))
}

// ---------------------------------------------------------------- writing

/// What a new document holds.
pub enum Item {
    Para { style: Style, runs: Vec<Run> },
    Image { png: Vec<u8>, width_pt: f32, height_pt: f32 },
    PageBreak,
}

fn esc(s: &str) -> String {
    quick_xml::escape::escape(s).into_owned()
}

fn run_xml(r: &Run) -> String {
    let mut props = String::new();
    if r.bold {
        props.push_str("<w:b/>");
    }
    if r.italic {
        props.push_str("<w:i/>");
    }
    if r.underline {
        props.push_str("<w:u w:val=\"single\"/>");
    }
    let rpr = if props.is_empty() { String::new() } else { format!("<w:rPr>{props}</w:rPr>") };
    let mut parts = String::new();
    for (i, line) in r.text.split('\n').enumerate() {
        if i > 0 {
            parts.push_str("<w:br/>");
        }
        for (j, seg) in line.split('\t').enumerate() {
            if j > 0 {
                parts.push_str("<w:tab/>");
            }
            if !seg.is_empty() {
                parts.push_str(&format!("<w:t xml:space=\"preserve\">{}</w:t>", esc(seg)));
            }
        }
    }
    format!("<w:r>{rpr}{parts}</w:r>")
}

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:cs="Calibri" w:eastAsia="Calibri"/><w:sz w:val="22"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="120"/></w:pPr></w:pPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:rPr><w:b/><w:sz w:val="48"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="240"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="36"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="200"/><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:b/><w:sz w:val="28"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/><w:outlineLvl w:val="2"/></w:pPr><w:rPr><w:b/><w:sz w:val="24"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:pPr><w:ind w:left="720"/></w:pPr></w:style>
</w:styles>"#;

/// Write a .docx with these items (atomically).
pub fn write_docx(out: &Path, items: &[Item]) -> Result<()> {
    let mut body = String::new();
    let mut media: Vec<(String, &[u8])> = vec![];
    for it in items {
        match it {
            Item::Para { style, runs } => {
                let ppr = match style {
                    Style::Normal => String::new(),
                    Style::Title => "<w:pPr><w:pStyle w:val=\"Title\"/></w:pPr>".into(),
                    Style::Heading(n) => format!("<w:pPr><w:pStyle w:val=\"Heading{}\"/></w:pPr>", (*n).clamp(1, 3)),
                    Style::List => "<w:pPr><w:pStyle w:val=\"ListParagraph\"/></w:pPr>".into(),
                };
                let runs: String = runs.iter().map(run_xml).collect();
                body.push_str(&format!("<w:p>{ppr}{runs}</w:p>"));
            }
            Item::PageBreak => body.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>"),
            Item::Image { png, width_pt, height_pt } => {
                let n = media.len() + 1;
                let rid = format!("rIdImg{n}");
                media.push((format!("image{n}.png"), png));
                // 6.5 in wide at most
                let k = (468.0 / width_pt).min(1.0);
                let (cx, cy) = ((width_pt * k * 12700.0) as i64, (height_pt * k * 12700.0) as i64);
                body.push_str(&format!(
                    r#"<w:p><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{n}" name="Picture {n}"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:nvPicPr><pic:cNvPr id="{n}" name="image{n}.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
                ));
            }
        }
    }
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><w:body>{body}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let mut rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    );
    for (i, (name, _)) in media.iter().enumerate() {
        rels.push_str(&format!(r#"<Relationship Id="rIdImg{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/{name}"/>"#, i + 1));
    }
    rels.push_str("</Relationships>");
    let types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#;
    let root_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

    crate::out::atomic_write(out, |tmp| {
        let f = std::fs::File::create(tmp)?;
        let mut z = zip::ZipWriter::new(f);
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let mut put = |name: &str, data: &[u8]| -> std::io::Result<()> {
            z.start_file(name, o).map_err(std::io::Error::other)?;
            z.write_all(data)
        };
        put("[Content_Types].xml", types.as_bytes())?;
        put("_rels/.rels", root_rels.as_bytes())?;
        put("word/document.xml", document.as_bytes())?;
        put("word/styles.xml", STYLES.as_bytes())?;
        put("word/_rels/document.xml.rels", rels.as_bytes())?;
        for (name, data) in &media {
            put(&format!("word/media/{name}"), data)?;
        }
        z.finish().map_err(std::io::Error::other)?;
        Ok(())
    })?;
    Ok(())
}

/// Read a text file in whatever encoding it's in (UTF-8, UTF-16, cp1252).
pub fn read_any_text(src: &Path) -> Result<String> {
    let raw = std::fs::read(src)?;
    if let Some(rest) = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Ok(String::from_utf8_lossy(rest).into_owned());
    }
    if raw.starts_with(&[0xFF, 0xFE]) || raw.starts_with(&[0xFE, 0xFF]) {
        let le = raw[0] == 0xFF;
        let units: Vec<u16> = raw[2..].chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        return Ok(String::from_utf16_lossy(&units));
    }
    match String::from_utf8(raw) {
        Ok(s) => Ok(s),
        // cp1252 / latin-1: map bytes 0x80..0x9F through the Windows table
        Err(e) => {
            const CP1252: [char; 32] = ['€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ'];
            Ok(e.into_bytes().iter().map(|&b| if (0x80..0xA0).contains(&b) { CP1252[(b - 0x80) as usize] } else { b as char }).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::tmpdir;

    #[test]
    fn write_then_read_back() {
        let d = tmpdir("docx");
        let p = d.join("t.docx");
        let items = vec![
            Item::Para { style: Style::Heading(1), runs: vec![Run { text: "Tiêu đề".into(), ..Default::default() }] },
            Item::Para { style: Style::Normal, runs: vec![Run { text: "plain & ".into(), ..Default::default() }, Run { text: "bold".into(), bold: true, ..Default::default() }] },
            Item::PageBreak,
            Item::Para { style: Style::List, runs: vec![Run { text: "item\tone".into(), italic: true, ..Default::default() }] },
            Item::Image { png: crate::imaging::png_bytes(&image::DynamicImage::new_rgb8(4, 4)).unwrap(), width_pt: 100.0, height_pt: 50.0 },
        ];
        write_docx(&p, &items).unwrap();
        let blocks = read_blocks(&p).unwrap();
        assert_eq!(blocks[0], Block::Para { style: Style::Heading(1), runs: vec![Run { text: "Tiêu đề".into(), ..Default::default() }] });
        let Block::Para { runs, .. } = &blocks[1] else { panic!() };
        assert_eq!(runs[0].text, "plain & ");
        assert!(runs[1].bold);
        let list = blocks.iter().find(|b| matches!(b, Block::Para { style: Style::List, .. })).unwrap();
        assert_eq!(list.text(), "item\tone");
        assert!(read_text(&p).unwrap().contains("plain & bold"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn text_encodings() {
        let d = tmpdir("enc");
        let p = d.join("a.txt");
        std::fs::write(&p, [0xEF, 0xBB, 0xBF, b'h', b'i']).unwrap();
        assert_eq!(read_any_text(&p).unwrap(), "hi");
        std::fs::write(&p, [0xFF, 0xFE, b'o', 0, b'k', 0]).unwrap();
        assert_eq!(read_any_text(&p).unwrap(), "ok");
        std::fs::write(&p, [b'c', b'a', b'f', 0xE9, b' ', 0x80]).unwrap();
        assert_eq!(read_any_text(&p).unwrap(), "café €");
        let _ = std::fs::remove_dir_all(&d);
    }
}
