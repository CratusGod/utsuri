//! Protect (add an AES-256 password) and Unlock (remove it).

use super::subtitle;
use crate::ui::frame::{Tool, ToolS, footer};
use crate::ui::*;
use rune::{Cx, Tx};
use std::path::PathBuf;

pub struct Protect {
    paths: Vec<PathBuf>,
    pw: rune::TextEdit,
    pw2: rune::TextEdit,
    show: bool,
}

impl Protect {
    pub fn new(paths: Vec<PathBuf>) -> (Self, String) {
        let sub = subtitle(&paths);
        let mut pw = rune::TextEdit::new("");
        pw.focused = true; // the caret starts in Password (autofocus would take it back from Confirm)
        (Protect { paths, pw, pw2: rune::TextEdit::new(""), show: false }, sub)
    }
    fn ok(&self) -> bool {
        self.pw.text.chars().count() >= 4 && self.pw.text == self.pw2.text
    }
}

fn strength(pw: &str) -> &'static str {
    let kinds = [pw.chars().any(|c| c.is_lowercase()), pw.chars().any(|c| c.is_uppercase()), pw.chars().any(|c| c.is_ascii_digit()), pw.chars().any(|c| !c.is_alphanumeric())].iter().filter(|b| **b).count();
    match (pw.chars().count(), kinds) {
        (n, k) if n >= 12 && k >= 3 => "strong",
        (n, k) if n >= 8 && k >= 2 => "good",
        _ => "weak",
    }
}

type P = ToolS<Protect>;

impl Tool for Protect {
    fn title(&self) -> String {
        "Protect PDF".into()
    }
    fn size(&self) -> (f32, f32) {
        (560.0, 360.0)
    }
    fn body(s: &P, _cx: &mut Cx<P>) -> El {
        let th = &s.theme;
        let p = &s.t;
        let field = |key: &str, e: &rune::TextEdit, first: bool| {
            let f = rune_kit::TextInput::new(e).secure().revealed(p.show).width(Some(280.0)).on_edit(move |s: &mut P| if first { &mut s.t.pw } else { &mut s.t.pw2 }).on_key(move |s: &mut P, k: &rune::KeyEvent, tx: &mut Tx| {
                // Tab moves between the two fields
                if !matches!(k.key, rune::Key::Tab) {
                    return false;
                }
                s.t.pw.focused = !first;
                s.t.pw2.focused = first;
                tx.focus(if first { "pw2" } else { "pw" }, true);
                true
            }).on_submit(|s: &mut P, tx| Protect::enter(s, tx)).build(&s.kit).key(key);
            // the caret starts in Password; once something is typed autofocus lets go, or it
            // would pull the caret back from Confirm on every redraw
            if first && p.pw.text.is_empty() && p.pw2.text.is_empty() { f.autofocus() } else { f }
        };
        let line = |l: &str, e: El| row().items_center().gap(14.0).child(el().w(90.0).child(label(l, th))).child(e);
        let (msg, c) = if p.pw.text.is_empty() {
            ("AES-256 encryption. Anyone opening the PDF will need this password.".to_string(), th.text_dim)
        } else if p.pw.text.chars().count() < 4 {
            ("Use at least 4 characters.".to_string(), WARN)
        } else if p.pw.text != p.pw2.text {
            ("Passwords don't match yet.".to_string(), WARN)
        } else {
            (format!("Looks good · {} password", strength(&p.pw.text)), GOOD)
        };
        col()
            .w_full()
            .grow(1.0)
            .gap(12.0)
            .pt(4.0)
            .child(line("Password", field("pw", &p.pw, true)))
            .child(line("Confirm", field("pw2", &p.pw2, false)))
            .child(row().child(el().w(104.0)).child(checkbox("show", p.show, "Show", th).on_press(|s: &mut P| s.t.show = !s.t.show)))
            .child(text(msg).key("status").wrap().w_full().font(SMALL).color(col_of(c)))
            .child(rune::spacer())
            .child(footer(s, "Protect", p.ok(), None))
    }
    fn enter(s: &mut P, tx: &mut Tx) {
        log::debug!("protect: Enter with {} and {} characters", s.t.pw.text.chars().count(), s.t.pw2.text.chars().count());
        if !s.t.ok() {
            return;
        }
        let src = s.t.paths[0].clone();
        let pw = std::mem::take(&mut s.t.pw.text);
        s.t.pw2 = rune::TextEdit::new("");
        s.submit(tx, "tool:Protect", &[src.clone()], "Encrypting…", String::new(), move || Ok(vec![utsuri_engine::pdf::protect_pdf(&src, &pw)?]));
    }
}

pub struct Unlock {
    paths: Vec<PathBuf>,
    needs: bool,
    pw: rune::TextEdit,
    wrong: bool,
}

impl Unlock {
    pub fn new(paths: Vec<PathBuf>) -> anyhow::Result<(Self, String)> {
        if !utsuri_engine::pdf::is_encrypted(&paths[0])? {
            utsuri_engine::refuse!("This PDF has no password");
        }
        let needs = utsuri_engine::pdf::needs_password(&paths[0]);
        let sub = subtitle(&paths);
        let mut pw = rune::TextEdit::new("");
        pw.focused = true;
        Ok((Unlock { paths, needs, pw, wrong: false }, sub))
    }
}

type U = ToolS<Unlock>;

impl Tool for Unlock {
    fn title(&self) -> String {
        "Unlock PDF".into()
    }
    fn size(&self) -> (f32, f32) {
        (560.0, 320.0)
    }
    fn body(s: &U, _cx: &mut Cx<U>) -> El {
        let th = &s.theme;
        let u = &s.t;
        let mut c = col().w_full().grow(1.0).gap(12.0).pt(4.0);
        if u.needs {
            c = c.child(
                row().items_center().gap(14.0).child(label("Password", th)).child(
                    rune_kit::TextInput::new(&u.pw)
                        .secure()
                        .width(Some(280.0))
                        .invalid(u.wrong)
                        .on_edit(|s: &mut U| &mut s.t.pw)
                        .on_change(|s: &mut U, _| s.t.wrong = false)
                        .on_submit(|s: &mut U, tx| Unlock::enter(s, tx))
                        .build(&s.kit)
                        .key("pw")
                        .autofocus(),
                ),
            );
            c = c.child(if u.wrong {
                text("Wrong password").key("status").font(STRONG).color(col_of(BAD))
            } else {
                text("Type the password used to open this PDF. The copy you get won't ask for it.").key("status").wrap().w_full().font(SMALL).color(col_of(th.text_dim))
            });
        } else {
            c = c.child(text("This PDF only has an owner password (it opens without one, but limits printing or copying). Unlocking removes those limits.").wrap().w_full().font(BODY).color(col_of(th.label())));
        }
        c.child(rune::spacer()).child(footer(s, "Unlock", !u.needs || !u.pw.text.is_empty(), None))
    }
    fn enter(s: &mut U, tx: &mut Tx) {
        let src = s.t.paths[0].clone();
        log::debug!("unlock: Enter with {} characters", s.t.pw.text.chars().count());
        if s.t.needs && !utsuri_engine::pdf::password_ok(&src, &s.t.pw.text) {
            s.t.wrong = true;
            return;
        }
        let pw = std::mem::take(&mut s.t.pw.text);
        s.submit(tx, "tool:Unlocked", &[src.clone()], "Unlocking…", String::new(), move || Ok(vec![utsuri_engine::pdf::unlock(&src, &pw)?]));
    }
}
