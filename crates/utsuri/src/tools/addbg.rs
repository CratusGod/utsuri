//! Add BG: put the picture on a gradient backdrop with padding, round corners and a shadow.

use super::subtitle;
use crate::ui::frame::{Tool, ToolS, footer};
use crate::ui::*;
use image::DynamicImage;
use rune::{Cx, Tx};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use utsuri_engine::imaging::{Background, add_background};

const PRESETS: &[(&str, &[u32])] = &[
    ("Sunset", &[0xffb36b, 0xff5f6d, 0xa24bff]),
    ("Prism", &[0xff6b9d, 0xffb36b, 0xffe66b, 0x7dffb0, 0x6bd5ff, 0x8a7bff]),
    ("Peach", &[0xffd6a5, 0xff9a8b]),
    ("Aurora", &[0x00d2ff, 0x7a5cff, 0xff5fd2]),
    ("Mint", &[0xa8ffce, 0x3ad6c6]),
    ("Ocean", &[0x43cbff, 0x2058d6]),
    ("Dusk", &[0x2b1055, 0xd53a9d]),
    ("Lemon", &[0xfff59d, 0xffb74d]),
    ("Night", &[0x0f2027, 0x2c5364]),
    ("Rose", &[0xffdde1, 0xee9ca7]),
];
const ASPECTS: &[&str] = &["Auto", "1:1", "4:3", "16:9", "9:16", "4:5"];

fn rgb(v: u32) -> [u8; 3] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

fn color(c: [u8; 3]) -> Color {
    col_of(Rgba(c[0], c[1], c[2], 255))
}

