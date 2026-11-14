//! Metadata: what's hidden in a photo or PDF (camera, date, GPS), and a private copy
//! without it.

use super::subtitle;
use crate::ui::frame::{Tool, ToolS, footer};
use crate::ui::*;
use rune::{Cx, Tx};
use std::path::PathBuf;

pub struct Metadata {
    paths: Vec<PathBuf>,
    pdf: bool,
    summary: Vec<(String, String)>,
    all: Vec<(String, String)>,
    count: usize,
    gps: Option<String>,
    show_all: bool,
}

impl Metadata {
    pub fn new(paths: Vec<PathBuf>, pdf: bool) -> anyhow::Result<(Self, String)> {
        let p = &paths[0];
        let (summary, all, count, gps) = if pdf {
            let all = utsuri_engine::pdf::read_pdf_metadata(p)?;
            let n = all.iter().filter(|(k, _)| k != "Pages").count();
            (vec![], all, n, None)
        } else {
            let summary = utsuri_engine::imaging::metadata_summary(p);
            let all = utsuri_engine::imaging::read_image_metadata(p);
            let (n, _) = utsuri_engine::imaging::count_metadata(p);
            let gps = summary.iter().find(|(k, _)| k == "Location").map(|(_, v)| v.clone());
            (summary, all, n, gps)
        };
        let sub = subtitle(&paths);
        Ok((Metadata { paths, pdf, summary, all, count, gps, show_all: pdf }, sub))
    }
}

type S = ToolS<Metadata>;

fn rows(list: &[(String, String)], th: &Theme, warn_key: Option<&str>) -> El {
    let mut c = col().w_full().gap(6.0);
    for (k, v) in list {
        let warn = warn_key == Some(k.as_str());
        c = c.child(
            row()
                .w_full()
                .gap(16.0)
                .items_start()
                .child(el().w(150.0).child(text(k).font(SMALL).color(col_of(th.text_dim))))
                .child(text(v).wrap().grow(1.0).min_w(0.0).font(if warn { STRONG } else { BODY }).color(col_of(if warn { WARN } else { th.label() }))),
        );
    }
    c
}

impl Tool for Metadata {
    fn title(&self) -> String {
        "Metadata".into()
    }
    fn size(&self) -> (f32, f32) {
        (640.0, 580.0)
    }
    fn body(s: &S, _cx: &mut Cx<S>) -> El {
        let th = &s.theme;
        let m = &s.t;
        let headline = if m.count == 0 {
            text("No hidden metadata found. This file is already clean.").font(STRONG).color(col_of(GOOD))
        } else {
            text(format!("{} hidden field{}{}", m.count, if m.count != 1 { "s" } else { "" }, if m.gps.is_some() { " · includes your GPS location" } else { "" })).font(font(16.0, 650.0)).color(col_of(th.label()))
        };
        let mut c = col().w_full().grow(1.0).min_h(0.0).gap(10.0).child(headline);
        if !m.summary.is_empty() {
            let mut glance = col().w_full().gap(8.0).child(section("At a glance", th)).child(rows(&m.summary, th, Some("Location")));
            if m.gps.is_some() {
                glance = glance.child(row().child(pill("map", "Open map", th).on_press(|s: &mut S| {
                    if let Some(loc) = &s.t.gps {
                        let q = loc.replace(' ', "");
                        let (lat, lon) = (q.split(',').next().unwrap_or(""), q.split(',').nth(1).unwrap_or(""));
                        crate::shell::open_url(&format!("https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map=16/{lat}/{lon}"));
                    }
                })));
            }
            c = c.child(card(glance));
        }
        let toggle_all = row()
            .key("all-toggle")
            .gap(8.0)
            .items_center()
            .cursor(rune::Cursor::Pointer)
            .child(text(if m.show_all { "▾" } else { "▸" }).font(BODY).color(col_of(th.text_dim)))
            .child(section(&format!("All fields ({})", m.all.len()), th))
            .on_press(|s: &mut S| s.t.show_all = !s.t.show_all);
        c = c.child(toggle_all);
        if m.show_all {
            c = c.child(field_box(col().key("all").w_full().grow(1.0).min_h(80.0).padding(12.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(rows(&m.all, th, None))));
        } else {
            c = c.child(rune::spacer());
        }
        let extra = (!m.pdf && m.gps.is_some()).then(|| {
            pill("location-only", "Remove location only", th).on_press_tx(|s: &mut S, tx: &mut Tx| {
                let src = s.t.paths[0].clone();
                s.submit(tx, "tool:Metadata", &[src.clone()], "Removing location…", "location removed".into(), move || Ok(vec![utsuri_engine::imaging::strip_location(&src)?]));
            })
        });
        c.child(footer(s, "Remove all metadata", m.count > 0, extra))
    }
    fn enter(s: &mut S, tx: &mut Tx) {
        if s.t.count == 0 {
            return;
        }
        let src = s.t.paths[0].clone();
        let (n, pdf) = (s.t.count, s.t.pdf);
        s.submit(tx, "tool:Metadata", &[src.clone()], "Removing metadata…", format!("{n} field{} removed", if n != 1 { "s" } else { "" }), move || {
            Ok(vec![if pdf { utsuri_engine::pdf::strip_pdf_metadata(&src)? } else { utsuri_engine::imaging::strip_image_metadata(&src)? }])
        });
    }
}
