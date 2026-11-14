//! Organize pages: reorder, rotate, delete, duplicate, add blank pages, and cut into
//! several files, across one or more PDFs. Ported from pdfwin.py (OrganizeWindow).

use super::subtitle;
use crate::ui::frame::{Tool, ToolS, footer};
use crate::ui::*;
use rune::{Cx, Tx};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq)]
struct Item {
    src: usize,
    page: Option<usize>,
    rot: i64,
}

pub struct Organize {
    paths: Vec<PathBuf>,
    items: Vec<Item>,
    selected: Vec<bool>,
    /// item positions where a new file starts
    cuts: Vec<usize>,
    /// one picture per source page, filled in by a worker
    thumbs: Vec<Vec<rune::Pixels>>,
    anchor: Option<usize>,
    /// "1-3, 7": Enter selects those pages
    pick: rune::TextEdit,
    pick_bad: bool,
    /// the thumbnail worker stops once this is dropped (window closed)
    _alive: std::sync::Arc<()>,
}

const THUMB_W: u32 = 140;

fn pages_of(paths: &[PathBuf]) -> anyhow::Result<(Vec<Item>, Vec<usize>)> {
    let mut items = vec![];
    let mut counts = vec![];
    for (si, p) in paths.iter().enumerate() {
        let n = utsuri_engine::pdf::open(p)?.get_pages().len();
        counts.push(n);
        items.extend((0..n).map(|i| Item { src: si, page: Some(i), rot: 0 }));
    }
    Ok((items, counts))
}

impl Organize {
    pub fn new(paths: Vec<PathBuf>) -> anyhow::Result<(Self, String)> {
        let (items, counts) = pages_of(&paths)?;
        let thumbs: Vec<Vec<rune::Pixels>> = counts.iter().map(|n| (0..*n).map(|_| rune::Pixels::new()).collect()).collect();
        // thumbnails off the UI thread, in page order; each picture shows as it lands
        let (pp, tt) = (paths.clone(), thumbs.clone());
        let alive = std::sync::Arc::new(());
        let watch = std::sync::Arc::downgrade(&alive);
        std::thread::spawn(move || {
            for (si, p) in pp.iter().enumerate() {
                for (i, px) in tt[si].iter().enumerate() {
                    if watch.strong_count() == 0 {
                        return;
                    }
                    if let Ok(im) = utsuri_engine::pdf::render_width(p, None, i, THUMB_W * 2) {
                        let rgba = im.to_rgba8();
                        px.set(rgba.width(), rgba.height(), rgba.into_raw());
                    }
                }
            }
        });
        let n = items.len();
        let sub = format!("{} · {} pages", subtitle(&paths), n);
        Ok((Organize { paths, items, selected: vec![false; n], cuts: vec![], thumbs, anchor: None, pick: rune::TextEdit::new(""), pick_bad: false, _alive: alive }, sub))
    }

    /// Select the pages typed in the box (1-based positions); empty clears the selection.
    fn pick_pages(&mut self) {
        let n = self.items.len();
        match parse_pages(&self.pick.text, n) {
            Some(list) => {
                self.selected = vec![false; n];
                for i in list {
                    self.selected[i] = true;
                }
                self.pick_bad = false;
            }
            None => self.pick_bad = true,
        }
    }

    fn sel(&self) -> Vec<usize> {
        self.selected.iter().enumerate().filter(|(_, s)| **s).map(|(i, _)| i).collect()
    }

    fn shift_cuts(&mut self, at: usize, by: isize) {
        for c in &mut self.cuts {
            if *c > at || (by < 0 && *c == at + 1) {
                *c = (*c as isize + by).max(0) as usize;
            }
        }
        self.cuts.retain(|c| *c > 0 && *c < self.items.len());
        self.cuts.sort();
        self.cuts.dedup();
    }

    fn rotate(&mut self, by: i64) {
        for i in self.sel() {
            self.items[i].rot = (self.items[i].rot + by) % 360;
        }
    }

    fn delete(&mut self) {
        for i in self.sel().into_iter().rev() {
            self.items.remove(i);
            self.selected.remove(i);
            self.shift_cuts(i, -1);
        }
    }

    fn duplicate(&mut self) {
        for i in self.sel().into_iter().rev() {
            self.items.insert(i + 1, self.items[i]);
            self.selected.insert(i + 1, false);
            self.shift_cuts(i, 1);
        }
    }

    fn blank(&mut self) {
        let sel = self.sel();
        let at = sel.last().map_or(self.items.len(), |i| i + 1);
        let src = sel.last().map_or(0, |i| self.items[*i].src);
        self.items.insert(at, Item { src, page: None, rot: 0 });
        self.selected.insert(at, false);
        if at > 0 {
            self.shift_cuts(at - 1, 1);
        }
    }

    fn step(&mut self, back: bool) {
        let sel = self.sel();
        if sel.len() != 1 {
            return;
        }
        let i = sel[0];
        let j = if back { i.checked_sub(1) } else { (i + 1 < self.items.len()).then_some(i + 1) };
        if let Some(j) = j {
            self.items.swap(i, j);
            self.selected.swap(i, j);
        }
    }

