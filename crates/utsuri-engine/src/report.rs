//! "What just happened", short and useful, for the notification. Ported from report.py.

use crate::kinds::{Kind, category};
use crate::out::{file_size, human_size};
use std::path::{Path, PathBuf};

/// Something to put on the clipboard when the job finishes.
#[derive(Clone, Debug)]
pub enum Clip {
    Text(String),
    Image(image::RgbaImage),
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub title: String,
    pub detail: String,
    pub outs: Vec<PathBuf>,
    pub srcs: Vec<PathBuf>,
    /// Highlighted figure, e.g. "75% smaller".
    pub good: String,
    pub clip: Option<Clip>,
}

impl Report {
    pub fn new(title: impl Into<String>, detail: impl Into<String>, outs: Vec<PathBuf>, srcs: Vec<PathBuf>) -> Self {
        Report { title: title.into(), detail: detail.into(), outs, srcs, ..Default::default() }
    }
    pub fn good(mut self, g: impl Into<String>) -> Self {
        self.good = g.into();
        self
    }
    pub fn clip(mut self, c: Clip) -> Self {
        self.clip = Some(c);
        self
    }
    /// The first image result, for the notification's thumbnail.
    pub fn thumb(&self) -> Option<&PathBuf> {
        self.outs.iter().find(|o| category(o) == Some(Kind::Image))
    }
}

fn dims(p: &Path) -> Option<(u32, u32)> {
    image::image_dimensions(p).ok().or_else(|| crate::imgio::open_image(p).ok().map(|i| (i.width(), i.height())))
}

fn pages(p: &Path) -> Option<usize> {
    crate::pdf::page_count(p).ok()
}

fn dim_str(d: Option<(u32, u32)>) -> String {
    d.map(|(w, h)| format!("{w}×{h}")).unwrap_or_default()
}

/// ("2.4 MB → 610.0 KB", "75% smaller"); the second part is empty without a real change.
pub fn size_change(before: u64, after: u64) -> (String, String) {
    let text = format!("{} → {}", human_size(before), human_size(after));
    if before == 0 {
        return (text, String::new());
    }
    let pct = (1.0 - after as f64 / before as f64) * 100.0;
    let v = if pct >= 1.0 {
        format!("{pct:.0}% smaller")
    } else if pct <= -1.0 {
        format!("{:.0}% larger", -pct)
    } else {
        "same size".into()
    };
    (text, v)
}

