//! Notifications: a stack of solid cards at the bottom-right. Ported from toasts.py.
//! A job shows a spinner card only if it takes longer than 150 ms; the card then turns
//! into the result: title, detail, a highlighted figure, a thumbnail, and Open /
//! Show in folder / Undo. Cards sit at fixed places (no flow layout), so the app knows
//! where every button is (the E2E run reads them from the log).

use super::*;
use rune::{AppHandle, Cx};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use utsuri_engine::report::Report;

pub const WIDTH: f32 = 380.0;
pub const GAP: f32 = 10.0;
/// Margin around the cards inside the window (room around the cards).
pub const PAD: f32 = 12.0;
/// The notification window's fixed height: five of the tallest cards.
pub const HEIGHT: f32 = 5.0 * 129.0 + 4.0 * GAP + 2.0 * PAD;
const DONE_TTL: Duration = Duration::from_secs(7);
const ERROR_TTL: Duration = Duration::from_secs(10);
/// An update card waits longer: it asks something.
const UPDATE_TTL: Duration = Duration::from_secs(60);

pub enum Body {
    Working { title: String, detail: String },
    Done(Box<Report>, Option<rune::Pixels>),
    Error { title: String, msg: String },
    Info { title: String, msg: String },
    /// A newer version: Update / Later.
    Update { version: String },
}

pub struct Toast {
    pub id: u64,
    pub body: Body,
    born: Instant,
    hover_until: Option<Instant>,
    pub undone: bool,
}

#[derive(Default)]
pub struct Stack {
    pub toasts: Vec<Toast>,
    next: u64,
}

impl Stack {
    pub fn push(&mut self, body: Body) -> u64 {
        self.next += 1;
        self.toasts.push(Toast { id: self.next, body, born: Instant::now(), hover_until: None, undone: false });
        // at most five on screen
        while self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
        self.next
    }

