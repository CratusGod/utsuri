//! Pack anything into ZIP/TAR, unpack safely. Ported from archive.py.
//! Writes go to a hidden ".utsuri-*.part" name first and are renamed into place at the
//! end, so a failed job never leaves half an archive or folder behind.

use crate::kinds::{archive_stem, archive_suffix};
use crate::out::{output_dir, temp_part, unique_path};
use crate::refuse;
use anyhow::Result;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

/// id -> (label, suffix, hint)
pub const FORMATS: &[(&str, &str, &str, &str)] = &[("zip", "ZIP", ".zip", "Opens anywhere"), ("tar-gz", "TAR.GZ", ".tar.gz", "Linux standard"), ("tar-xz", "TAR.XZ", ".tar.xz", "Smallest")];
pub const MAX_MEMBERS: usize = 200_000;
/// Leave this much disk free after extracting.
pub const SPARE_BYTES: u64 = 256 * 1024 * 1024;

/// (path, name inside the archive) for `item` and everything under it, plus how many
/// links and special files were left out. Links are never followed.
fn entries(item: &Path) -> (Vec<(PathBuf, String)>, usize) {
    let Ok(meta) = std::fs::symlink_metadata(item) else { return (vec![], 1) };
    let base = item.parent().unwrap_or(Path::new(""));
    if meta.file_type().is_symlink() {
        return (vec![], 1);
    }
    let name = |p: &Path| p.strip_prefix(base).unwrap_or(p).components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
    if meta.is_file() {
        return (vec![(item.to_path_buf(), name(item))], 0);
    }
    let mut out = vec![(item.to_path_buf(), name(item))];
    let mut skipped = 0;
    let mut stack = vec![item.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut kids: Vec<PathBuf> = std::fs::read_dir(&dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        kids.sort();
        let mut subdirs = vec![];
        for k in kids {
            match std::fs::symlink_metadata(&k) {
                Ok(m) if m.file_type().is_symlink() => skipped += 1,
                Ok(m) if m.is_dir() => {
                    out.push((k.clone(), name(&k)));
                    subdirs.push(k);
                }
                Ok(m) if m.is_file() => out.push((k.clone(), name(&k))),
                _ => skipped += 1,
            }
        }
        stack.extend(subdirs.into_iter().rev());
    }
    (out, skipped)
}

/// Archive `paths` into one file beside the first. One item is named after itself,
/// several become "Archive". Returns (archive, entries written, skipped).
pub fn pack(paths: &[PathBuf], fmt: &str) -> Result<(PathBuf, usize, usize)> {
    let Some(&(_, _, suffix, _)) = FORMATS.iter().find(|f| f.0 == fmt) else { refuse!("Unknown archive format '{fmt}'") };
    let first = &paths[0];
    let name = if paths.len() == 1 { if first.is_dir() { first.file_name().unwrap_or_default().to_string_lossy().into_owned() } else { archive_stem(first) } } else { "Archive".into() };
    let dest = unique_path(&output_dir(first).join(format!("{name}{suffix}")));
    let mut all = vec![];
    let mut skipped = 0;
    for p in paths {
        let (e, s) = entries(p);
        all.extend(e.into_iter().filter(|(src, _)| *src != dest));
        skipped += s;
    }
    if all.is_empty() {
        refuse!("Nothing to archive (only links or special files)");
    }
    let part = temp_part(dest.parent().unwrap_or(Path::new(".")), ".part");
    let r = (|| -> Result<()> {
        let f = std::fs::File::create(&part)?;
        match fmt {
            "zip" => {
                let mut z = zip::ZipWriter::new(f);
                let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).compression_level(Some(6)).large_file(true);
                for (src, arc) in &all {
                    if src.is_dir() {
                        z.add_directory(format!("{arc}/"), o)?;
                    } else {
                        z.start_file(arc.as_str(), o)?;
                        std::io::copy(&mut std::fs::File::open(src)?, &mut z)?;
                    }
                }
                z.finish()?;
            }
            _ => {
                let w: Box<dyn Write> = if fmt == "tar-gz" {
                    Box::new(flate2::write::GzEncoder::new(f, flate2::Compression::default()))
                } else {
                    Box::new(liblzma::write::XzEncoder::new(f, 6))
                };
                let mut t = tar::Builder::new(w);
                t.follow_symlinks(false);
                for (src, arc) in &all {
                    if src.is_dir() {
                        t.append_dir(arc, src)?;
                    } else {
                        t.append_path_with_name(src, arc)?;
                    }
                }
                t.into_inner()?.flush()?;
            }
        }
        std::fs::rename(&part, &dest)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(&part);
    r?;
    Ok((dest, all.len(), skipped))
}

fn safe_rel(name: &str) -> Result<PathBuf> {
    let n = name.replace('\\', "/");
    let p = Path::new(&n);
    if n.starts_with('/') || (n.len() > 1 && n.as_bytes()[1] == b':') || p.components().any(|c| matches!(c, Component::ParentDir | Component::Prefix(_) | Component::RootDir)) {
        refuse!("Refusing to extract: '{name}' points outside the folder");
    }
    Ok(p.components().filter(|c| matches!(c, Component::Normal(_))).collect())
}

fn room_for(total: u64, where_: &Path) -> Result<()> {
    let free = free_space(where_);
    if let Some(free) = free {
        if total + SPARE_BYTES > free {
            refuse!("Not enough disk space: needs {} MB, {} MB free", total / 1024 / 1024, free / 1024 / 1024);
        }
    }
    Ok(())
}

fn free_space(p: &Path) -> Option<u64> {
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = p.as_os_str().encode_wide_null();
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(windows::core::PCWSTR(wide.as_ptr()), Some(&mut free), None, None).ok()? };
    Some(free)
}

trait Wide {
    fn encode_wide_null(&self) -> Vec<u16>;
}

impl Wide for std::ffi::OsStr {
    fn encode_wide_null(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().chain([0]).collect()
    }
}

fn open_tar(src: &Path) -> Result<tar::Archive<Box<dyn Read>>> {
    let f = std::fs::File::open(src)?;
    let name = src.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    let r: Box<dyn Read> = if name.ends_with(".gz") || name.ends_with(".tgz") {
        Box::new(flate2::read::GzDecoder::new(f))
    } else if name.ends_with(".xz") || name.ends_with(".txz") {
        Box::new(liblzma::read::XzDecoder::new(f))
    } else if name.ends_with(".bz2") || name.ends_with(".tbz2") {
        Box::new(bzip2::read::BzDecoder::new(f))
    } else {
        Box::new(f)
    };
    Ok(tar::Archive::new(r))
}

/// Extract into a new folder beside the archive. An archive holding exactly one
/// top-level folder isn't wrapped in a second one. Returns (folder, entries).
pub fn unpack(src: &Path) -> Result<(PathBuf, usize)> {
    let where_ = output_dir(src);
    let tmp = temp_part(&where_, ".part");
    std::fs::create_dir_all(&tmp)?;
    let bad = |e: &dyn std::fmt::Display| crate::UserError(format!("{} isn't a readable archive ({e})", src.file_name().unwrap_or_default().to_string_lossy()));
    let r = (|| -> Result<(PathBuf, usize)> {
        let count;
        if archive_suffix(src) == ".zip" {
            let mut z = zip::ZipArchive::new(std::fs::File::open(src)?).map_err(|e| bad(&e))?;
            if z.len() > MAX_MEMBERS {
                refuse!("Too many entries ({})", z.len());
            }
            let mut total = 0u64;
            for i in 0..z.len() {
                let f = z.by_index_raw(i).map_err(|e| bad(&e))?;
                safe_rel(&f.name().map(|n| n.into_owned()).unwrap_or_default())?;
                if f.is_symlink() {
                    refuse!("Refusing to extract: '{}' is a symbolic link", f.name().map(|n| n.into_owned()).unwrap_or_default());
                }
                if f.encrypted() {
                    refuse!("Password-protected ZIPs aren't supported");
                }
                total += f.size();
            }
            room_for(total, &where_)?;
            for i in 0..z.len() {
                let mut f = z.by_index(i).map_err(|e| bad(&e))?;
                let rel = safe_rel(&f.name().map(|n| n.into_owned()).unwrap_or_default())?;
                let dest = tmp.join(&rel);
                if f.is_dir() {
                    std::fs::create_dir_all(&dest)?;
                } else {
                    if let Some(p) = dest.parent() {
                        std::fs::create_dir_all(p)?;
                    }
                    std::io::copy(&mut f, &mut std::fs::File::create(&dest)?)?;
                }
            }
            count = z.len();
        } else {
            // first pass: check every member and the total size
            let mut total = 0u64;
            let mut n = 0usize;
            for e in open_tar(src)?.entries().map_err(|e| bad(&e))? {
                let e = e.map_err(|e| bad(&e))?;
                let path = e.path().map_err(|e| bad(&e))?.to_string_lossy().into_owned();
                safe_rel(&path)?;
                let t = e.header().entry_type();
                if t.is_symlink() || t.is_hard_link() {
                    refuse!("Refusing to extract: '{path}' is a link");
                }
                if !(t.is_file() || t.is_dir()) && !t.is_pax_global_extensions() && !t.is_pax_local_extensions() && !t.is_gnu_longname() {
                    refuse!("Refusing to extract: '{path}' is a special file");
                }
                total += e.size();
                n += 1;
                if n > MAX_MEMBERS {
                    refuse!("Too many entries");
                }
            }
            room_for(total, &where_)?;
            for e in open_tar(src)?.entries().map_err(|e| bad(&e))? {
                let mut e = e.map_err(|e| bad(&e))?;
                let rel = safe_rel(&e.path().map_err(|e| bad(&e))?.to_string_lossy())?;
                if rel.as_os_str().is_empty() {
                    continue;
                }
                let dest = tmp.join(&rel);
                if e.header().entry_type().is_dir() {
                    std::fs::create_dir_all(&dest)?;
                } else if e.header().entry_type().is_file() {
                    if let Some(p) = dest.parent() {
                        std::fs::create_dir_all(p)?;
                    }
                    std::io::copy(&mut e, &mut std::fs::File::create(&dest)?)?;
                }
            }
            count = n;
        }
        let top: Vec<PathBuf> = std::fs::read_dir(&tmp)?.flatten().map(|e| e.path()).collect();
        let dest = if top.len() == 1 && top[0].is_dir() {
            let d = unique_path(&where_.join(top[0].file_name().unwrap()));
            std::fs::rename(&top[0], &d)?;
            let _ = std::fs::remove_dir(&tmp);
            d
        } else {
            let d = unique_path(&where_.join(archive_stem(src)));
            std::fs::rename(&tmp, &d)?;
            d
        };
        Ok((dest, count))
    })();
    if tmp.exists() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::tmpdir;

    fn tree(d: &Path) -> PathBuf {
        let root = d.join("stuff");
        std::fs::create_dir_all(root.join("sub/deeper")).unwrap();
        std::fs::write(root.join("a.txt"), "alpha").unwrap();
        std::fs::write(root.join("sub/b.txt"), "beta").unwrap();
        std::fs::write(root.join("sub/deeper/c.bin"), vec![7u8; 5000]).unwrap();
        root
    }

    #[test]
    fn pack_and_unpack_every_format() {
        let d = tmpdir("arch");
        let root = tree(&d);
        for (fmt, _, suffix, _) in FORMATS {
            let (a, n, skipped) = pack(std::slice::from_ref(&root), fmt).unwrap();
            assert_eq!(a.file_name().unwrap().to_string_lossy(), format!("stuff{suffix}"));
            assert_eq!((n, skipped), (6, 0), "{fmt}");
            let (dest, count) = unpack(&a).unwrap();
            // one top-level folder: not wrapped again; "stuff" exists so it's "stuff (1)"
            assert_eq!(dest.file_name().unwrap(), "stuff (1)", "{fmt}");
            assert!(count >= 6);
            assert_eq!(std::fs::read_to_string(dest.join("sub/b.txt")).unwrap(), "beta");
            assert_eq!(std::fs::read(dest.join("sub/deeper/c.bin")).unwrap().len(), 5000);
            std::fs::remove_dir_all(&dest).unwrap();
            std::fs::remove_file(&a).unwrap();
        }
        // several items -> "Archive"
        let x = d.join("x.txt");
        std::fs::write(&x, "x").unwrap();
        let (a, _, _) = pack(&[root.clone(), x], "zip").unwrap();
        assert_eq!(a.file_name().unwrap(), "Archive.zip");
        assert!(std::fs::read_dir(&d).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".utsuri-")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn refuses_zip_slip() {
        let d = tmpdir("slip");
        let a = d.join("evil.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&a).unwrap());
        z.start_file("../escape.txt", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"x").unwrap();
        z.finish().unwrap();
        let e = unpack(&a).unwrap_err();
        assert!(crate::friendly(&e).contains("points outside"));
        assert!(!d.parent().unwrap().join("escape.txt").exists());
        assert!(std::fs::read_dir(&d).unwrap().count() == 1, "no leftovers");
        let bad = d.join("bad.tar.gz");
        std::fs::write(&bad, b"not a tar").unwrap();
        assert!(crate::friendly(&unpack(&bad).unwrap_err()).contains("isn't a readable archive"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
