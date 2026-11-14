//! One Utsuri per user. A second launch (the file manager's "Utsuri…" action, the launcher)
//! hands its files to the running copy over a Unix socket in the runtime dir and exits.
//! The message is JSON: {"files": [...]} or {"show": true}.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

/// A hand-off is a list of paths: 1 MB is plenty, and a stray writer can't fill memory.
const MAX_MESSAGE: u64 = 1 << 20;
const IO_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Message {
    #[serde(default)]
    pub files: Vec<PathBuf>,
    #[serde(default)]
    pub show: bool,
}

/// `$XDG_RUNTIME_DIR/utsuri.sock`; `UTSURI_PIPE` (a path or a bare name) for tests.
pub fn pipe_name() -> String {
    let run = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    match std::env::var("UTSURI_PIPE") {
        Ok(p) if p.contains('/') => p,
        Ok(p) => run.join(format!("{p}.sock")).display().to_string(),
        Err(_) => run.join("utsuri.sock").display().to_string(),
    }
}

/// If Utsuri already runs, give it these args' files and return true.
pub fn try_handoff(args: &[String]) -> bool {
    if args.iter().any(|a| a == "--selftest" || a.starts_with("--cli")) {
        return false;
    }
    let files: Vec<PathBuf> = args.iter().skip(1).filter(|a| !a.starts_with("--")).map(PathBuf::from).filter(|p| p.exists()).map(|p| std::path::absolute(&p).unwrap_or(p)).collect();
    let msg = Message { show: files.is_empty() && !args.iter().any(|a| a == "--quiet"), files };
    let Ok(data) = serde_json::to_vec(&msg) else { return false };
    let Ok(mut s) = UnixStream::connect(pipe_name()) else { return false };
    let _ = s.set_write_timeout(Some(IO_TIMEOUT));
    s.write_all(&data).is_ok()
}

/// Listen forever on a background thread; every message goes to `deliver`.
pub fn serve(deliver: impl Fn(Message) + Send + 'static) {
    let path = PathBuf::from(pipe_name());
    // a socket left by a crashed Utsuri: nothing answers on it, so replace it
    if path.exists() && UnixStream::connect(&path).is_err() {
        let _ = std::fs::remove_file(&path);
    }
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            log::warn!("second launches won't reach this Utsuri ({}): {e}", path.display());
            return;
        }
    };
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    let spawned = std::thread::Builder::new().name("utsuri-pipe".into()).spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { continue };
            let _ = conn.set_read_timeout(Some(IO_TIMEOUT));
            let mut buf = Vec::new();
            let _ = conn.take(MAX_MESSAGE).read_to_end(&mut buf);
            match serde_json::from_slice::<Message>(&buf) {
                Ok(m) => deliver(m),
                Err(e) => log::warn!("bad hand-off message ({} bytes): {e}", buf.len()),
            }
        }
    });
    if let Err(e) = spawned {
        log::warn!("no hand-off thread: {e}");
    }
}