    fn toggle_cut(&mut self) {
        let sel = self.sel();
        if sel.len() != 1 || sel[0] == 0 {
            return;
        }
        let i = sel[0];
        if let Some(k) = self.cuts.iter().position(|c| *c == i) {
            self.cuts.remove(k);
        } else {
            self.cuts.push(i);
            self.cuts.sort();
        }
    }

    fn click(&mut self, i: usize, mods: rune::Mods) {
        if mods.shift && self.anchor.is_some() {
            let a = self.anchor.unwrap();
            for k in a.min(i)..=a.max(i) {
                self.selected[k] = true;
            }
        } else if mods.ctrl || mods.cmd {
            self.selected[i] = !self.selected[i];
            self.anchor = Some(i);
        } else {
            let only = self.selected[i] && self.sel().len() == 1;
            self.selected.iter_mut().for_each(|s| *s = false);
            self.selected[i] = !only;
            self.anchor = Some(i);
        }
    }
}

type S = ToolS<Organize>;

impl Tool for Organize {
    fn title(&self) -> String {
        "Organize pages".into()
    }
    fn size(&self) -> (f32, f32) {
        (1000.0, 780.0)
    }
    fn key(s: &mut S, k: &rune::KeyEvent, _tx: &mut Tx) -> bool {
        if matches!(k.key, rune::Key::Delete) && !s.t.sel().is_empty() {
            s.t.delete();
            return true;
        }
        if k.is_command('a') {
            s.t.selected.iter_mut().for_each(|x| *x = true);
            return true;
        }
        false
    }
    fn body(s: &S, cx: &mut Cx<S>) -> El {
        let th = &s.theme;
        let o = &s.t;
        let per_row = cx.layout_with("pages", |r| (((r[2] - 8.0) / (THUMB_W as f32 + 26.0)).floor() as usize).max(1)).unwrap_or(5);
        let sel = o.sel();
        let any = !sel.is_empty();
        let one = sel.len() == 1;
        let bar = row()
            .w_full()
            .gap(6.0)
            .flex_wrap()
            .child(pill_on("rot-left", "⟲", any, th).help("Rotate left").on_press(|s: &mut S| s.t.rotate(270)))
            .child(pill_on("rot-right", "⟳", any, th).help("Rotate right").on_press(|s: &mut S| s.t.rotate(90)))
            .child(pill_on("delete", "Delete", any, th).on_press(|s: &mut S| s.t.delete()))
            .child(pill_on("duplicate", "Duplicate", any, th).on_press(|s: &mut S| s.t.duplicate()))
            .child(pill("blank", "Blank page", th).help("After the selected page (or at the end)").on_press(|s: &mut S| s.t.blank()))
            .child(pill_on("back", "◀ Move", one && sel[0] > 0, th).on_press(|s: &mut S| s.t.step(true)))
            .child(pill_on("forward", "Move ▶", one && sel[0] + 1 < o.items.len(), th).on_press(|s: &mut S| s.t.step(false)))
            .child(pill_on("cut", "✂ New file here", one && sel[0] > 0, th).help("Start a new PDF at this page").on_press(|s: &mut S| s.t.toggle_cut()))
            .child(pill("select-all", "Select all", th).on_press(|s: &mut S| {
                let all = s.t.selected.iter().all(|x| *x);
                s.t.selected.iter_mut().for_each(|x| *x = !all);
            }))
            .child(pill_on("extract", "Extract selection", any, th).help("Save only the selected pages as a new PDF").on_press_tx(|s: &mut S, tx: &mut Tx| extract(s, tx)));
        let picker = row()
            .w_full()
            .gap(10.0)
            .items_center()
            .child(label("Select pages", th))
            .child(
                rune_kit::TextInput::new(&o.pick)
                    .placeholder("e.g. 1-3, 7")
                    .width(Some(200.0))
                    .invalid(o.pick_bad)
                    .on_edit(|s: &mut S| &mut s.t.pick)
                    .on_change(|s: &mut S, _| s.t.pick_bad = false)
                    // Enter here selects; it must never press Save
                    .on_submit(|s: &mut S, _| s.t.pick_pages())
                    .build(&s.kit)
                    .key("pick"),
            )
            .child(dim(if o.pick_bad { format!("Use page numbers from 1 to {}", o.items.len()) } else if any { format!("{} selected", sel.len()) } else { "Enter selects · empty clears".to_string() }, th).key("pick-hint"));
        let mut cells: Vec<El> = vec![];
        for (i, it) in o.items.iter().enumerate() {
            if o.cuts.contains(&i) {
                cells.push(el().size(4.0, 190.0).radius(2.0).fill(col_of(th.accent_a)));
            }
            let on = o.selected[i];
            let pic = match it.page {
                Some(pg) => {
                    let px = &o.thumbs[it.src][pg];
                    let mut p = el().w_full().h_full().pixels_fit(px, rune::SurfaceFit::Fit);
                    if it.rot != 0 {
                        p = p.rotate((it.rot as f32).to_radians());
                    }
                    el().size(THUMB_W as f32, 160.0).center().child(p)
                }
                None => el().size(THUMB_W as f32, 160.0).center().child(el().size(100.0, 140.0).radius(2.0).fill(Color::WHITE).center().child(text("blank").font(SMALL).color(Color { r: 0.3, g: 0.3, b: 0.3, a: 1.0 }))),
            };
            let caption = match it.page {
                Some(pg) if o.paths.len() > 1 => format!("{}  ·  {}:{}", i + 1, it.src + 1, pg + 1),
                Some(pg) => format!("{}  ·  p{}", i + 1, pg + 1),
                None => format!("{}  ·  blank", i + 1),
            };
            cells.push(
                col()
                    .key(format!("page/{i}"))
                    .w(THUMB_W as f32 + 12.0)
                    .padding(6.0)
                    .gap(4.0)
                    .items_center()
                    .radius(10.0)
                    .fill(if on { col_of(th.accent_b).alpha(0.32) } else { white(0.04) })
                    .border(if on { 2.0 } else { 1.0 }, if on { col_of(th.accent_a) } else { white(0.12) })
                    .cursor(rune::Cursor::Pointer)
                    .child(pic)
                    .child(dim(caption, th))
                    .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                        let m = tx.mods();
                        s.t.click(i, m);
                    }),
            );
        }
        let mut grid = col().w_full().gap(14.0).py(4.0);
        let mut it = cells.into_iter().peekable();
        while it.peek().is_some() {
            let mut r = row().gap(14.0);
            for c in it.by_ref().take(per_row) {
                r = r.child(c);
            }
            grid = grid.child(r);
        }
        let pages = col().key("pages").w_full().grow(1.0).min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(grid);
        let files = o.cuts.len() + 1;
        let label = if files > 1 { format!("Save {files} PDFs") } else { "Save".to_string() };
        let reset = pill("reset", "Reset", th).on_press(|s: &mut S| {
            if let Ok((items, _)) = pages_of(&s.t.paths) {
                s.t.selected = vec![false; items.len()];
                s.t.items = items;
                s.t.cuts.clear();
            }
        });
        col().w_full().grow(1.0).min_h(0.0).gap(12.0).child(bar).child(picker).child(pages).child(footer(s, &label, !o.items.is_empty(), Some(reset)))
    }
    fn enter(s: &mut S, tx: &mut Tx) {
        if s.t.items.is_empty() {
            return;
        }
        let (paths, cuts) = (s.t.paths.clone(), s.t.cuts.clone());
        let items: Vec<utsuri_engine::pdf::PageRef> = s.t.items.iter().map(|i| (i.src, i.page, i.rot)).collect();
        let n = items.len();
        s.submit(tx, "tool:Organize", &paths.clone(), "Saving…", format!("{n} page{}", if n != 1 { "s" } else { "" }), move || utsuri_engine::pdf::organize(&paths, &items, "Organized", &cuts));
    }
}

