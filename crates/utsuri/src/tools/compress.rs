//! Compress images: by quality or to a target size, with a before/after preview at 100%.

use super::subtitle;
use crate::ui::frame::{Tool, ToolS, footer};
use crate::ui::*;
use image::DynamicImage;
use rune::{Cx, Tx};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use utsuri_engine::imaging;
use utsuri_engine::out::{file_size, human_size};

const SIDES: &[(&str, Option<u32>)] = &[("Original size", None), ("Max 3840 px", Some(3840)), ("Max 2560 px", Some(2560)), ("Max 1920 px", Some(1920)), ("Max 1280 px", Some(1280)), ("Max 800 px", Some(800))];
const TARGETS: &[(&str, usize)] = &[("2 MB", 2_000_000), ("1 MB", 1_000_000), ("500 KB", 500_000), ("200 KB", 200_000), ("100 KB", 100_000)];

pub struct Estimate {
    after: rune::Pixels,
    bytes: usize,
    quality: u8,
}

pub struct Compress {
    paths: Vec<PathBuf>,
    pv: Arc<DynamicImage>,
    full_long: u32,
    mode: usize,
    quality: u8,
    target: usize,
    side: usize,
    webp: bool,
    before: rune::Pixels,
    est: rune::Latest<Result<Estimate, String>>,
    shown: Option<Result<Estimate, String>>,
}

/// A 100% detail crop (the middle third) for judging quality.
fn detail(im: &DynamicImage) -> DynamicImage {
    let (w, h) = (im.width(), im.height());
    let (cw, ch) = (w.min(320), h.min(240));
    im.crop_imm((w - cw) / 2, (h - ch) / 2, cw, ch)
}

impl Compress {
    pub fn new(paths: Vec<PathBuf>) -> anyhow::Result<(Self, String)> {
        let full = utsuri_engine::imgio::open_image(&paths[0])?;
        let full_long = full.width().max(full.height());
        let pv = if full_long > 1600 { full.resize(1600, 1600, image::imageops::FilterType::Triangle) } else { full };
        let before = pixels_of(&detail(&pv));
        let sub = subtitle(&paths);
        let est = rune::Latest::default().debounce(Duration::from_millis(80));
        Ok((Compress { paths, pv: Arc::new(pv), full_long, mode: 0, quality: 72, target: 2, side: 0, webp: false, before, est, shown: None }, sub))
    }

    fn target_fmt(&self) -> &'static str {
        imaging::lossy_target(&self.paths[0], self.webp)
    }

    /// Re-estimate in the background (the newest settings win).
    fn kick(&self) {
        let im = self.pv.clone();
        let fmt = self.target_fmt();
        let out_long = SIDES[self.side].1.map_or(self.full_long, |s| s.min(self.full_long));
        let px_ratio = (out_long as f64 / im.width().max(im.height()) as f64).powi(2);
        let (mode, q, limit) = (self.mode, self.quality, TARGETS[self.target].1);
        self.est.request((), move |_| {
            let size = |q: u8| utsuri_engine::imgio::encoded_size(&im, fmt, q).map(|n| (n as f64 * px_ratio) as usize).map_err(|e| e.to_string());
            let q = if mode == 1 {
                let (mut lo, mut hi, mut best) = (30i32, 92i32, 30u8);
                while lo <= hi {
                    let m = (lo + hi) / 2;
                    if size(m as u8)? <= limit {
                        best = m as u8;
                        lo = m + 1;
                    } else {
                        hi = m - 1;
                    }
                }
                best
            } else {
                q
            };
            let bytes = size(q)?;
            let mut v = Vec::new();
            if fmt == "PNG" {
                imaging::write_quantized_png(&im, q, &mut v).map_err(|e| e.to_string())?;
            } else {
                utsuri_engine::imgio::encode(&im, fmt, q, &mut v).map_err(|e| e.to_string())?;
            }
            let after = match fmt {
                "HEIC" | "AVIF" => (*im).clone(),
                _ => image::load_from_memory(&v).map_err(|e| e.to_string())?,
            };
            Ok(Estimate { after: pixels_of(&detail(&after)), bytes, quality: q })
        });
    }
}

type S = ToolS<Compress>;

