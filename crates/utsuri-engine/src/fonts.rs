//! System fonts by their Windows file name ("segoeui.ttf", "consola.ttf"…). On Windows
//! they're read from %WINDIR%\Fonts; elsewhere fontconfig picks the closest installed
//! TrueType face (Noto Sans / DejaVu, which cover Vietnamese).

use std::path::PathBuf;

/// The bytes of the font Windows calls `name`, or its closest Linux equivalent.
pub fn read(name: &str) -> Option<Vec<u8>> {
    std::fs::read(path(name)?).ok()
}

#[cfg(windows)]
fn path(name: &str) -> Option<PathBuf> {
    let dir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    Some(dir.join("Fonts").join(name))
}

#[cfg(not(windows))]
fn path(name: &str) -> Option<PathBuf> {
    let pattern = pattern(name)?;
    let out = std::process::Command::new("fc-match").args(["-f", "%{file}", &pattern]).stdin(std::process::Stdio::null()).output().ok()?;
    let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    p.is_file().then_some(p)
}

/// The fontconfig pattern for a Windows font file name.
#[cfg(not(windows))]
fn pattern(name: &str) -> Option<String> {
    let stem = name.to_ascii_lowercase();
    let stem = stem.trim_end_matches(".ttf");
    let (family, style) = match stem {
        "segoeui" | "arial" => ("Noto Sans,DejaVu Sans,sans-serif", ""),
        "segoeuib" | "arialbd" => ("Noto Sans,DejaVu Sans,sans-serif", ":bold"),
        "segoeuii" | "ariali" => ("Noto Sans,DejaVu Sans,sans-serif", ":italic"),
        "segoeuiz" | "arialbi" => ("Noto Sans,DejaVu Sans,sans-serif", ":bold:italic"),
        "consola" | "cour" => ("Noto Sans Mono,DejaVu Sans Mono,monospace", ""),
        _ => return None,
    };
    Some(format!("{family}{style}:fontformat=TrueType"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn regular_and_mono_resolve() {
        assert!(super::read("segoeui.ttf").is_some_and(|b| b.len() > 10_000));
        assert!(super::read("consola.ttf").is_some_and(|b| b.len() > 10_000));
    }
}
