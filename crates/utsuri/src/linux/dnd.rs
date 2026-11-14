//! File drops on the radial menu under Wayland. Rune's layer-shell overlays don't take part
//! in drag and drop, so while the menu shows in drag mode an invisible layer surface of the
//! same size sits on top of it and receives the drag (`wl_data_device`): its files on enter
//! (the rings follow them) and on drop (the slice under the pointer runs). It never draws
//! anything visible and is unmapped whenever the menu hides, so it can't eat clicks.

use std::io::Read as _;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm::{Format, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1};

const URI_LIST: &str = "text/uri-list";
/// Longest wait for the source app to hand over its file list.
const RECEIVE_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_MS: i32 = 10;

/// What the menu learns from a drag.
pub enum Drag {
    Entered(Vec<PathBuf>),
    Dropped(Vec<PathBuf>),
}

enum Cmd {
    /// Cover this rect (layout px, from the output's top-left).
    Show { x: i32, y: i32, w: i32, h: i32 },
    Hide,
}

static TX: Mutex<Option<Sender<Cmd>>> = Mutex::new(None);

/// Start the catcher thread; `deliver` gets every drag event (on that thread).
pub fn start(deliver: impl Fn(Drag) + Send + 'static) {
    let (tx, rx) = channel();
    *TX.lock().unwrap() = Some(tx);
    let spawned = std::thread::Builder::new().name("utsuri-dnd".into()).spawn(move || {
        if let Err(e) = run(rx, Box::new(deliver)) {
            log::warn!("file drops on the menu won't work: {e:#}");
        }
    });
    if let Err(e) = spawned {
        log::warn!("no drop catcher thread: {e}");
    }
}

pub fn show(x: i32, y: i32, w: i32, h: i32) {
    send(Cmd::Show { x, y, w, h });
}

pub fn hide() {
    send(Cmd::Hide);
}

fn send(c: Cmd) {
    if let Some(tx) = TX.lock().unwrap().as_ref() {
        let _ = tx.send(c);
    }
}

#[derive(Default)]
struct Globals {
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    seat: Option<WlSeat>,
    ddm: Option<WlDataDeviceManager>,
    layer_shell: Option<ZwlrLayerShellV1>,
}

struct Shown {
    surface: WlSurface,
    layer: ZwlrLayerSurfaceV1,
    buffer: Option<WlBuffer>,
    size: (i32, i32),
}

struct State {
    g: Globals,
    shown: Option<Shown>,
    /// The offer of the drag over us, and whether it carries files.
    offer: Option<(WlDataOffer, bool)>,
    /// Mime types of offers announced but not entered yet.
    pending_mimes: Vec<(WlDataOffer, Vec<String>)>,
    /// Work for the loop after a dispatch: (offer, dropped?).
    want_files: Option<(WlDataOffer, bool)>,
    configured: Option<u32>,
}

type Deliver = Box<dyn Fn(Drag) + Send>;

fn run(rx: Receiver<Cmd>, deliver: Deliver) -> anyhow::Result<()> {
    let conn = Connection::connect_to_env()?;
    let mut queue: EventQueue<State> = conn.new_event_queue();
    let qh = queue.handle();
    let display = conn.display();
    display.get_registry(&qh, ());
    let mut st = State { g: Globals::default(), shown: None, offer: None, pending_mimes: vec![], want_files: None, configured: None };
    queue.roundtrip(&mut st)?;
    let (Some(seat), Some(ddm)) = (st.g.seat.clone(), st.g.ddm.clone()) else { anyhow::bail!("the compositor offers no seat or data device manager") };
    anyhow::ensure!(st.g.layer_shell.is_some() && st.g.compositor.is_some() && st.g.shm.is_some(), "the compositor offers no layer shell");
    let _device = ddm.get_data_device(&seat, &qh, ());
    queue.roundtrip(&mut st)?;
    log::debug!("drop catcher ready");

    loop {
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                Cmd::Show { x, y, w, h } => show_surface(&mut st, &mut queue, &qh, x, y, w, h),
                Cmd::Hide => hide_surface(&mut st),
            }
            queue.flush()?;
        }
        if let Some((offer, dropped)) = st.want_files.take() {
            let files = receive(&conn, &offer);
            if dropped {
                offer.finish();
                offer.destroy();
                st.offer = None;
                queue.flush()?;
                if !files.is_empty() {
                    deliver(Drag::Dropped(files));
                }
            } else if !files.is_empty() {
                deliver(Drag::Entered(files));
            }
        }
        // wait for events (or the next command) without blocking forever
        queue.flush()?;
        if let Some(guard) = queue.prepare_read() {
            let fd = guard.connection_fd().as_raw_fd();
            let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            // SAFETY: one valid pollfd, read only by poll during the call.
            let n = unsafe { libc::poll(&mut p, 1, POLL_MS) };
            if n > 0 {
                let _ = guard.read();
            }
        }
        queue.dispatch_pending(&mut st)?;
    }
}

