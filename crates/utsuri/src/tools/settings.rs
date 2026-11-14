//! Settings and the "How to use" welcome. Ported from settings.py.

use crate::settings::{Settings, TRIGGERS};
use crate::ui::frame::{Tool, ToolS};
use crate::ui::*;
use rune::{AppHandle, Cx, Tx};
use shared::theme::Theme;

const DPIS: &[u32] = &[100, 150, 200, 300];

pub struct SettingsWindow {
    pub s: Settings,
    langs: Vec<String>,
    start: bool,
    shell: bool,
    ocr: rune::TextEdit,
}

impl SettingsWindow {
    pub fn new() -> Self {
        let s = Settings::load();
        let ocr = rune::TextEdit::new(s.ocr_lang.clone());
        SettingsWindow { s, langs: utsuri_engine::ocr::langs(), start: crate::shell::start_at_login(), shell: crate::shell::shell_menu_installed(), ocr }
    }
}

type S = ToolS<SettingsWindow>;

/// Save and tell the app (theme, trigger key, quality and the rest apply at once).
fn changed(s: &mut S) {
    s.t.s.save();
    s.settings_changed();
    let th = shared::theme::by_name(&s.t.s.theme);
    if th.name != s.theme.name {
        s.kit = kit(&th);
        s.theme = th;
    }
}

impl Tool for SettingsWindow {
    fn title(&self) -> String {
        "Settings".into()
    }
    fn size(&self) -> (f32, f32) {
        (600.0, 560.0)
    }
    fn enter(_s: &mut S, tx: &mut Tx) {
        tx.close_window();
    }
    fn body(s: &S, _cx: &mut Cx<S>) -> El {
        let th = &s.theme;
        let st = &s.t.s;
        let lab = |t: &str| el().w(180.0).child(label(t, th));
        let line = |l: &str, ctl: El| row().w_full().items_center().gap(14.0).child(lab(l)).child(ctl);

        let trigger_i = TRIGGERS.iter().position(|(k, _)| *k == st.trigger).unwrap_or(0);
        let trig_labels: Vec<String> = TRIGGERS.iter().map(|(_, l)| format!("{l} + drag")).collect();
        let trig_refs: Vec<&str> = trig_labels.iter().map(String::as_str).collect();
        let trigger = col().gap(4.0).child(toggles("trigger", &trig_refs, trigger_i, th, |s: &mut S, i| {
            s.t.s.trigger = TRIGGERS[i].0.to_string();
            changed(s);
        }));
        let trigger = trigger.child(dim("…pressed while you're dragging a file", th));

        let output = toggles("output", &["Next to the original", "In an \"Utsuri\" folder"], if st.output == "folder" { 1 } else { 0 }, th, |s: &mut S, i| {
            s.t.s.output = if i == 1 { "folder".into() } else { "next".into() };
            changed(s);
        });
        let quality = row().items_center().gap(12.0).child(slider("quality", st.jpg_quality as f32, 50.0, 100.0, 220.0, th, |s: &mut S, v| {
            let q = v.round() as u8;
            if q != s.t.s.jpg_quality {
                s.t.s.jpg_quality = q;
                changed(s);
            }
        }))
        .child(dim(st.jpg_quality.to_string(), th).key("quality/value"));
        let dpi_labels: Vec<String> = DPIS.iter().map(|d| format!("{d} dpi")).collect();
        let dpi_refs: Vec<&str> = dpi_labels.iter().map(String::as_str).collect();
        let dpi = toggles("dpi", &dpi_refs, DPIS.iter().position(|d| *d == st.pdf_dpi).unwrap_or(2), th, |s: &mut S, i| {
            s.t.s.pdf_dpi = DPIS[i];
            changed(s);
        });
        let have = if s.t.langs.is_empty() { "Tesseract isn't installed".to_string() } else { format!("installed: {}", s.t.langs.join(", ")) };
        let ocr = row().items_center().gap(10.0).child(
            rune_kit::TextInput::new(&s.t.ocr).width(Some(110.0)).on_edit(|s: &mut S| &mut s.t.ocr).on_change(|s: &mut S, _| {
                s.t.s.ocr_lang = s.t.ocr.text.trim().to_string();
                changed(s);
            }).build(&s.kit),
        )
        .child(dim(have, th));
        let themes: Vec<&'static str> = shared::theme::all().iter().map(|t| t.name).collect();
        let cur = themes.iter().position(|n| *n == st.theme).unwrap_or(0);
        let theme = picker("theme", &themes, cur, 220.0, th, move |s: &mut S, i| {
            s.t.s.theme = shared::theme::all()[i].name.to_string();
            changed(s);
        });
        let start = checkbox("start", s.t.start, "Start Utsuri with Windows", th).on_press(|s: &mut S| {
            let on = !s.t.start;
            match crate::shell::set_start_at_login(on) {
                Ok(()) => s.t.start = on,
                Err(e) => s.error("Couldn't change start at login", e.to_string()),
            }
        });
        let shell = checkbox("shell", s.t.shell, "\"Utsuri…\" in right-click menus and Send To", th).on_press(|s: &mut S| {
            let on = !s.t.shell;
            let r = if on { crate::shell::install_shell_menu("") } else { crate::shell::uninstall_shell_menu("") };
            match r {
                Ok(()) => s.t.shell = on,
                Err(e) => s.error("Couldn't change the right-click menu", e.to_string()),
            }
        });
        let footer = row()
            .w_full()
            .items_center()
            .child(dim(concat!("Utsuri ", env!("CARGO_PKG_VERSION"), "  ·  originals are never modified"), th))
            .child(rune::spacer())
            .child(primary("done", "Done", true, th).on_press_tx(|_s: &mut S, tx: &mut Tx| tx.close_window()));
        col()
            .w_full()
            .grow(1.0)
            .gap(16.0)
            .pt(6.0)
            .child(line("Open the menu with", trigger))
            .child(line("Save results", output))
            .child(line("JPG / WebP quality", quality))
            .child(line("PDF → image resolution", dpi))
            .child(line("Text recognition", ocr))
            .child(line("Theme", theme))
            .child(row().w_full().gap(14.0).child(el().w(180.0)).child(col().gap(10.0).child(start).child(shell)))
            .child(rune::spacer())
            .child(footer)
    }
}

