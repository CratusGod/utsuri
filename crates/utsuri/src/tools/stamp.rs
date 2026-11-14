//! Stamp: a text watermark or an image stamp on chosen pages, with a live preview.
//! Ported from pdfwin.py (WatermarkWindow).

use super::{name, subtitle};
use crate::ui::frame::{Tool, ToolS, footer};
use crate::ui::*;
use rune::{Cx, Tx};
use std::path::PathBuf;
use utsuri_engine::pdf::{CORNERS, TextMark};

const COLORS: &[[u8; 3]] = &[[153, 25, 25], [20, 20, 20], [120, 120, 120], [30, 90, 200], [220, 40, 40], [30, 150, 70]];
const POSITIONS: &[&str] = &["Centre", "Top left", "Top right", "Bottom left", "Bottom right"];

pub struct Stamp {
    paths: Vec<PathBuf>,
    mode: usize,
    text: rune::TextEdit,
    size: f32,
    opacity: f32,
    angle: f32,
    color: usize,
    pos: usize,
    pages: rune::TextEdit,
    image: Option<PathBuf>,
    scale: f32,
    page_px: Option<rune::Pixels>,
    page_size: (f32, f32),
    page_work: rune::Latest<Option<rune::Pixels>>,
    mark: Option<(rune::Pixels, f32, f32)>,
    err: Option<String>,
    count: usize,
}

impl Stamp {
    pub fn new(paths: Vec<PathBuf>) -> anyhow::Result<(Self, String)> {
        let doc = utsuri_engine::pdf::open(&paths[0])?;
        let count = doc.get_pages().len();
        let pid = *doc.get_pages().values().next().ok_or_else(|| anyhow::anyhow!("no pages"))?;
        let v = utsuri_engine::pdf::view(&doc, pid);
        let sub = subtitle(&paths);
        let mut s = Stamp {
            paths,
            mode: 0,
            text: rune::TextEdit::new("CONFIDENTIAL"),
            size: 60.0,
            opacity: 0.2,
            angle: 45.0,
            color: 0,
            pos: 0,
            pages: rune::TextEdit::new(""),
            image: None,
            scale: 0.25,
            page_px: None,
            page_size: (v.w, v.h),
            page_work: rune::Latest::default(),
            mark: None,
            err: None,
            count,
        };
        s.refresh_mark();
        Ok((s, sub))
    }

    fn where_(&self) -> &'static str {
        CORNERS[self.pos]
    }

    fn ranges_ok(&self) -> Result<(), String> {
        utsuri_engine::pdf::parse_ranges(&self.pages.text, self.count).map(|_| ()).map_err(|e| utsuri_engine::friendly(&e))
    }

    /// Rebuild the mark image from the current settings.
    fn refresh_mark(&mut self) {
        self.err = None;
        let r: anyhow::Result<(image::RgbaImage, f32, f32)> = if self.mode == 0 {
            let angle = if self.pos == 0 { self.angle } else { 0.0 };
            utsuri_engine::pdf::text_mark(&self.text.text, self.size, self.opacity, angle, COLORS[self.color])
        } else {
            match &self.image {
                None => Err(anyhow::anyhow!("Pick an image to stamp")),
                Some(p) => utsuri_engine::imgio::open_preview(p, 600).map(|im| {
                    let im = utsuri_engine::pdf::faded(&im.to_rgba8(), self.opacity);
                    let w = self.page_size.0 * self.scale;
                    let h = w * im.height() as f32 / im.width() as f32;
                    (im, w, h)
                }),
            }
        };
        match r {
            Ok((im, w, h)) => self.mark = Some((pixels_of(&image::DynamicImage::ImageRgba8(im)), w, h)),
            Err(e) => {
                self.mark = None;
                self.err = Some(utsuri_engine::friendly(&e));
            }
        }
    }
}

type S = ToolS<Stamp>;

