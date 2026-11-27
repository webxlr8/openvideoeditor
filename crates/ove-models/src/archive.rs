//! Files that arrive inside a zip archive (a Python wheel, for the inference runtime): the archive is verified as a
//! download first, then each member is streamed out into `<name>.part`, hashed while it is written, and renamed only
//! when its size and SHA-256 match the member's pins. Only stored and deflated members are read.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::download::{io_err, part_path};
use crate::error::{ModelError, Result};
use crate::manifest::ModelFile;

const CHUNK: usize = 1 << 16;

/// Copies `from` to `dest` (through `dest.part`), checking `pin`'s size and SHA-256 (an offline install).
/// `on_bytes(done)` reports progress. A file that does not match is refused and nothing is left behind.
pub fn copy_verified(
    model: &str,
    pin: &ModelFile,
    from: &Path,
    dest: &Path,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<()> {
    let md = std::fs::metadata(from).map_err(|e| {
        ModelError::Io(format!("could not find {} for {model} in the folder ({}): {e}", pin.name, from.display()))
    })?;
    if md.len() != pin.bytes {
        return Err(ModelError::Corrupt { model: model.into(), file: pin.name.clone() });
    }
    let src = File::open(from).map_err(|e| io_err("read", from, e))?;
    write_verified(model, &pin.name, src, pin.bytes, &pin.sha256, dest, on_bytes)
}

/// Extracts `file`'s archive member from the zip archive at `archive` into `dest`, checking the member's size and
/// SHA-256. A mismatch deletes the extracted bytes and reports the file as corrupt.
pub fn extract_member(model: &str, archive: &Path, file: &ModelFile, dest: &Path) -> Result<()> {
    let Some(member) = &file.archive_member else {
        return Err(ModelError::InvalidManifest {
            id: model.into(),
            reason: format!("{} is no archive member", file.name),
        });
    };
    let corrupt = || ModelError::Corrupt { model: model.into(), file: file.name.clone() };
    let f = File::open(archive).map_err(|e| io_err("read", archive, e))?;
    let mut zip = zip::ZipArchive::new(f).map_err(|_| corrupt())?;
    let entry = zip.by_name(&member.path).map_err(|_| {
        ModelError::Download(format!("the archive for {model} has no {} (it should hold {})", member.path, file.name))
    })?;
    if entry.size() != file.bytes {
        return Err(corrupt());
    }
    // Never read more than the pinned size, whatever the archive claims.
    let limited = entry.take(file.bytes + 1);
    write_verified(model, &file.name, limited, file.bytes, &file.sha256, dest, &mut |_| {})
}

fn write_verified(
    model: &str,
    name: &str,
    mut src: impl Read,
    bytes: u64,
    sha256: &str,
    dest: &Path,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<()> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io_err("create", dir, e))?;
    }
    let part = part_path(dest);
    let result = (|| {
        let mut out = File::create(&part).map_err(|e| io_err("write", &part, e))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; CHUNK];
        let mut have = 0u64;
        loop {
            let n = src.read(&mut buf).map_err(|_| ModelError::Corrupt { model: model.into(), file: name.into() })?;
            if n == 0 {
                break;
            }
            have += n as u64;
            if have > bytes {
                return Err(ModelError::Corrupt { model: model.into(), file: name.into() });
            }
            out.write_all(&buf[..n]).map_err(|e| io_err("write", &part, e))?;
            hasher.update(&buf[..n]);
            on_bytes(have);
        }
        out.sync_all().map_err(|e| io_err("write", &part, e))?;
        if have != bytes || format!("{:x}", hasher.finalize()) != sha256 {
            return Err(ModelError::Corrupt { model: model.into(), file: name.into() });
        }
        Ok(())
    })();
    match result {
        Ok(()) => std::fs::rename(&part, dest).map_err(|e| io_err("install", dest, e)),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}
