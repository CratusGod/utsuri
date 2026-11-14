//! `utsuri.exe --cli ...`: the engine without a window, answering in one JSON object.
//! The E2E harness uses `rings` to know where each slice sits (like the Python E2E's
//! build_rings), and `run` to check a slice's result headless.
//!
//!   --cli rings PATH...        the menu for this drop
//!   --cli run KEY PATH...      run a slice that needs no window
//!   --cli selftest             open, decode and encode once (packaging check)

use anyhow::Result;
use std::path::PathBuf;

fn print(v: serde_json::Value) {
    println!("{v}");
}

pub fn main(args: &[String]) -> Result<()> {
    let settings = crate::settings::Settings::load();
    settings.apply();
    let paths = |from: usize| -> Vec<PathBuf> { args.iter().skip(from).map(PathBuf::from).collect() };
    match args.first().map(String::as_str) {
        Some("rings") => {
            let rings = utsuri_engine::actions::rings(&paths(1));
            let json: Vec<serde_json::Value> = rings
                .iter()
                .map(|(name, slices)| serde_json::json!({"name": name, "slices": slices.iter().map(|s| serde_json::json!({"label": s.label, "key": s.key, "hint": s.hint})).collect::<Vec<_>>()}))
                .collect();
            print(serde_json::json!({"ok": true, "rings": json}));
        }
        Some("run") if args.len() >= 3 => match utsuri_engine::actions::run_now(&args[1], &paths(2)) {
            Ok(r) => print(serde_json::json!({"ok": true, "title": r.title, "detail": r.detail, "good": r.good, "outs": r.outs})),
            Err(e) => {
                print(serde_json::json!({"ok": false, "error": utsuri_engine::friendly(&e)}));
                std::process::exit(1);
            }
        },
        Some("fixtures") if args.len() >= 2 => {
            let files = utsuri_engine::fixtures::write_all(std::path::Path::new(&args[1]))?;
            print(serde_json::json!({"ok": true, "files": files}));
        }
        Some("selftest") => {
            let im = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(8, 8, image::Rgb([200, 100, 50])));
            let mut v = Vec::new();
            utsuri_engine::imgio::encode(&im, "WEBP", 80, &mut v)?;
            print(serde_json::json!({"ok": true, "webp_bytes": v.len(), "heic": utsuri_engine::imgio::heic_ok(), "ffmpeg": utsuri_engine::media::available(), "ocr": utsuri_engine::ocr::langs()}));
        }
        _ => {
            print(serde_json::json!({"ok": false, "error": "usage: --cli rings PATH... | run KEY PATH... | selftest"}));
            std::process::exit(2);
        }
    }
    Ok(())
}
