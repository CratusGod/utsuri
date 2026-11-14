//! Utsuri-Setup.exe: installs Utsuri for this user (no admin rights).
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod app {
pub fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = install::Opts::from_args(&args);
    let payload = include_bytes!(env!("UTSURI_PAYLOAD"));
    let r = install::install(&install::UTSURI, payload, &o, |exe| {
        // the right-click "Utsuri…" entry and Send To, written by the app itself
        let mut c = std::process::Command::new(exe);
        c.arg("--register-shell").arg(&o.suffix);
        let s = c.status()?;
        anyhow::ensure!(s.success(), "couldn't add the right-click entry");
        Ok(())
    });
    match r {
        Ok(_) => {
            if o.silent {
                println!("installed");
            }
        }
        Err(e) => {
            if o.silent {
                eprintln!("{e:#}");
                std::process::exit(1);
            }
            install::tell("Utsuri setup", &format!("Setup didn't finish:\n\n{e:#}"), true);
        }
    }
}

}

#[cfg(windows)]
fn main() {
    app::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("This tool is Windows-only; see tests/linux for the Linux harness.");
    std::process::exit(2);
}