/// Linux: zenity's colour dialog (GTK), when installed.
#[cfg(not(windows))]
fn pick_color(start: [u8; 3]) -> Option<[u8; 3]> {
    let init = format!("rgb({},{},{})", start[0], start[1], start[2]);
    let out = std::process::Command::new("zenity").args(["--color-selection", "--show-palette", "--title=Background colour", "--color", &init]).stdin(std::process::Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_rgb(String::from_utf8_lossy(&out.stdout).trim())
}

/// "rgb(1,2,3)", "rgba(1,2,3,0.5)" or "#rrggbb".
#[cfg(not(windows))]
fn parse_rgb(s: &str) -> Option<[u8; 3]> {
    if let Some(hex) = s.strip_prefix('#') {
        let v = u32::from_str_radix(hex.get(..6)?, 16).ok()?;
        return Some([(v >> 16) as u8, (v >> 8) as u8, v as u8]);
    }
    let inner = s.split_once('(')?.1.trim_end_matches(')');
    let mut it = inner.split(',').map(|p| p.trim().parse::<f32>().ok().map(|v| v.round().clamp(0.0, 255.0) as u8));
    Some([it.next()??, it.next()??, it.next()??])
}

/// Windows' own colour dialog (modal, owned by the active window).
#[cfg(windows)]
fn pick_color(start: [u8; 3]) -> Option<[u8; 3]> {
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::UI::Controls::Dialogs::{CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW};
    use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    static mut CUSTOM: [COLORREF; 16] = [COLORREF(0x00ffffff); 16];
    let rgb = |c: [u8; 3]| COLORREF(c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16);
    unsafe {
        let mut cc = CHOOSECOLORW {
            lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
            hwndOwner: GetActiveWindow(),
            rgbResult: rgb(start),
            lpCustColors: std::ptr::addr_of_mut!(CUSTOM) as *mut COLORREF,
            Flags: CC_FULLOPEN | CC_RGBINIT,
            ..Default::default()
        };
        ChooseColorW(&mut cc).as_bool().then(|| {
            let v = cc.rgbResult.0;
            [v as u8, (v >> 8) as u8, (v >> 16) as u8]
        })
    }
}

pub struct AddBg {
    paths: Vec<PathBuf>,
    pv: Arc<DynamicImage>,
    k: f32,
    bg: Background,
    preset: Option<usize>,
    aspect: usize,
    shown: Option<rune::Pixels>,
    work: rune::Latest<rune::Pixels>,
}

impl AddBg {
    pub fn new(paths: Vec<PathBuf>) -> anyhow::Result<(Self, String)> {
        let full = utsuri_engine::imgio::open_image(&paths[0])?;
        let long = full.width().max(full.height());
        let pv = if long > 900 { full.resize(900, 900, image::imageops::FilterType::Triangle) } else { full };
        let k = pv.width().max(pv.height()) as f32 / long as f32;
        let bg = Background { colors: PRESETS[0].1.iter().map(|v| rgb(*v)).collect(), ..Default::default() };
        let sub = subtitle(&paths);
        Ok((AddBg { paths, pv: Arc::new(pv), k, bg, preset: Some(0), aspect: 0, shown: None, work: rune::Latest::default().debounce(Duration::from_millis(30)) }, sub))
    }

    fn scaled(&self) -> Background {
        Background { radius: (self.bg.radius as f32 * self.k).round() as u32, aspect: ASPECTS[self.aspect].into(), ..self.bg.clone() }
    }

    fn kick(&self) {
        let (pv, bg) = (self.pv.clone(), self.scaled());
        self.work.request((), move |_| pixels_of(&DynamicImage::ImageRgba8(add_background(&pv, &bg))));
    }
}

type S = ToolS<AddBg>;

fn bg_slider(key: &'static str, name: &str, v: f32, lo: f32, hi: f32, shown: String, th: &Theme, set: fn(&mut Background, f32)) -> El {
    row().items_center().gap(10.0).child(el().w(70.0).child(label(name, th))).child(slider(key, v, lo, hi, 190.0, th, move |s: &mut S, x| {
        set(&mut s.t.bg, x);
        s.t.kick();
    }))
    .child(dim(shown, th).w(52.0))
}

impl Tool for AddBg {
    fn title(&self) -> String {
        "Add background".into()
    }
    fn size(&self) -> (f32, f32) {
        (980.0, 760.0)
    }
    fn setup(s: &mut S, waker: rune::Waker) {
        s.t.work.set_waker(waker);
        s.t.kick();
    }
    fn poll(s: &mut S, _tx: &mut Tx) -> bool {
        match s.t.work.take() {
            Some(px) => {
                s.t.shown = Some(px);
                true
            }
            None => false,
        }
    }
    fn body(s: &S, _cx: &mut Cx<S>) -> El {
        let th = &s.theme;
        let b = &s.t;
        let mut presets = row().gap(8.0).flex_wrap();
        for (i, (name, cs)) in PRESETS.iter().enumerate() {
            let stops: Vec<(f32, Color)> = cs.iter().enumerate().map(|(k, v)| (k as f32 / (cs.len() - 1).max(1) as f32, color(rgb(*v)))).collect();
            let on = b.preset == Some(i);
            presets = presets.child(
                el().key(format!("preset/{i}"))
                    .size(34.0, 34.0)
                    .radius(17.0)
                    .linear_gradient(2.36, stops)
                    .border(if on { 2.5 } else { 1.0 }, if on { Color::WHITE } else { white(0.25) })
                    .cursor(rune::Cursor::Pointer)
                    .help(*name)
                    .on_press(move |s: &mut S| {
                        s.t.preset = Some(i);
                        s.t.bg.colors = PRESETS[i].1.iter().map(|v| rgb(*v)).collect();
                        s.t.kick();
                    }),
            );
        }
        let n = b.bg.colors.len();
        let mut chips = row().gap(6.0).items_center().flex_wrap();
        for (i, c) in b.bg.colors.iter().enumerate() {
            let mut chip = row().gap(2.0).items_center().child(
                el().key(format!("color/{i}"))
                    .size(30.0, 30.0)
                    .radius(8.0)
                    .fill(color(*c))
                    .border(1.0, white(0.4))
                    .cursor(rune::Cursor::Pointer)
                    .help("Change this colour")
                    .on_press(move |s: &mut S| {
                        if let Some(nc) = pick_color(s.t.bg.colors[i]) {
                            s.t.bg.colors[i] = nc;
                            s.t.preset = None;
                            s.t.kick();
                        }
                    }),
            );
            if n > 1 {
                chip = chip.child(el().key(format!("color/{i}/remove")).size(18.0, 18.0).radius(9.0).center().hover(white(0.15)).cursor(rune::Cursor::Pointer).child(text("×").font(SMALL).color(col_of(th.text_dim))).on_press(move |s: &mut S| {
                    s.t.bg.colors.remove(i);
                    s.t.preset = None;
                    s.t.kick();
                }));
            }
            chips = chips.child(chip);
        }
        if n < 6 {
            chips = chips.child(pill("add-color", "+", th).on_press(|s: &mut S| {
                let last = *s.t.bg.colors.last().unwrap();
                s.t.bg.colors.push([255 - last[0], 255 - last[1], 255 - last[2]]);
                s.t.preset = None;
                s.t.kick();
            }));
        }
        let left = col().grow(1.0).min_w(0.0).gap(10.0).child(section("Background", th)).child(presets).child(chips);
        let bgv = &b.bg;
        let right = col()
            .gap(8.0)
            .child(bg_slider("padding", "Padding", bgv.padding * 100.0, 0.0, 40.0, format!("{:.0}%", bgv.padding * 100.0), th, |g, v| g.padding = v.round() / 100.0))
            .child(bg_slider("corners", "Corners", bgv.radius as f32, 0.0, 200.0, format!("{} px", bgv.radius), th, |g, v| g.radius = v.round() as u32))
            .child(bg_slider("shadow", "Shadow", bgv.shadow * 100.0, 0.0, 100.0, format!("{:.0}%", bgv.shadow * 100.0), th, |g, v| g.shadow = v.round() / 100.0))
            .child(bg_slider("angle", "Angle", bgv.angle, 0.0, 360.0, format!("{:.0}°", bgv.angle), th, |g, v| g.angle = v.round()))
            .child(row().items_center().gap(10.0).child(el().w(70.0).child(label("Size", th))).child(toggles("aspect", ASPECTS, b.aspect, th, |s: &mut S, i| {
                s.t.aspect = i;
                s.t.kick();
            })));
        let preview = picture("preview", b.shown.as_ref()).w_full().grow(1.0).min_h(160.0);
        let copy = pill("copy", "Copy", th).help("Copy the result to the clipboard").on_press(|s: &mut S| {
            let (src, bg) = (s.t.paths[0].clone(), Background { aspect: ASPECTS[s.t.aspect].into(), ..s.t.bg.clone() });
            if let Some(app) = s.app.clone() {
                app.update(move |h, _| {
                    h.spawn("x:copy".into(), vec![], "Copying…".into(), Box::new(move || {
                        let im = utsuri_engine::imgio::open_image(&src)?;
                        let out = add_background(&im, &bg);
                        Ok(utsuri_engine::report::Report::new("Image copied", "with background", vec![], vec![src]).clip(utsuri_engine::report::Clip::Image(out)))
                    }))
                });
            }
        });
        col().w_full().grow(1.0).min_h(0.0).gap(12.0).child(row().w_full().gap(28.0).child(left).child(right)).child(preview).child(footer(s, "Save with Background", true, Some(copy)))
    }
    fn enter(s: &mut S, tx: &mut Tx) {
        let src = s.t.paths[0].clone();
        let bg = Background { aspect: ASPECTS[s.t.aspect].into(), ..s.t.bg.clone() };
        s.submit(tx, "tool:Add BG", &[src.clone()], "Saving…", String::new(), move || Ok(vec![utsuri_engine::imaging::add_background_and_save(&src, &bg)?]));
    }
}