pub(crate) fn open(theme: &Theme, app: Option<AppHandle<crate::Hub>>, area: (i32, i32, i32, i32)) -> anyhow::Result<crate::ToolEntry> {
    super::launch(SettingsWindow::new(), String::new(), theme, app, area)
}

// ------------------------------------------------------------------ welcome

pub struct Welcome {
    pub key: String,
}

impl Tool for Welcome {
    fn title(&self) -> String {
        "Welcome to Utsuri".into()
    }
    fn size(&self) -> (f32, f32) {
        (600.0, 470.0)
    }
    fn enter(s: &mut ToolS<Welcome>, tx: &mut Tx) {
        got_it(s, tx);
    }
    fn body(s: &ToolS<Welcome>, _cx: &mut Cx<ToolS<Welcome>>) -> El {
        let th = &s.theme;
        let key = &s.t.key;
        let steps = [
            ("1", "Start dragging a file in File Explorer or on the desktop.".to_string()),
            ("2", format!("Press {key}. The menu opens under your cursor.")),
            ("3", "Drop the file on a slice. The result is saved next to the original; the original is never changed.".to_string()),
            ("4", format!("Tap {key} again to switch rings: Convert, Tools, More, Pack.")),
        ];
        let mut list = col().w_full().gap(14.0).pt(4.0);
        for (n, t) in steps {
            list = list.child(
                row()
                    .w_full()
                    .gap(14.0)
                    .items_start()
                    .child(el().size(28.0, 28.0).radius(14.0).center().linear_gradient(2.0, [(0.0, col_of(th.accent_a)), (1.0, col_of(th.accent_b))]).border(1.0, white(0.5)).child(text(n).font(STRONG).color(Color::WHITE)))
                    .child(text(t).wrap().font(font(15.0, 450.0)).color(col_of(th.label())).grow(1.0).min_w(0.0).pt(4.0)),
            );
        }
        col()
            .w_full()
            .grow(1.0)
            .gap(14.0)
            .child(dim("Convert and edit any file from a radial menu", th))
            .child(list)
            .child(card(col().w_full().child(text("Every result shows a notification with Open, Show in folder and Undo. You can also click the tray icon to pick a file, or turn on the right-click entry in Settings.").wrap().w_full().font(SMALL).color(col_of(th.text_dim)))))
            .child(rune::spacer())
            .child(row().w_full().child(rune::spacer()).child(primary("got-it", "Got it", true, th).on_press_tx(|s: &mut ToolS<Welcome>, tx: &mut Tx| got_it(s, tx))))
    }
}

fn got_it(s: &mut ToolS<Welcome>, tx: &mut Tx) {
    let mut st = Settings::load();
    if !st.welcomed {
        st.welcomed = true;
        st.save();
        s.settings_changed();
    }
    tx.close_window();
}

pub(crate) fn open_welcome(key: &str, theme: &Theme, app: Option<AppHandle<crate::Hub>>, area: (i32, i32, i32, i32)) -> anyhow::Result<crate::ToolEntry> {
    super::launch(Welcome { key: key.into() }, String::new(), theme, app, area)
}
