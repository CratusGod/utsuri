//! Child processes without a console window (Windows) and detached launches, behind one
//! platform-neutral API so callers don't carry `cfg`s.

use std::process::Command;

pub trait Quiet {
    /// No console window flashes up for this child (Windows); a no-op elsewhere.
    fn quiet(&mut self) -> &mut Self;
    /// Runs on after this process exits, in its own process group.
    fn detached(&mut self) -> &mut Self;
}

impl Quiet for Command {
    #[cfg(windows)]
    fn quiet(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
    }

    #[cfg(not(windows))]
    fn quiet(&mut self) -> &mut Self {
        self
    }

    #[cfg(windows)]
    fn detached(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(0x0000_0008 | 0x0000_0200) // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    }

    #[cfg(unix)]
    fn detached(&mut self) -> &mut Self {
        use std::os::unix::process::CommandExt;
        self.process_group(0)
    }
}

/// A program's file name on this platform: "ffmpeg" → "ffmpeg.exe" on Windows.
pub fn exe_name(stem: &str) -> String {
    if cfg!(windows) { format!("{stem}.exe") } else { stem.to_string() }
}