    /// Add a card under an id the app chose.
    pub fn push_id(&mut self, id: u64, body: Body) {
        self.next = self.next.max(id);
        self.toasts.push(Toast { id, body, born: Instant::now(), hover_until: None, undone: false });
        while self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    /// Replace a card's content (a spinner turning into its result).
    pub fn set(&mut self, id: u64, body: Body) {
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == id) {
            t.body = body;
            t.born = Instant::now();
        } else {
            self.push(body);
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.toasts.retain(|t| t.id != id);
    }

    /// Drop expired cards. Returns true if any went.
    pub fn expire(&mut self) -> bool {
        let now = Instant::now();
        let before = self.toasts.len();
        self.toasts.retain(|t| {
            let ttl = match t.body {
                Body::Working { .. } => return true,
                Body::Error { .. } => ERROR_TTL,
                Body::Update { .. } => UPDATE_TTL,
                _ => DONE_TTL,
            };
            t.hover_until.is_some_and(|h| h > now) || now.duration_since(t.born) < ttl
        });
        self.toasts.len() != before
    }

    pub fn height_of(t: &Toast) -> f32 {
        match &t.body {
            Body::Working { .. } => 66.0,
            Body::Update { .. } => 110.0,
            Body::Done(r, _) => if r.outs.is_empty() || t.undone { 82.0 } else { 118.0 },
            Body::Error { msg, .. } | Body::Info { msg, .. } => 78.0 + (msg.chars().count() as f32 / 48.0).floor().min(3.0) * 17.0,
        }
    }

    pub fn total_height(&self) -> f32 {
        self.toasts.iter().map(Self::height_of).sum::<f32>() + GAP * self.toasts.len().saturating_sub(1) as f32
    }

    /// Each card's rect in the window (logical px), stacked from the bottom.
    pub fn rects(&self) -> Vec<[f32; 4]> {
        let mut y = HEIGHT - PAD - self.total_height();
        self.toasts
            .iter()
            .map(|t| {
                let h = Self::height_of(t);
                let r = [PAD, y, PAD + WIDTH, y + h];
                y += h + GAP;
                r
            })
            .collect()
    }

    /// The window: always room for five of the tallest cards. It never changes size (resizing an
    /// overlay made Rune rebuild its caches, which stalled the app longer each time); the space
    /// around the cards lets clicks through.
    pub fn window_size(&self) -> (f32, f32) {
        (WIDTH + PAD * 2.0, HEIGHT)
    }
}

/// The buttons of a result card: label, width, offset from the card's left.
pub const BUTTONS: [(&str, f32); 3] = [("Open", 58.0), ("Show in folder", 116.0), ("Undo", 58.0)];
pub const BUTTONS_X: f32 = 78.0;
/// The buttons of an update card.
pub const UPDATE_BUTTONS: [(&str, f32); 2] = [("Update", 76.0), ("Later", 64.0)];
pub const UPDATE_X: f32 = 54.0;

/// Where each card's buttons are (window logical px): (toast id, label, x, y, w, h).
pub fn button_rects(stack: &Stack) -> Vec<(u64, &'static str, [f32; 4])> {
    let mut out = vec![];
    for (t, r) in stack.toasts.iter().zip(stack.rects()) {
        if let Body::Update { .. } = &t.body {
            let (mut x, y) = (r[0] + UPDATE_X, r[3] - 38.0);
            for (l, w) in UPDATE_BUTTONS {
                out.push((t.id, l, [x, y, w, 26.0]));
                x += w + 8.0;
            }
        }
        if let Body::Done(rep, _) = &t.body {
            if rep.outs.is_empty() || t.undone {
                continue;
            }
            let (mut x, y) = (r[0] + BUTTONS_X, r[3] - 36.0);
            for (l, w) in BUTTONS {
                out.push((t.id, l, [x, y, w, 26.0]));
                x += w + 8.0;
            }
        }
    }
    out
}

pub enum Action {
    Open(PathBuf),
    Reveal(PathBuf),
    Undo(u64, Vec<PathBuf>),
    Close(u64),
    /// Install the update the card offers (and close it).
    Update(u64),
}

pub struct ToastS {
    pub stack: Stack,
    pub theme: Theme,
    pub kit: rune_kit::Kit,
    pub app: Option<AppHandle<crate::Hub>>,
    /// where the stack was last put (x, y, w, h in points): nothing to redo while it holds
    pub placed: Option<(f32, f32, f32, f32)>,
}

impl ToastS {
    pub fn new(theme: Theme) -> ToastS {
        ToastS { stack: Stack::default(), kit: kit(&theme), theme, app: None, placed: None }
    }
}

fn elide(s: &str, n: usize) -> String {
    if s.chars().count() > n { format!("{}…", s.chars().take(n - 1).collect::<String>()) } else { s.to_string() }
}

fn act(s: &mut ToastS, a: Action) {
    if let Some(app) = s.app.clone() {
        app.update(move |h, tx| h.toast_action(a, tx));
    }
}

pub fn view(s: &ToastS, _cx: &mut Cx<ToastS>) -> El {
    let th = &s.theme;
    let (lab, dimc, accent) = (col_of(th.label()), col_of(th.text_dim), col_of(th.accent_a));
    let (w, h) = s.stack.window_size();
    let mut root = el().key("toasts").size(w, h);
    for (t, r) in s.stack.toasts.iter().zip(s.stack.rects()) {
        let id = t.id;
        let (cw, ch) = (r[2] - r[0], r[3] - r[1]);
        let mut card = el()
            .key(format!("toast/{id}"))
            .absolute(r[0], r[1])
            .size(cw, ch)
            .radius(16.0)
            .fill(col_of(th.tint(Rgba(22, 18, 34, 255))))
            .border(1.0, white(0.16))
            .hit()
            .on_hover_change(move |s: &mut ToastS, inside, _| {
                if inside {
                    if let Some(t) = s.stack.toasts.iter_mut().find(|t| t.id == id) {
                        t.hover_until = Some(Instant::now() + Duration::from_secs(3));
                    }
                }
            });
        // close
        card = card.child(
            el().key(format!("toast/{id}/close"))
                .absolute(cw - 30.0, 6.0)
                .size(24.0, 24.0)
                .radius(12.0)
                .center()
                .hover(white(0.12))
                .cursor(rune::Cursor::Pointer)
                .child(text("×").font(font(17.0, 400.0)).color(dimc))
                .on_press(move |s: &mut ToastS| act(s, Action::Close(id))),
        );
        let x = 16.0;
        match &t.body {
            Body::Working { title, detail } => {
                card = card
                    .child(el().absolute(x, ch / 2.0 - 12.0).size(24.0, 24.0).center().child(rune_kit::Spinner::new().size(20.0).color(accent).build(&s.kit)))
                    .child(text(title).font(font(14.5, 600.0)).color(lab).absolute(x + 38.0, 14.0))
                    .child(text(elide(detail, 44)).font(SMALL).color(dimc).absolute(x + 38.0, 36.0));
            }
            Body::Error { title, msg } | Body::Info { title, msg } => {
                let is_err = matches!(t.body, Body::Error { .. });
                let badge = if is_err { col_of(Rgba(255, 94, 94, 220)) } else { accent.alpha(0.85) };
                card = card
                    .child(el().absolute(x, 14.0).size(26.0, 26.0).radius(13.0).fill(badge).center().child(text(if is_err { "!" } else { "i" }).font(font(15.0, 700.0)).color(Color::WHITE)))
                    .child(text(title).font(font(14.5, 600.0)).color(lab).absolute(x + 38.0, 15.0))
                    .child(text(msg.clone()).wrap().font(SMALL).color(dimc).absolute(x + 38.0, 37.0).w(cw - x - 38.0 - 34.0));
            }
            Body::Update { version } => {
                card = card
                    .child(el().absolute(x, 14.0).size(26.0, 26.0).radius(13.0).fill(accent.alpha(0.85)).center().child(text("↑").font(font(15.0, 700.0)).color(Color::WHITE)))
                    .child(text(format!("Utsuri {version} is available")).font(font(14.5, 600.0)).color(lab).absolute(UPDATE_X - 16.0 + x, 15.0))
                    .child(text("Update now? Utsuri closes and opens again by itself.").wrap().font(SMALL).color(dimc).absolute(UPDATE_X - 16.0 + x, 37.0).w(cw - UPDATE_X - 34.0));
                let mut bx = UPDATE_X;
                for (i, (l, bw)) in UPDATE_BUTTONS.iter().enumerate() {
                    let b = row()
                        .key(format!("toast/{id}/{l}"))
                        .absolute(bx, ch - 38.0)
                        .size(*bw, 26.0)
                        .radius(13.0)
                        .center()
                        .cursor(rune::Cursor::Pointer)
                        .role("button")
                        .label(*l);
                    let b = if i == 0 {
                        b.linear_gradient(0.0, [(0.0, col_of(th.accent_a)), (1.0, col_of(th.accent_b))]).border(1.0, white(0.35)).child(text(*l).font(font(12.5, 600.0)).color(Color::WHITE))
                    } else {
                        b.fill(white(0.10)).border(1.0, white(0.18)).hover(white(0.12)).child(text(*l).font(font(12.5, 500.0)).color(lab))
                    };
                    card = card.child(b.on_press(move |s: &mut ToastS| act(s, if i == 0 { Action::Update(id) } else { Action::Close(id) })));
                    bx += bw + 8.0;
                }
            }
            Body::Done(rep, thumb) => {
                let icon = match (thumb, t.undone) {
                    (Some(px), false) => el().size(50.0, 50.0).radius(8.0).fill(black(0.25)).child(el().w_full().h_full().radius(8.0).pixels_fit(px, rune::SurfaceFit::Fit)),
                    _ => el().size(40.0, 40.0).radius(20.0).fill(accent.alpha(0.85)).center().child(text(if t.undone { "↶" } else { "✓" }).font(font(19.0, 600.0)).color(Color::WHITE)),
                };
                card = card.child(el().absolute(x, 14.0).size(50.0, 50.0).center().child(icon));
                let title = if t.undone { "Undone: moved to the Recycle Bin".to_string() } else { rep.title.clone() };
                let tx = BUTTONS_X - 2.0;
                card = card.child(text(elide(&title, 34)).font(font(14.5, 600.0)).color(lab).absolute(tx, 14.0)).child(text(elide(&rep.detail, 44)).font(SMALL).color(dimc).absolute(tx, 36.0));
                if !rep.good.is_empty() && !t.undone {
                    let good = col_of(GOOD);
                    card = card.child(row().absolute(tx, 57.0).h(20.0).px(8.0).radius(10.0).fill(good.alpha(0.16)).items_center().child(text(&rep.good).font(font(11.5, 600.0)).color(good)));
                }
                if !rep.outs.is_empty() && !t.undone {
                    let first = rep.outs[0].clone();
                    let outs = rep.outs.clone();
                    let mut bx = BUTTONS_X;
                    for (i, (l, bw)) in BUTTONS.iter().enumerate() {
                        let (first, outs) = (first.clone(), outs.clone());
                        card = card.child(
                            row().key(format!("toast/{id}/{l}"))
                                .absolute(bx, ch - 36.0)
                                .size(*bw, 26.0)
                                .radius(13.0)
                                .center()
                                .fill(white(0.10))
                                .border(1.0, white(0.18))
                                .hover(white(0.12))
                                .cursor(rune::Cursor::Pointer)
                                .role("button")
                                .label(*l)
                                .child(text(*l).font(font(12.5, 500.0)).color(lab))
                                .on_press(move |s: &mut ToastS| {
                                    act(s, match i {
                                        0 => Action::Open(first.clone()),
                                        1 => Action::Reveal(first.clone()),
                                        _ => Action::Undo(id, outs.clone()),
                                    })
                                }),
                        );
                        bx += bw + 8.0;
                    }
                }
            }
        }
        root = root.child(card);
    }
    root
}
