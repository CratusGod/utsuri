//! Video and audio through the system ffmpeg. Ported from media.py.
//! Every call is an argv list, reads nothing from stdin, and writes a hidden ".part"
//! file that is renamed into place only when ffmpeg succeeds.

use crate::kinds::Kind;
use crate::refuse;
use anyhow::Result;
use crate::proc::Quiet as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const VIDEO_TARGETS: &[&str] = &["MP4", "WEBM", "GIF", "MP3"];
pub const AUDIO_TARGETS: &[&str] = &["MP3", "OPUS", "WAV", "FLAC"];

pub fn hint(t: &str) -> &'static str {
    match t {
        "MP4" => "Plays everywhere",
        "WEBM" => "Open web video",
        "GIF" => "First 15 s · 480 px",
        "MP3" => "Universal audio",
        "OPUS" => "Small, great quality",
        "WAV" => "Uncompressed",
        "FLAC" => "Lossless",
        _ => "",
    }
}

const EVEN: &str = "scale=trunc(iw/2)*2:trunc(ih/2)*2";

fn args(target: &str) -> Vec<&'static str> {
    match target {
        "MP4" => vec!["-vf", EVEN, "-c:v", "libx264", "-crf", "23", "-preset", "medium", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "160k", "-movflags", "+faststart"],
        "WEBM" => vec!["-c:v", "libvpx-vp9", "-crf", "33", "-b:v", "0", "-row-mt", "1", "-c:a", "libopus", "-b:a", "128k"],
        "GIF" => vec!["-t", "15", "-an", "-vf", "fps=15,scale='min(480,iw)':-1:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse", "-loop", "0"],
        "MP3" => vec!["-vn", "-c:a", "libmp3lame", "-q:a", "2"],
        "OPUS" => vec!["-vn", "-c:a", "libopus", "-b:a", "128k"],
        "WAV" => vec!["-vn", "-c:a", "pcm_s16le"],
        "FLAC" => vec!["-vn", "-c:a", "flac"],
        "COMPRESS" => vec!["-vf", "scale=-2:'min(1080,trunc(ih/2)*2)'", "-c:v", "libx264", "-crf", "28", "-preset", "medium", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "128k", "-movflags", "+faststart"],
        _ => vec![],
    }
}

pub fn ffmpeg() -> Option<PathBuf> {
    crate::ocr::which(&crate::proc::exe_name("ffmpeg"))
}

pub fn available() -> bool {
    use std::sync::OnceLock;
    static A: OnceLock<bool> = OnceLock::new();
    *A.get_or_init(|| ffmpeg().is_some())
}

pub fn targets_for(kind: Kind, src: &Path) -> Vec<&'static str> {
    let e = crate::kinds::ext(src);
    let cur = match e.as_str() {
        "m4v" => "MP4".to_string(),
        "ogg" => "OPUS".to_string(),
        e => e.to_ascii_uppercase(),
    };
    let all = match kind {
        Kind::Video => VIDEO_TARGETS,
        Kind::Audio => AUDIO_TARGETS,
        _ => &[],
    };
    all.iter().copied().filter(|t| *t != cur).collect()
}

fn run_ffmpeg(src: &Path, out: &Path, a: &[&str]) -> Result<PathBuf> {
    let Some(exe) = ffmpeg() else { refuse!("ffmpeg isn't installed. Run: winget install Gyan.FFmpeg") };
    let ext = out.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let part = crate::out::temp_part(out.parent().unwrap_or(Path::new(".")), &format!(".part{ext}"));
    let r = (|| -> Result<PathBuf> {
        let res = Command::new(exe)
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(src)
            .args(a)
            .arg(&part)
            .stdin(Stdio::null())
            .quiet()
            .output()?;
        let err = String::from_utf8_lossy(&res.stderr);
        if !res.status.success() || crate::out::file_size(&part) == 0 {
            let mut msg = err.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("ffmpeg failed").to_string();
            if err.contains("does not contain any stream") || err.contains("matches no streams") {
                msg = if a.contains(&"-vn") { "This file has no audio track".into() } else { "This file has no video track".into() };
            }
            refuse!("{}: {msg}", src.file_name().unwrap_or_default().to_string_lossy());
        }
        std::fs::rename(&part, out)?;
        Ok(out.to_path_buf())
    })();
    let _ = std::fs::remove_file(&part);
    r
}

pub fn convert(src: &Path, kind: Kind, target: &str) -> Result<PathBuf> {
    if !targets_for(kind, src).contains(&target) {
        refuse!("Can't convert {} to {target}", kind.name());
    }
    run_ffmpeg(src, &crate::out::sibling(src, "", Some(&target.to_ascii_lowercase())), &args(target))
}

pub fn compress_video(src: &Path) -> Result<PathBuf> {
    run_ffmpeg(src, &crate::out::sibling(src, "Compressed", Some("mp4")), &args("COMPRESS"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 2-second test clip with sound (needs ffmpeg).
    pub fn clip(dir: &Path) -> Option<PathBuf> {
        let exe = ffmpeg()?;
        let p = dir.join("clip.mp4");
        let ok = Command::new(exe)
            .args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc=duration=2:size=320x240:rate=15", "-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-shortest", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"])
            .arg(&p)
            .quiet()
            .status()
            .ok()?
            .success();
        ok.then_some(p)
    }

    #[test]
    fn video_and_audio_conversions() {
        let d = crate::testutil::tmpdir("media");
        let Some(src) = clip(&d) else {
            eprintln!("ffmpeg not installed: skipped");
            return;
        };
        assert_eq!(targets_for(Kind::Video, &src), vec!["WEBM", "GIF", "MP3"]);
        for t in ["GIF", "MP3"] {
            let o = convert(&src, Kind::Video, t).unwrap();
            assert!(crate::out::file_size(&o) > 0, "{t}");
        }
        let mp3 = d.join("clip.mp3");
        let wav = convert(&mp3, Kind::Audio, "WAV").unwrap();
        assert!(crate::out::file_size(&wav) > 100_000);
        let c = compress_video(&src).unwrap();
        assert_eq!(c.file_name().unwrap(), "clip Compressed.mp4");
        // an audio file has no video track
        let e = convert(&mp3, Kind::Audio, "MP4").unwrap_err();
        assert!(crate::friendly(&e).contains("Can't convert"));
        // no leftovers
        assert!(std::fs::read_dir(&d).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".utsuri-")));
        let _ = std::fs::remove_dir_all(&d);
    }
}