/// "1-3, 7" → zero-based positions, each within 1..=n. Empty → none. None if unreadable.
fn parse_pages(s: &str, n: usize) -> Option<Vec<usize>> {
    let mut out = vec![];
    for part in s.split([',', ';', ' ']).map(str::trim).filter(|p| !p.is_empty()) {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.trim().parse::<usize>().ok()?, b.trim().parse::<usize>().ok()?),
            None => {
                let v = part.parse::<usize>().ok()?;
                (v, v)
            }
        };
        if a == 0 || b == 0 || a > n || b > n {
            return None;
        }
        for v in a.min(b)..=a.max(b) {
            if !out.contains(&(v - 1)) {
                out.push(v - 1);
            }
        }
    }
    out.sort();
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::parse_pages;

    #[test]
    fn page_ranges() {
        assert_eq!(parse_pages("1-3, 7", 8), Some(vec![0, 1, 2, 6]));
        assert_eq!(parse_pages("3-1", 5), Some(vec![0, 1, 2]));
        assert_eq!(parse_pages("", 5), Some(vec![]));
        assert_eq!(parse_pages("9", 5), None);
        assert_eq!(parse_pages("a", 5), None);
        assert_eq!(parse_pages("2 4", 5), Some(vec![1, 3]));
    }
}

/// Save only the selected pages, in their order, as a new PDF.
fn extract(s: &mut S, tx: &mut Tx) {
    let sel = s.t.sel();
    if sel.is_empty() {
        return;
    }
    let paths = s.t.paths.clone();
    let items: Vec<utsuri_engine::pdf::PageRef> = sel.iter().map(|i| {
        let it = s.t.items[*i];
        (it.src, it.page, it.rot)
    }).collect();
    let n = items.len();
    s.submit(tx, "tool:Organize", &paths.clone(), "Extracting…", format!("{n} page{}", if n != 1 { "s" } else { "" }), move || utsuri_engine::pdf::organize(&paths, &items, "Extracted", &[]));
}
