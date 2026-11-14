//! User settings, saved as JSON in %APPDATA%\Utsuri-rs\settings.json.
//! Same options as the Python app's settings.py.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// "shift", "ctrl" or "alt"
    pub trigger: String,
    /// "next" or "folder"
    pub output: String,
    pub jpg_quality: u8,
    pub pdf_dpi: u32,
    pub theme: String,
    pub ocr_lang: String,
    pub start_at_login: bool,
    pub shell_menu: bool,
    pub welcomed: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            trigger: "shift".into(),
            output: "next".into(),
            jpg_quality: 90,
            pdf_dpi: 200,
            theme: shared::theme::DEFAULT.into(),
            ocr_lang: "eng".into(),
            start_at_login: false,
            shell_menu: false,
            welcomed: false,
        }
    }
}

pub const TRIGGERS: &[(&str, &str)] = &[("shift", "Shift"), ("ctrl", "Ctrl"), ("alt", "Alt")];

fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("UTSURI_SETTINGS") {
        return PathBuf::from(p);
    }
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Utsuri-rs").join("settings.json")
}

impl Settings {
    pub fn load() -> Settings {
        let mut s: Settings = std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        // environment overrides for tests
        if let Ok(k) = std::env::var("UTSURI_KEY") {
            s.trigger = k.to_ascii_lowercase();
        }
        if let Ok(t) = std::env::var("UTSURI_THEME") {
            s.theme = t;
        }
        s.jpg_quality = s.jpg_quality.clamp(10, 100);
        s.pdf_dpi = s.pdf_dpi.clamp(50, 600);
        if !TRIGGERS.iter().any(|t| t.0 == s.trigger) {
            s.trigger = "shift".into();
        }
        s
    }

    pub fn save(&self) {
        let p = path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = shared::paths::atomic_write(&p, |tmp| std::fs::write(tmp, &t));
        }
    }

    /// Hand the engine what it needs.
    pub fn apply(&self) {
        utsuri_engine::set_options(utsuri_engine::Options {
            output_folder: self.output == "folder",
            jpg_quality: self.jpg_quality,
            pdf_dpi: self.pdf_dpi,
            ocr_lang: self.ocr_lang.clone(),
        });
    }

    pub fn key(&self) -> crate::trigger::Key {
        match self.trigger.as_str() {
            "ctrl" => crate::trigger::Key::Ctrl,
            "alt" => crate::trigger::Key::Alt,
            _ => crate::trigger::Key::Shift,
        }
    }

    pub fn trigger_label(&self) -> &'static str {
        TRIGGERS.iter().find(|t| t.0 == self.trigger).map_or("Shift", |t| t.1)
    }
}