fn show_surface(st: &mut State, queue: &mut EventQueue<State>, qh: &QueueHandle<State>, x: i32, y: i32, w: i32, h: i32) {
    hide_surface(st);
    let (Some(comp), Some(ls)) = (st.g.compositor.clone(), st.g.layer_shell.clone()) else { return };
    let surface = comp.create_surface(qh, ());
    let layer = ls.get_layer_surface(&surface, None, Layer::Overlay, "utsuri-drop".into(), qh, ());
    layer.set_size(w.max(1) as u32, h.max(1) as u32);
    layer.set_anchor(Anchor::Top | Anchor::Left);
    layer.set_margin(y, 0, 0, x);
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    surface.commit();
    st.configured = None;
    st.shown = Some(Shown { surface, layer, buffer: None, size: (w, h) });
    let end = Instant::now() + Duration::from_millis(500);
    while st.configured.is_none() && Instant::now() < end {
        if queue.blocking_dispatch(st).is_err() {
            break;
        }
    }
    let Some(serial) = st.configured else {
        log::debug!("drop catcher: no configure");
        return;
    };
    let Some(shown) = st.shown.as_mut() else { return };
    shown.layer.ack_configure(serial);
    match transparent_buffer(&st.g.shm, qh, shown.size) {
        Ok(b) => {
            shown.surface.attach(Some(&b), 0, 0);
            shown.surface.damage_buffer(0, 0, shown.size.0, shown.size.1);
            shown.buffer = Some(b);
        }
        Err(e) => log::debug!("drop catcher buffer: {e:#}"),
    }
    shown.surface.commit();
}

fn hide_surface(st: &mut State) {
    if let Some(s) = st.shown.take() {
        s.layer.destroy();
        s.surface.destroy();
        if let Some(b) = s.buffer {
            b.destroy();
        }
    }
    st.offer = None;
}

/// A fully transparent ARGB buffer: the surface takes input but shows nothing.
fn transparent_buffer(shm: &Option<WlShm>, qh: &QueueHandle<State>, (w, h): (i32, i32)) -> anyhow::Result<WlBuffer> {
    let shm = shm.as_ref().ok_or_else(|| anyhow::anyhow!("no wl_shm"))?;
    let (w, h) = (w.max(1), h.max(1));
    let len = (w * h * 4) as usize;
    let name = std::ffi::CString::new("utsuri-drop")?;
    // SAFETY: a valid C string; memfd_create returns a new fd or -1.
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    anyhow::ensure!(fd >= 0, "memfd_create failed");
    // SAFETY: fd was just created and is owned only here.
    let file = unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(fd) };
    file.set_len(len as u64)?; // zero-filled: alpha 0 everywhere
    let pool: WlShmPool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buf = pool.create_buffer(0, w, h, w * 4, Format::Argb8888, qh, ());
    pool.destroy();
    Ok(buf)
}

/// The dragged files: ask the source for its uri-list and read it from a pipe.
fn receive(conn: &Connection, offer: &WlDataOffer) -> Vec<PathBuf> {
    let mut fds = [0i32; 2];
    // SAFETY: fds has room for the two ends.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return vec![];
    }
    // SAFETY: pipe2 just returned these two fds, owned only here.
    let (r, w): (OwnedFd, OwnedFd) = unsafe { (std::os::fd::FromRawFd::from_raw_fd(fds[0]), std::os::fd::FromRawFd::from_raw_fd(fds[1])) };
    offer.receive(URI_LIST.into(), w.as_fd());
    let _ = conn.flush();
    drop(w); // only the source keeps a write end: EOF once it is done
    let mut file = std::fs::File::from(r);
    let mut data = Vec::new();
    let end = Instant::now() + RECEIVE_TIMEOUT;
    let mut chunk = [0u8; 8192];
    loop {
        let left = end.saturating_duration_since(Instant::now()).as_millis() as i32;
        if left <= 0 {
            log::debug!("drop catcher: the source took too long");
            break;
        }
        let mut p = libc::pollfd { fd: file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd.
        if unsafe { libc::poll(&mut p, 1, left) } <= 0 {
            break;
        }
        match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => data.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
        if data.len() > 1 << 20 {
            break;
        }
    }
    parse_uri_list(&String::from_utf8_lossy(&data))
}