impl Tool for Compress {
    fn title(&self) -> String {
        "Compress".into()
    }
    fn size(&self) -> (f32, f32) {
        (680.0, 660.0)
    }
    fn setup(s: &mut S, waker: rune::Waker) {
        s.t.est.set_waker(waker);
        s.t.kick();
    }
    fn poll(s: &mut S, _tx: &mut Tx) -> bool {
        match s.t.est.take() {
            Some(e) => {
                s.t.shown = Some(e);
                true
            }
            None => false,
        }
    }
    fn body(s: &S, _cx: &mut Cx<S>) -> El {
        let th = &s.theme;
        let c = &s.t;
        let mode = toggles("mode", &["Quality", "Target size"], c.mode, th, |s: &mut S, i| {
            s.t.mode = i;
            s.t.kick();
        });
        let how = if c.mode == 0 {
            row().items_center().gap(12.0).child(el().w(130.0).child(label("Quality", th))).child(slider("quality", c.quality as f32, 10.0, 95.0, 260.0, th, |s: &mut S, v| {
                let q = v.round() as u8;
                if q != s.t.quality {
                    s.t.quality = q;
                    s.t.kick();
                }
            }))
            .child(dim(c.quality.to_string(), th).key("quality/value"))
        } else {
            let labels: Vec<&str> = TARGETS.iter().map(|t| t.0).collect();
            row().items_center().gap(12.0).child(el().w(130.0).child(label("Make each under", th))).child(toggles("target", &labels, c.target, th, |s: &mut S, i| {
                s.t.target = i;
                s.t.kick();
            }))
        };
        let sides: Vec<&'static str> = SIDES.iter().map(|x| x.0).collect();
        let size = row().items_center().gap(12.0).child(el().w(130.0).child(label("Size", th))).child(picker("side", &sides, c.side, 200.0, th, |s: &mut S, i| {
            s.t.side = i;
            s.t.kick();
        }));
        let webp = checkbox("webp", c.webp, "Convert to WebP (usually the smallest)", th).on_press(|s: &mut S| {
            s.t.webp = !s.t.webp;
            s.t.kick();
        });
        let pending = c.est.is_pending();
        let after = match &c.shown {
            Some(Ok(e)) => Some(&e.after),
            _ => None,
        };
        let well = |key: &str, px: Option<&rune::Pixels>, cap: &str| col().grow(1.0).min_w(0.0).gap(6.0).child(picture(key, px).w_full().h(220.0)).child(dim(cap, th));
        let previews = row().w_full().gap(12.0).child(well("before", Some(&c.before), "Original")).child(well("after", after, if pending { "Compressed (updating…)" } else { "Compressed" }));
        let orig1 = file_size(&c.paths[0]);
        let total_orig: u64 = c.paths.iter().map(|p| file_size(p)).sum();
        let result = match &c.shown {
            Some(Err(e)) => text(e.clone()).key("result").wrap().w_full().font(BODY).color(col_of(BAD)),
            Some(Ok(e)) => {
                let (n_cmp, o_cmp) = if c.paths.len() > 1 { ((e.bytes as f64 * total_orig as f64 / orig1.max(1) as f64) as u64, total_orig) } else { (e.bytes as u64, orig1) };
                let pct = (1.0 - n_cmp as f64 / o_cmp.max(1) as f64) * 100.0;
                let note = if c.mode == 1 { format!(" · quality ≈ {}", e.quality) } else { String::new() };
                let verdict = if pct >= 1.0 {
                    text(format!("≈ {pct:.0}% smaller · {}{note}", c.target_fmt())).font(STRONG).color(col_of(GOOD))
                } else {
                    text("Already well compressed — try a smaller size or WebP").font(STRONG).color(col_of(WARN))
                };
                col().key("result").gap(4.0).child(text(format!("{}  →  ≈ {}", human_size(o_cmp), human_size(n_cmp))).font(font(18.0, 650.0)).color(col_of(th.label()))).child(verdict)
            }
            None => dim("Estimating…", th).key("result"),
        };
        col()
            .w_full()
            .grow(1.0)
            .gap(12.0)
            .child(mode)
            .child(how)
            .child(size)
            .child(webp)
            .child(section("Before and after, at 100%", th))
            .child(previews)
            .child(result)
            .child(rune::spacer())
            .child(footer(s, "Compress", true, None))
    }
    fn enter(s: &mut S, tx: &mut Tx) {
        let paths = s.t.paths.clone();
        let side = SIDES[s.t.side].1;
        let fmt = s.t.target_fmt();
        if s.t.mode == 1 {
            let (label, limit) = TARGETS[s.t.target];
            s.submit(tx, "tool:Compress", &paths.clone(), "Compressing…", format!("target {label}"), move || {
                let mut outs = vec![];
                for p in &paths {
                    let im = utsuri_engine::imgio::open_image(p)?;
                    let im = match side {
                        Some(sd) if im.width().max(im.height()) > sd => im.resize(sd, sd, image::imageops::FilterType::Lanczos3),
                        _ => im,
                    };
                    let (q, s2) = imaging::plan_target_size(&im, fmt, limit)?;
                    let cap = match (side, s2) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                    outs.push(imaging::compress_to(p, q, cap, fmt)?);
                }
                Ok(outs)
            });
        } else {
            let q = s.t.quality;
            s.submit(tx, "tool:Compress", &paths.clone(), "Compressing…", format!("quality {q}"), move || paths.iter().map(|p| imaging::compress_to(p, q, side, fmt)).collect());
        }
    }
}