fn name(p: &Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

/// `action` is "convert:PNG" or "tool:Compress" style.
pub fn summarize(action: &str, srcs: &[PathBuf], outs: &[PathBuf], extra: &str) -> Report {
    let outs: Vec<PathBuf> = outs.to_vec();
    if outs.is_empty() {
        return Report::new(action, if extra.is_empty() { "Nothing was written" } else { extra }, vec![], srcs.to_vec());
    }
    let src = srcs.first().cloned().unwrap_or_else(|| outs[0].clone());
    let before: u64 = srcs.iter().filter(|s| s.is_file()).map(|s| file_size(s)).sum();
    let after: u64 = outs.iter().filter(|o| o.is_file()).map(|o| file_size(o)).sum();
    let (kind, n) = action.split_once(':').unwrap_or(("", action));
    let o0 = &outs[0];
    let r = |t: String, d: String| Report::new(t, d, outs.clone(), srcs.to_vec());

    if kind == "convert" {
        let fmt = n;
        let title = if srcs.len() > 1 && outs.len() == 1 {
            format!("{} files → 1 {fmt}", srcs.len())
        } else if outs.len() > 1 && srcs.len() == 1 {
            format!("{} pages → {} {fmt}s", pages(&src).unwrap_or(outs.len()), outs.len())
        } else if outs.len() > 1 {
            format!("{} files → {fmt}", outs.len())
        } else {
            format!("{} → {fmt}", src.extension().unwrap_or_default().to_string_lossy().to_uppercase())
        };
        let mut bits = vec![if outs.len() == 1 { name(o0) } else { format!("{}/{} …", o0.parent().map(name).unwrap_or_default(), name(o0)) }];
        if category(o0) == Some(Kind::Image) {
            if let Some(d) = dims(o0) {
                bits.push(dim_str(Some(d)));
            }
        }
        if crate::kinds::ext(o0) == "pdf" {
            if let Some(p) = pages(o0) {
                bits.push(format!("{p} page{}", if p != 1 { "s" } else { "" }));
            }
        }
        bits.push(human_size(after));
        return r(title, bits.join(" · "));
    }
    match n {
        "Compress" => {
            let (sizes, mut verdict) = size_change(before, after);
            if verdict.ends_with("larger") || verdict == "same size" {
                verdict = "already well compressed".into();
            }
            let title = if srcs.len() > 1 { format!("Compressed {} files", srcs.len()) } else { "Compressed".into() };
            r(title, if extra.is_empty() { sizes } else { format!("{sizes} · {extra}") }).good(verdict)
        }
        "Crop" | "Resize" | "Edit" => {
            let verb = match n {
                "Crop" => "Cropped",
                "Resize" => "Resized",
                _ => "Edited",
            };
            if outs.len() > 1 {
                return r(format!("{verb} {} images", outs.len()), format!("{} → {}", human_size(before), human_size(after)));
            }
            let (d0, d1) = (dims(&src), dims(o0));
            let mut detail = if d0.is_some() && d1.is_some() && d0 != d1 { format!("{} → {}", dim_str(d0), dim_str(d1)) } else { dim_str(d1) };
            if !extra.is_empty() {
                detail.push_str(&format!(" · {extra}"));
            }
            r(verb.into(), format!("{detail} · {}", name(o0)))
        }
        "Add BG" => r("Background added".into(), format!("{} PNG · {} · {}", dim_str(dims(o0)), human_size(after), name(o0))),
        "Annotate" => r("Annotated".into(), if extra.is_empty() { name(o0) } else { format!("{extra} · {}", name(o0)) }),
        "Redact" => r("Redacted".into(), if extra.is_empty() { name(o0) } else { format!("{extra} · {}", name(o0)) }).good("burned into pixels"),
        "Metadata" => r("Metadata removed".into(), if extra.is_empty() { name(o0) } else { format!("{extra} · {}", name(o0)) }).good("private copy"),
        "Split" => r(format!("Split into {} PDFs", outs.len()), format!("{} …", name(o0))),
        "Merge" => r(format!("Merged {} PDFs", srcs.len()), format!("{} pages · {} · {}", pages(o0).unwrap_or(0), human_size(after), name(o0))),
        "Rotate" => {
            let p = pages(o0).unwrap_or(0);
            r("Rotated 90°".into(), format!("{p} page{} · {}", if p != 1 { "s" } else { "" }, name(o0)))
        }
        "Protect" => r("Password protected".into(), format!("AES-256 · {}", name(o0))).good("encrypted"),
        "Extract" => r(format!("Extracted {} image{}", outs.len(), if outs.len() != 1 { "s" } else { "" }), format!("{}/ · {}", o0.parent().map(name).unwrap_or_default(), human_size(after))),
        _ => r(if n.is_empty() { action.to_string() } else { n.to_string() }, format!("{} · {}", name(o0), human_size(after))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_verdicts() {
        assert_eq!(size_change(1000, 250).1, "75% smaller");
        assert_eq!(size_change(1000, 1200).1, "20% larger");
        assert_eq!(size_change(1000, 1000).1, "same size");
        assert_eq!(size_change(0, 10).1, "");
    }

    #[test]
    fn convert_and_compress_titles() {
        let d = crate::testutil::tmpdir("report");
        let a = d.join("a.png");
        crate::testutil::photo(64, 32).save(&a).unwrap();
        let b = crate::imgio::image_to(&a, "JPG").unwrap();
        let r = summarize("convert:JPG", std::slice::from_ref(&a), std::slice::from_ref(&b), "");
        assert_eq!(r.title, "PNG → JPG");
        assert!(r.detail.starts_with("a.jpg · 64×32 · "), "{}", r.detail);
        assert_eq!(r.thumb(), Some(&b));
        let c = crate::imaging::compress_image(&a, 40, None, false).unwrap();
        let r = summarize("tool:Compress", &[a], &[c], "");
        assert_eq!(r.title, "Compressed");
        assert!(r.good.ends_with("smaller") || r.good == "already well compressed");
        let _ = std::fs::remove_dir_all(&d);
    }
}
