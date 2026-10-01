//! Test helpers: temp dirs and a raw zip writer that can produce GitHub-style
//! MS-DOS entries, which the `zip` crate's writer never emits.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "phpm-store-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = crate::link::remove_tree(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = crate::link::remove_tree(&self.0);
    }
}

struct Record {
    name: String,
    made_by: u16,
    attrs: u32,
    data: Vec<u8>,
    crc: u32,
    offset: u32,
}

#[derive(Default)]
pub(crate) struct ZipBuilder {
    out: Vec<u8>,
    records: Vec<Record>,
}

impl ZipBuilder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// As GitHub writes them: made by MS-DOS 0.0, attributes 0.
    pub(crate) fn dos_file(self, name: &str, data: &[u8]) -> Self {
        self.entry(name, 0, 0, data)
    }

    #[cfg_attr(not(unix), allow(dead_code, reason = "only unix tests check modes"))]
    pub(crate) fn dos_file_attrs(self, name: &str, attrs: u32, data: &[u8]) -> Self {
        self.entry(name, 0, attrs, data)
    }

    pub(crate) fn dos_dir(self, name: &str) -> Self {
        self.entry(name, 0, 0x10, b"")
    }

    #[cfg_attr(not(unix), allow(dead_code, reason = "only unix tests check modes"))]
    pub(crate) fn dos_dir_attrs(self, name: &str, attrs: u32) -> Self {
        self.entry(name, 0, attrs, b"")
    }

    /// Made by Unix 2.3 with a full `st_mode`, as GitHub writes executables and symlinks.
    pub(crate) fn unix_file(self, name: &str, mode: u32, data: &[u8]) -> Self {
        self.entry(name, 0x0317, mode << 16, data)
    }

    #[cfg_attr(not(unix), allow(dead_code, reason = "only unix tests check modes"))]
    pub(crate) fn unix_dir(self, name: &str, mode: u32) -> Self {
        self.entry(name, 0x0317, (mode << 16) | 0x10, b"")
    }

    fn entry(mut self, name: &str, made_by: u16, attrs: u32, data: &[u8]) -> Self {
        let offset = u32::try_from(self.out.len()).unwrap();
        let crc = crc32(data);
        let len = u32::try_from(data.len()).unwrap();
        let name_len = u16::try_from(name.len()).unwrap();
        let o = &mut self.out;
        o.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        o.extend_from_slice(&10_u16.to_le_bytes());
        o.extend_from_slice(&0_u16.to_le_bytes());
        o.extend_from_slice(&0_u16.to_le_bytes());
        o.extend_from_slice(&0_u32.to_le_bytes());
        o.extend_from_slice(&crc.to_le_bytes());
        o.extend_from_slice(&len.to_le_bytes());
        o.extend_from_slice(&len.to_le_bytes());
        o.extend_from_slice(&name_len.to_le_bytes());
        o.extend_from_slice(&0_u16.to_le_bytes());
        o.extend_from_slice(name.as_bytes());
        o.extend_from_slice(data);
        self.records.push(Record {
            name: name.to_owned(),
            made_by,
            attrs,
            data: data.to_vec(),
            crc,
            offset,
        });
        self
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        let cd_start = u32::try_from(self.out.len()).unwrap();
        for r in &self.records {
            let len = u32::try_from(r.data.len()).unwrap();
            let name_len = u16::try_from(r.name.len()).unwrap();
            let o = &mut self.out;
            o.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
            o.extend_from_slice(&r.made_by.to_le_bytes());
            o.extend_from_slice(&10_u16.to_le_bytes());
            o.extend_from_slice(&0_u16.to_le_bytes());
            o.extend_from_slice(&0_u16.to_le_bytes());
            o.extend_from_slice(&0_u32.to_le_bytes());
            o.extend_from_slice(&r.crc.to_le_bytes());
            o.extend_from_slice(&len.to_le_bytes());
            o.extend_from_slice(&len.to_le_bytes());
            o.extend_from_slice(&name_len.to_le_bytes());
            o.extend_from_slice(&0_u16.to_le_bytes());
            o.extend_from_slice(&0_u16.to_le_bytes());
            o.extend_from_slice(&0_u16.to_le_bytes());
            o.extend_from_slice(&0_u16.to_le_bytes());
            o.extend_from_slice(&r.attrs.to_le_bytes());
            o.extend_from_slice(&r.offset.to_le_bytes());
            o.extend_from_slice(r.name.as_bytes());
        }
        let cd_len = u32::try_from(self.out.len()).unwrap() - cd_start;
        let count = u16::try_from(self.records.len()).unwrap();
        let o = &mut self.out;
        o.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        o.extend_from_slice(&0_u16.to_le_bytes());
        o.extend_from_slice(&0_u16.to_le_bytes());
        o.extend_from_slice(&count.to_le_bytes());
        o.extend_from_slice(&count.to_le_bytes());
        o.extend_from_slice(&cd_len.to_le_bytes());
        o.extend_from_slice(&cd_start.to_le_bytes());
        o.extend_from_slice(&0_u16.to_le_bytes());
        self.out
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