/// `file://` lines of a text/uri-list, percent-decoded; comments and other schemes skipped.
pub fn parse_uri_list(s: &str) -> Vec<PathBuf> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("file://"))
        // file://host/path: keep the path (host is empty or localhost on a desktop)
        .map(|rest| rest.find('/').map_or(rest, |i| &rest[i..]))
        .map(|p| PathBuf::from(<std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(percent_decode(p))))
        .collect()
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(st: &mut Self, reg: &WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            match interface.as_str() {
                "wl_compositor" => st.g.compositor = Some(reg.bind(name, version.min(4), qh, ())),
                "wl_shm" => st.g.shm = Some(reg.bind(name, 1, qh, ())),
                "wl_seat" if st.g.seat.is_none() => st.g.seat = Some(reg.bind(name, version.min(5), qh, ())),
                "wl_data_device_manager" => st.g.ddm = Some(reg.bind(name, version.min(3), qh, ())),
                "zwlr_layer_shell_v1" => st.g.layer_shell = Some(reg.bind(name, version.min(4), qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for State {
    fn event(st: &mut Self, _: &ZwlrLayerSurfaceV1, ev: zwlr_layer_surface_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            zwlr_layer_surface_v1::Event::Configure { serial, .. } => st.configured = Some(serial),
            zwlr_layer_surface_v1::Event::Closed => st.shown = None,
            _ => {}
        }
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(st: &mut Self, _: &WlDataDevice, ev: wl_data_device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            wl_data_device::Event::DataOffer { id } => st.pending_mimes.push((id, vec![])),
            wl_data_device::Event::Enter { serial, surface, id, .. } => {
                let ours = st.shown.as_ref().is_some_and(|s| s.surface == surface);
                let Some(offer) = id else { return };
                let mimes = st.pending_mimes.iter().find(|(o, _)| *o == offer).map(|(_, m)| m.clone()).unwrap_or_default();
                st.pending_mimes.retain(|(o, _)| *o != offer);
                let files = ours && mimes.iter().any(|m| m == URI_LIST);
                if files {
                    offer.accept(serial, Some(URI_LIST.into()));
                    offer.set_actions(DndAction::Copy, DndAction::Copy);
                    st.want_files = Some((offer.clone(), false));
                } else {
                    offer.accept(serial, None);
                }
                st.offer = Some((offer, files));
            }
            wl_data_device::Event::Leave => {
                st.offer = None;
            }
            wl_data_device::Event::Drop => {
                if let Some((offer, true)) = st.offer.clone() {
                    st.want_files = Some((offer, true));
                }
            }
            wl_data_device::Event::Selection { id } => {
                // the clipboard's offer: not ours to keep
                if let Some(o) = id {
                    st.pending_mimes.retain(|(p, _)| *p != o);
                    o.destroy();
                }
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

impl Dispatch<WlDataOffer, ()> for State {
    fn event(st: &mut Self, offer: &WlDataOffer, ev: wl_data_offer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_data_offer::Event::Offer { mime_type } = ev {
            if let Some((_, m)) = st.pending_mimes.iter_mut().find(|(o, _)| o == offer) {
                m.push(mime_type);
            }
        }
    }
}

delegate_noop!(State: ignore WlCompositor);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: ignore WlShmPool);
delegate_noop!(State: ignore WlBuffer);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: ignore WlSeat);
delegate_noop!(State: ignore WlDataDeviceManager);
delegate_noop!(State: ignore ZwlrLayerShellV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_list_to_paths() {
        let l = "# comment\r\nfile:///home/u/My%20File.png\r\nfile://localhost/tmp/a%C3%A9.pdf\r\nhttps://x.y/z\r\n";
        assert_eq!(parse_uri_list(l), vec![PathBuf::from("/home/u/My File.png"), PathBuf::from("/tmp/aé.pdf")]);
        assert_eq!(parse_uri_list("file:///a%2"), vec![PathBuf::from("/a%2")]);
    }
}
