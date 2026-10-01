//! Tar (and tar.gz) dists, extracted as Composer's `TarDownloader` does with
//! `PharData::extractTo`: files keep their exact mode, directories get the
//! default mode, and symlink entries become empty files with the link's mode.

use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use tar::{Archive, EntryType};

use crate::error::{Error, IoContext, Result};
use crate::extract::{Entry, Limits, safe_relative, single_top_dir};

struct Item {
    rel: PathBuf,
    kind: Kind,
    mode: u32,
    data: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Dir,
}

fn decompress(package: &str, bytes: &[u8], limit: u64) -> Result<Vec<u8>> {
    let err = |reason: String| Error::Archive {
        package: package.to_owned(),
        reason,
    };
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(bytes)
            .take(limit.saturating_add(1))
            .read_to_end(&mut out)
            .map_err(|e| err(e.to_string()))?;
        if out.len() as u64 > limit {
            return Err(err(format!("more than {limit} bytes uncompressed")));
        }
        return Ok(out);
    }
    if bytes.starts_with(b"BZh") {
        return Err(err(
            "bzip2-compressed tar dists are not supported yet".to_owned()
        ));
    }
    Ok(bytes.to_vec())
}

fn read_items(package: &str, tar: &[u8], limits: Limits) -> Result<Vec<Item>> {
    let err = |reason: String| Error::Archive {
        package: package.to_owned(),
        reason,
    };
    let mut archive = Archive::new(Cursor::new(tar));
    let mut items = Vec::new();
    let mut budget = limits.max_bytes;
    for entry in archive.entries().map_err(|e| err(e.to_string()))? {
        let mut entry = entry.map_err(|e| err(e.to_string()))?;
        let kind = match entry.header().entry_type() {
            EntryType::Directory => Kind::Dir,
            EntryType::Regular | EntryType::Continuous | EntryType::Symlink | EntryType::Link => {
                Kind::File
            }
            _ => continue,
        };
        let name = entry
            .path()
            .map_err(|e| err(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        let rel = safe_relative(&name).map_err(err)?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        let mode = entry.header().mode().map_err(|e| err(e.to_string()))? & 0o777;
        let mut data = Vec::new();
        if entry.header().entry_type().is_file() {
            entry
                .by_ref()
                .take(budget.saturating_add(1))
                .read_to_end(&mut data)
                .map_err(|e| err(format!("{name}: {e}")))?;
            if data.len() as u64 > budget {
                return Err(err(format!(
                    "more than {} bytes uncompressed",
                    limits.max_bytes
                )));
            }
            budget -= data.len() as u64;
        }
        items.push(Item {
            rel,
            kind,
            mode,
            data,
        });
        if items.len() > limits.max_entries {
            return Err(err(format!(
                "more than the limit of {} entries",
                limits.max_entries
            )));
        }
    }
    Ok(items)
}

/// Extract a tar or tar.gz into `dest`, which must not exist yet.
pub(crate) fn extract_tar(package: &str, bytes: &[u8], dest: &Path, limits: Limits) -> Result<()> {
    let tar = decompress(package, bytes, limits.max_bytes)?;
    let items = read_items(package, &tar, limits)?;
    let entries: Vec<Entry> = items
        .iter()
        .enumerate()
        .map(|(index, item)| Entry {
            index,
            rel: item.rel.clone(),
            is_dir: item.kind == Kind::Dir,
        })
        .collect();
    let strip = single_top_dir(&entries);
    fs::create_dir(dest).at(dest)?;
    for item in &items {
        let rel = match &strip {
            Some(top) => match item.rel.strip_prefix(top) {
                Ok(rest) if !rest.as_os_str().is_empty() => rest,
                _ => continue,
            },
            None => item.rel.as_path(),
        };
        let path = dest.join(rel);
        if item.kind == Kind::Dir {
            fs::create_dir_all(&path).at(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).at(parent)?;
        }
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        let mut file = fs::File::create(&path).at(&path)?;
        file.write_all(&item.data).at(&path)?;
        set_mode(&file, item.mode).at(&path)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(file: &fs::File, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "same signature as the unix version"
)]
fn set_mode(_file: &fs::File, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::extract_tar;
    use crate::extract::Limits;
    use crate::testutil::TempDir;
    use std::io::Write;

    type Spec<'a> = (&'a str, tar::EntryType, u32, &'a [u8], Option<&'a str>);

    fn tar_bytes(entries: &[Spec<'_>]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, kind, mode, data, link) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_mode(*mode);
            header.set_size(data.len() as u64);
            if let Some(link) = link {
                header.set_link_name(link).unwrap();
            }
            header.set_path(name).unwrap();
            header.set_cksum();
            builder.append(&header, *data).unwrap();
        }
        builder.into_inner().unwrap()
    }

    fn gz(bytes: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(bytes).unwrap();
        enc.finish().unwrap()
    }

    fn sample() -> Vec<u8> {
        use tar::EntryType as T;
        tar_bytes(&[
            ("baz/", T::Directory, 0o755, b"", None),
            ("baz/lib/", T::Directory, 0o755, b"", None),
            ("baz/lib/priv/", T::Directory, 0o700, b"", None),
            ("baz/lib/priv/p.php", T::Regular, 0o644, b"<?php\n", None),
            (
                "baz/lib/Baz.php",
                T::Regular,
                0o600,
                b"<?php class Baz {}\n",
                None,
            ),
            ("baz/tool", T::Regular, 0o755, b"#!/bin/sh\n", None),
            ("baz/alias.php", T::Symlink, 0o755, b"", Some("lib/Baz.php")),
            ("baz/dev", T::Char, 0o644, b"", None),
        ])
    }

    #[test]
    fn extracts_like_phar_data() {
        for bytes in [sample(), gz(&sample())] {
            let tmp = TempDir::new("untar");
            let dest = tmp.path().join("out");
            extract_tar("acme/baz", &bytes, &dest, Limits::default()).unwrap();
            assert_eq!(
                std::fs::read(dest.join("lib/Baz.php")).unwrap(),
                b"<?php class Baz {}\n"
            );
            assert_eq!(std::fs::read(dest.join("alias.php")).unwrap(), b"");
            assert!(!dest.join("dev").exists());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = |p: &str| {
                    std::fs::metadata(dest.join(p))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777
                };
                assert_eq!(mode("lib/Baz.php"), 0o600);
                assert_eq!(mode("tool"), 0o755);
                assert_eq!(mode("alias.php"), 0o755);
                assert_ne!(mode("lib/priv"), 0o700);
            }
        }
    }

    #[test]
    fn refuses_unsafe_or_oversized_archives() {
        use tar::EntryType as T;
        let tmp = TempDir::new("untar-bad");
        let mut evil = tar_bytes(&[("x/", T::Directory, 0o755, b"", None)]);
        let name = b"../evil.php";
        let mut header = [0_u8; 512];
        header[..name.len()].copy_from_slice(name);
        header[100..107].copy_from_slice(b"0000644");
        header[124..135].copy_from_slice(b"00000000000");
        header[156] = b'0';
        header[148..156].copy_from_slice(b"        ");
        let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        header[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        let end = evil.len() - 1024;
        evil.splice(end..end, header);
        let err = extract_tar("a/b", &evil, &tmp.path().join("e"), Limits::default()).unwrap_err();
        assert!(err.to_string().contains("unsafe path"), "{err}");
        let small = Limits {
            max_entries: 2,
            max_bytes: 1 << 20,
        };
        assert!(extract_tar("a/b", &sample(), &tmp.path().join("s"), small).is_err());
        let tiny = Limits {
            max_entries: 100,
            max_bytes: 4,
        };
        assert!(extract_tar("a/b", &sample(), &tmp.path().join("t"), tiny).is_err());
        assert!(extract_tar("a/b", &gz(&sample()), &tmp.path().join("g"), tiny).is_err());
        let err =
            extract_tar("a/b", b"BZh91AY", &tmp.path().join("b"), Limits::default()).unwrap_err();
        assert!(err.to_string().contains("bzip2"), "{err}");
        assert!(extract_tar("a/b", &[1; 700], &tmp.path().join("j"), Limits::default()).is_err());
    }

    #[test]
    fn keeps_a_flat_archive_as_is() {
        use tar::EntryType as T;
        let tmp = TempDir::new("untar-flat");
        let bytes = tar_bytes(&[
            ("composer.json", T::Regular, 0o644, b"{}", None),
            ("src/A.php", T::Regular, 0o644, b"<?php", None),
        ]);
        let dest = tmp.path().join("flat");
        extract_tar("a/b", &bytes, &dest, Limits::default()).unwrap();
        assert!(dest.join("composer.json").is_file() && dest.join("src/A.php").is_file());
    }
}
