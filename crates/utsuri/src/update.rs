//! Updates from GitHub (github.com/CratusGod/utsuri releases): a look a little after start
//! and twice a day after (Settings can turn that off), or now from the tray or Settings.
//! A newer version shows a card with Update and Later; Update downloads its setup, checks
//! its SHA-256, and runs it quietly over this install, which starts Utsuri again.
//!
//! Tests: UTSURI_UPDATE_FEED (a release list URL, file:// works), UTSURI_FAKE_VERSION (the
//! version this copy claims), UTSURI_UPDATE_AFTER_MS (first automatic look), and
//! UTSURI_UPDATE_YES=1 (install a found update without asking).

use crate::Hub;
use crate::ui::toasts::Body;
use shared::update::Release;
use std::time::Duration;

pub const SETUP: &str = "Utsuri-Setup.exe";
const EVERY: Duration = Duration::from_secs(12 * 3600);

#[derive(Default)]
pub struct Updater {
    pub found: Option<Release>,
    checking: bool,
    installing: bool,
    /// the version a card was shown for (an automatic look shows it once)
    shown: Option<String>,
}

pub fn current() -> String {
    std::env::var("UTSURI_FAKE_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string())
}

fn feed() -> String {
    std::env::var("UTSURI_UPDATE_FEED").unwrap_or_else(|_| shared::update::feed_url("CratusGod", "utsuri"))
}

fn agent() -> String {
    format!("Utsuri/{} (update check)", env!("CARGO_PKG_VERSION"))
}

/// Can this copy replace itself? (an installed copy, not a build folder)
pub fn installed() -> bool {
    #[cfg(windows)]
    {
        install::installed_here(&install::UTSURI).is_some()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Automatic looks: only for an installed copy, never in a test run (unless a test asks).
pub fn start_schedule(me: rune::AppHandle<Hub>) {
    let asked = std::env::var("UTSURI_UPDATE_AFTER_MS").ok().and_then(|v| v.parse::<u64>().ok());
    if asked.is_none() && (std::env::var_os("UTSURI_E2E").is_some() || !installed()) {
        log::info!("update: no automatic checks (not an installed copy, or a test run)");
        return;
    }
    let first = Duration::from_millis(asked.unwrap_or(30_000));
    std::thread::Builder::new()
        .name("utsuri-update".into())
        .spawn(move || {
            std::thread::sleep(first);
            loop {
                me.update(|h, _| {
                    if h.settings.check_updates {
                        h.check_updates(false);
                    }
                });
                std::thread::sleep(EVERY);
            }
        })
        .expect("update thread");
}

impl Hub {
    /// Look for a newer version. `manual`: say what came of it, even "up to date".
    pub fn check_updates(&mut self, manual: bool) {
        if self.updater.checking || self.updater.installing {
            return;
        }
        self.updater.checking = true;
        let me = self.me.clone();
        std::thread::spawn(move || {
            let r = shared::update::check(&feed(), &current(), SETUP, &agent());
            if let Some(me) = me {
                me.update(move |h, _| h.update_checked(r, manual));
            }
        });
    }

    fn update_checked(&mut self, r: Result<Option<Release>, String>, manual: bool) {
        self.updater.checking = false;
        match r {
            Ok(Some(rel)) => {
                log::info!("update: {} is available ({})", rel.version, rel.url);
                let v = rel.version.clone();
                self.updater.found = Some(rel);
                self.refresh_tray();
                if std::env::var_os("UTSURI_UPDATE_YES").is_some() {
                    self.install_update();
                    return;
                }
                if manual || self.updater.shown.as_deref() != Some(v.as_str()) {
                    self.updater.shown = Some(v.clone());
                    self.push_toast(Body::Update { version: v });
                }
            }
            Ok(None) => {
                log::info!("update: {} is the newest", current());
                self.updater.found = None;
                self.refresh_tray();
                if manual {
                    self.push_toast(Body::Info { title: "Utsuri is up to date".into(), msg: format!("You have the newest version, {}.", current()) });
                }
            }
            Err(e) => {
                log::warn!("update check: {e}");
                if manual {
                    self.error("Couldn't check for updates", e);
                }
            }
        }
    }

    /// Download the found version, check it, and run its setup over this install.
    pub fn install_update(&mut self) {
        let Some(rel) = self.updater.found.clone() else {
            self.check_updates(true);
            return;
        };
        if self.updater.installing {
            return;
        }
        if !installed() {
            #[cfg(windows)]
            self.error("Can't update this copy", "It runs from a build folder, not an install. Get the new version from the releases page.".into());
            #[cfg(not(windows))]
            crate::shell::open(std::path::Path::new(&rel.page));
            return;
        }
        self.updater.installing = true;
        log::info!("update: downloading {}", rel.version);
        let toast = self.push_toast(Body::Working { title: "Updating Utsuri".into(), detail: format!("Downloading version {}…", rel.version) });
        let me = self.me.clone();
        std::thread::spawn(move || {
            let dir = std::env::temp_dir().join("Utsuri-update");
            let r = shared::update::download(&rel, &dir, &agent());
            if let Some(me) = me {
                me.update(move |h, tx| h.update_downloaded(r, toast, tx));
            }
        });
    }

    fn update_downloaded(&mut self, r: Result<std::path::PathBuf, String>, toast: u64, tx: &mut rune::Tx) {
        let r = r.and_then(|setup| {
            log::info!("update: downloaded and checked {}", setup.display());
            #[cfg(windows)]
            return install::start_update(&install::UTSURI, &setup).map_err(|e| format!("{e:#}"));
            #[cfg(not(windows))]
            return Err(format!("run {} to update", setup.display()));
        });
        match r {
            Ok(()) => {
                log::info!("update: setup started; quitting so it can replace Utsuri");
                tx.quit();
            }
            Err(e) => {
                self.updater.installing = false;
                self.toasts(move |s| s.stack.remove(toast));
                self.error("Couldn't update Utsuri", e);
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn installer_version_is_the_apps() {
        assert_eq!(install::UTSURI.version, env!("CARGO_PKG_VERSION"), "bump crates/install AppInfo with Cargo.toml");
    }
}
