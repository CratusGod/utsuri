//! One Utsuri per user. A second launch (right-click → Utsuri…, Send To, the Start menu)
//! hands its files to the running copy over a named pipe and exits at once.
//! Ported from handoff.py and app.py (_ipc). The message is JSON: {"files": [...]} or
//! {"show": true}.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::PathBuf;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_INBOUND, ReadFile};
use windows::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT};
use windows::core::HSTRING;

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Message {
    #[serde(default)]
    pub files: Vec<PathBuf>,
    #[serde(default)]
    pub show: bool,
}

pub fn pipe_name() -> String {
    if let Ok(p) = std::env::var("UTSURI_PIPE") {
        // a full pipe path or just the name
        let name = p.rsplit(['\\', '/']).next().unwrap_or(&p).to_string();
        return format!(r"\\.\pipe\{name}");
    }
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
    // the name the Python Utsuri used: one Utsuri at a time
    format!(r"\\.\pipe\Utsuri-{user}")
}

/// If Utsuri already runs, give it these args' files and return true.
pub fn try_handoff(args: &[String]) -> bool {
    if args.iter().any(|a| a == "--selftest" || a.starts_with("--cli")) {
        return false;
    }
    let files: Vec<PathBuf> = args.iter().skip(1).filter(|a| !a.starts_with("--")).map(PathBuf::from).filter(|p| p.exists()).map(|p| std::path::absolute(&p).unwrap_or(p)).collect();
    let msg = Message { show: files.is_empty() && !args.iter().any(|a| a == "--quiet"), files };
    let Ok(data) = serde_json::to_vec(&msg) else { return false };
    match std::fs::OpenOptions::new().write(true).open(pipe_name()) {
        Ok(mut f) => f.write_all(&data).is_ok(),
        Err(_) => false,
    }
}

/// Listen forever on a background thread; every message goes to `deliver`.
pub fn serve(deliver: impl Fn(Message) + Send + 'static) {
    std::thread::Builder::new()
        .name("utsuri-pipe".into())
        .spawn(move || {
            let name = HSTRING::from(pipe_name());
            loop {
                let h = unsafe { CreateNamedPipeW(&name, PIPE_ACCESS_INBOUND | FILE_FLAGS_AND_ATTRIBUTES(0), PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, PIPE_UNLIMITED_INSTANCES, 0, 64 * 1024, 0, None) };
                if h == INVALID_HANDLE_VALUE {
                    log::warn!("couldn't create the pipe: {:?}", unsafe { GetLastError() });
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    continue;
                }
                let connected = unsafe { ConnectNamedPipe(h, None) }.is_ok() || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
                if connected {
                    let mut buf = Vec::new();
                    // a hand-off is a list of paths: 1 MB is plenty, and a stray writer can't fill memory
                    let _ = PipeReader(h).take(1 << 20).read_to_end(&mut buf);
                    match serde_json::from_slice::<Message>(&buf) {
                        Ok(m) => deliver(m),
                        Err(e) => log::warn!("bad pipe message ({} bytes): {e}", buf.len()),
                    }
                }
                unsafe {
                    let _ = DisconnectNamedPipe(h);
                    let _ = CloseHandle(h);
                }
            }
        })
        .expect("spawn pipe thread");
}

struct PipeReader(HANDLE);

impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut n = 0u32;
        match unsafe { ReadFile(self.0, Some(buf), Some(&mut n), None) } {
            Ok(()) => Ok(n as usize),
            // the writer closed: end of message
            Err(_) => Ok(0),
        }
    }
}