impl Tool for Stamp {
    fn title(&self) -> String {
        "Stamp".into()
    }
    fn size(&self) -> (f32, f32) {
        (1000.0, 720.0)
    }
    fn setup(s: &mut S, waker: rune::Waker) {
        s.t.page_work.set_waker(waker);
        let p = s.t.paths[0].clone();
        s.t.page_work.request((), move |_| utsuri_engine::pdf::render_width(&p, None, 0, 700).ok().map(|im| pixels_of(&im)));
    }
    fn poll(s: &mut S, _tx: &mut Tx) -> bool {
        match s.t.page_work.take() {
            Some(px) => {
                s.t.page_px = px;
                true
            }
            None => false,
        }
    }
    fn body(s: &S, cx: &mut Cx<S>) -> El {
        let th = &s.theme;
        let st = &s.t;
        let line = |l: &str, e: El| row().items_center().gap(10.0).child(el().w(72.0).child(label(l, th))).child(e);
        let mut left = col().w(420.0).gap(12.0).child(toggles("mode", &["Text", "Image"], st.mode, th, |s: &mut S, i| {
            s.t.mode = i;
            s.t.refresh_mark();
        }));
        if st.mode == 0 {
            left = left
                .child(rune_kit::TextInput::new(&st.text).placeholder("Watermark text").width(Some(400.0)).on_edit(|s: &mut S| &mut s.t.text).on_change(|s: &mut S, _| s.t.refresh_mark()).build(&s.kit).key("text"))
                .child(line("Size", row().items_center().gap(10.0).child(slider("size", st.size, 12.0, 200.0, 220.0, th, |s: &mut S, v| {
                    s.t.size = v.round();
                    s.t.refresh_mark();
                })).child(dim(format!("{:.0} pt", st.size), th))))
                .child(line("Angle", {
                    let sl = slider("angle", st.angle, -90.0, 90.0, 220.0, th, |s: &mut S, v| {
                        if s.t.pos == 0 {
                            s.t.angle = v.round();
                            s.t.refresh_mark();
                        }
                    });
                    row().items_center().gap(10.0).child(if st.pos == 0 { sl } else { sl.dim(0.4) }).child(dim(format!("{:.0}°", st.angle), th))
                }));
            let mut colors = row().gap(6.0);
            for (i, c) in COLORS.iter().enumerate() {
                let on = st.color == i;
                colors = colors.child(el().key(format!("color/{i}")).size(22.0, 22.0).radius(11.0).fill(col_of(Rgba(c[0], c[1], c[2], 255))).border(if on { 2.5 } else { 1.0 }, Color::WHITE).cursor(rune::Cursor::Pointer).on_press(move |s: &mut S| {
                    s.t.color = i;
                    s.t.refresh_mark();
                }));
            }
            left = left.child(line("Colour", colors));
        } else {
            left = left
                .child(row().items_center().gap(10.0).child(pill("choose", "Choose image…", th).on_press(|s: &mut S| {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff", "heic", "avif"]).pick_file() {
                        s.t.image = Some(p);
                        s.t.refresh_mark();
                    }
                })).child(dim(st.image.as_ref().map(|p| name(p)).unwrap_or_else(|| "No image yet".into()), th)))
                .child(line("Width", row().items_center().gap(10.0).child(slider("scale", st.scale * 100.0, 5.0, 100.0, 220.0, th, |s: &mut S, v| {
                    s.t.scale = v.round() / 100.0;
                    s.t.refresh_mark();
                })).child(dim(format!("{:.0}% of the page", st.scale * 100.0), th))));
        }
        left = left
            .child(line("Opacity", row().items_center().gap(10.0).child(slider("opacity", st.opacity * 100.0, 5.0, 100.0, 220.0, th, |s: &mut S, v| {
                s.t.opacity = v.round() / 100.0;
                s.t.refresh_mark();
            })).child(dim(format!("{:.0}%", st.opacity * 100.0), th))))
            .child(col().w_full().gap(6.0).child(label("Position", th)).child(toggles("pos", POSITIONS, st.pos, th, |s: &mut S, i| {
                s.t.pos = i;
                s.t.refresh_mark();
            }).w_full()))
            .child(line("Pages", rune_kit::TextInput::new(&st.pages).placeholder(format!("all {} (or e.g. 1-3, 7)", st.count)).width(Some(220.0)).on_edit(|s: &mut S| &mut s.t.pages).build(&s.kit).key("pages")));
        if let Err(e) = st.ranges_ok() {
            left = left.child(text(e).wrap().w_full().font(SMALL).color(col_of(WARN)));
        }
        if let Some(e) = &st.err {
            left = left.child(text(e.clone()).key("error").wrap().w_full().font(SMALL).color(col_of(WARN)));
        }
        // preview: the page, and the mark where it will land
        let fit = cx.layout_size("canvas").map(|a| Fit::new(a, st.page_size));
        let mut overlays = vec![];
        if let Some(f) = fit {
            let r = f.rect();
            overlays.push(boxed(r[0], r[1], r[2], r[3]).fill(Color::WHITE).shadow(4.0, 16.0, black(0.4)));
            if let Some(p) = &st.page_px {
                overlays.push(boxed(r[0], r[1], r[2], r[3]).pixels(p));
            }
            if let Some((px, w, h)) = &st.mark {
                let (pw, ph) = st.page_size;
                let k = 1f32.min((pw - 8.0) / w).min((ph - 8.0) / h);
                let (ww, hh) = (w * k, h * k);
                let margin = 28f32.min((pw - ww) / 2.0).min((ph - hh) / 2.0);
                let v = utsuri_engine::pdf::View { crop: [0.0, 0.0, pw, ph], rot: 0, w: pw, h: ph };
                let (u, vv) = utsuri_engine::pdf::anchor(&v, ww, hh, st.where_(), margin);
                let (x, y) = f.to_view(u, vv);
                overlays.push(boxed(x, y, ww * f.s, hh * f.s).key("mark").pixels(px));
            }
        }
        let preview = canvas_area("canvas", None, fit, overlays);
        let ok = st.mark.is_some() && st.ranges_ok().is_ok();
        col().w_full().grow(1.0).min_h(0.0).gap(12.0).child(row().w_full().grow(1.0).min_h(0.0).gap(20.0).child(left).child(col().grow(1.0).min_w(0.0).h_full().child(preview))).child(footer(s, "Stamp", ok, None))
    }
    fn enter(s: &mut S, tx: &mut Tx) {
        if s.t.mark.is_none() || s.t.ranges_ok().is_err() {
            return;
        }
        let src = s.t.paths[0].clone();
        let (where_, pages, opacity) = (s.t.where_(), s.t.pages.text.clone(), s.t.opacity);
        if s.t.mode == 0 {
            let (text, size, angle, color) = (s.t.text.text.clone(), s.t.size, s.t.angle, COLORS[s.t.color]);
            s.submit(tx, "tool:Watermarked", &[src.clone()], "Stamping…", String::new(), move || {
                Ok(vec![utsuri_engine::pdf::watermark_text(&src, &TextMark { text: &text, size, opacity, angle, color, where_, pages: &pages })?])
            });
        } else if let Some(img) = s.t.image.clone() {
            let scale = s.t.scale;
            s.submit(tx, "tool:Stamped", &[src.clone()], "Stamping…", String::new(), move || Ok(vec![utsuri_engine::pdf::stamp_image(&src, &img, scale, where_, &pages, opacity)?]));
        }
    }
}
