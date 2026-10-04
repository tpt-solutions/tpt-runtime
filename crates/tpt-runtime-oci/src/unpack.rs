//! Layer unpacking: filesystem preparation (SPEC §13).
//!
//! Extracts OCI/Docker tar (optionally gzip-compressed) layers into an
//! accumulating rootfs, applying overlay whiteout semantics and rejecting
//! hostile layers loudly (SPEC §46 malicious-image posture): path
//! traversal, absolute escapes and out-of-rootfs symlink targets never
//! touch the host filesystem.

use std::io::Read;
use std::path::Path;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};

/// Per-file cap applied during extraction (decompression-bomb guard).
pub const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// Unpacks one layer into `rootfs` (in manifest order). Existing files are
/// overwritten; whiteouts remove lower-layer content first.
pub fn unpack_layer(layer_blob: &Path, media_type: &str, rootfs: &Path) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let file = std::fs::File::open(layer_blob)?;
    let decoder: Box<dyn Read> = if media_type.ends_with("gzip") || media_type.ends_with("+gz") {
        Box::new(flate2::read::GzDecoder::new(file))
    } else if media_type.ends_with("zstd") {
        return Err(RuntimeError::new(
            ErrorKind::NotImplemented,
            "zstd-compressed layers are not supported yet",
        )
        .with_backend("oci"));
    } else {
        Box::new(file)
    };

    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries().map_err(unpack_error)? {
        let mut entry = entry.map_err(unpack_error)?;
        let raw_path = entry.path().map_err(unpack_error)?.to_path_buf();

        // Whiteouts apply to the accumulated rootfs before this layer's
        // own content lands (OCI overlay semantics).
        if let Some(name) = raw_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
        {
            if name == ".wh..wh..opq" {
                let dir = entry_dir(rootfs, &raw_path)?;
                make_opaque(&dir)?;
                continue;
            }
            if let Some(target) = name.strip_prefix(".wh.") {
                let dir = entry_dir(rootfs, &raw_path)?;
                remove_tree(&dir.join(target));
                continue;
            }
        }

        let relative = normalize_entry(&raw_path)?;
        let dest = rootfs.join(&relative);
        let entry_type = entry.header().entry_type();
        let mode = entry.header().mode().unwrap_or(0o644);

        match entry_type {
            tar::EntryType::Directory => {
                std::fs::create_dir_all(&dest)?;
            }
            tar::EntryType::Symlink => {
                let target = entry
                    .link_name()
                    .map_err(unpack_error)?
                    .map(|t| t.to_string_lossy().to_string())
                    .unwrap_or_default();
                let dir_depth = relative.split('/').count().saturating_sub(1);
                if !symlink_target_inside(dir_depth, &target) {
                    return Err(RuntimeError::new(
                        ErrorKind::StorageFailure,
                        format!(
                            "malicious layer: symlink '{}' targets '{target}' outside the rootfs",
                            relative
                        ),
                    )
                    .with_backend("oci"));
                }
                if let Err(err) = create_symlink(&target, &dest) {
                    // Windows without developer mode cannot create
                    // symlinks; record it rather than fail the pull.
                    warnings.push(format!(
                        "symlink '{relative}' → '{target}' not created: {err}"
                    ));
                }
            }
            tar::EntryType::Link => {
                let target = entry
                    .link_name()
                    .map_err(unpack_error)?
                    .map(|t| t.to_string_lossy().to_string())
                    .unwrap_or_default();
                let normalized = normalize_entry(Path::new(&target))?;
                let source = rootfs.join(normalized);
                if !source.is_file() {
                    warnings.push(format!(
                        "hardlink '{relative}' → '{target}' has no in-rootfs target"
                    ));
                    continue;
                }
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let _ = std::fs::remove_file(&dest);
                if std::fs::hard_link(&source, &dest).is_err() {
                    std::fs::copy(&source, &dest)?;
                }
            }
            _ => {
                // Regular file (and unknown types treated as files).
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let size = entry.size();
                if size > MAX_FILE_BYTES {
                    return Err(RuntimeError::new(
                        ErrorKind::StorageFailure,
                        format!("layer file '{relative}' is {size} bytes; over the extraction cap"),
                    )
                    .with_backend("oci"));
                }
                let mut bytes = Vec::with_capacity(size as usize);
                entry.read_to_end(&mut bytes).map_err(unpack_error)?;
                std::fs::write(&dest, &bytes)?;
                apply_mode(&dest, mode);
            }
        }
    }
    Ok(warnings)
}

/// Builds a rootfs-relative path from an entry path, rejecting traversal.
/// Leading slashes and `.` components are dropped (container layers are
/// rootfs-relative by definition); `..` anywhere is hostile.
fn normalize_entry(path: &Path) -> Result<String> {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut components: Vec<&str> = Vec::new();
    for component in text.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                return Err(RuntimeError::new(
                    ErrorKind::StorageFailure,
                    format!("malicious layer: entry '{text}' escapes the rootfs"),
                )
                .with_backend("oci"))
            }
            other => components.push(other),
        }
    }
    if components.is_empty() {
        return Err(RuntimeError::new(
            ErrorKind::StorageFailure,
            format!("malicious layer: empty entry path '{text}'"),
        )
        .with_backend("oci"));
    }
    Ok(components.join("/"))
}

/// The rootfs directory an entry's whiteout applies to (its parent).
fn entry_dir(rootfs: &Path, entry_path: &Path) -> Result<std::path::PathBuf> {
    let relative = normalize_entry(entry_path)?;
    Ok(rootfs
        .join(relative)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| rootfs.to_path_buf()))
}

/// Removes everything inside `dir` (opaque whiteout); the dir itself stays.
fn make_opaque(dir: &Path) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
    Ok(())
}

/// Removes a file or directory tree if present; never fails the pull.
fn remove_tree(path: &Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

/// Whether a symlink `target` relative to a directory at `dir_depth`
/// resolves inside the rootfs (lexical check; absolute targets escape).
fn symlink_target_inside(dir_depth: usize, target: &str) -> bool {
    if target.starts_with('/') {
        return false;
    }
    let mut depth = dir_depth as i64;
    for component in target.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => depth += 1,
        }
    }
    true
}

#[cfg(unix)]
fn create_symlink(target: &str, dest: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, dest)
}

#[cfg(windows)]
fn create_symlink(target: &str, dest: &Path) -> std::io::Result<()> {
    use std::os::windows::fs;
    // File vs dir symlinks need different calls; try file, then dir.
    match fs::symlink_file(target, dest) {
        Ok(()) => Ok(()),
        Err(file_err) => match fs::symlink_dir(target, dest) {
            Ok(()) => Ok(()),
            Err(_) => Err(file_err),
        },
    }
}

/// Applies the unix mode bits where the platform allows; on Windows only
/// the read-only attribute is honored.
fn apply_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
    #[cfg(windows)]
    {
        let _ = mode;
        let _ = path;
    }
}

fn unpack_error(err: std::io::Error) -> RuntimeError {
    RuntimeError::new(
        ErrorKind::StorageFailure,
        format!("layer is not a readable tar stream: {err}"),
    )
    .with_backend("oci")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tar::Builder;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tpt-oci-unpack-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Builds a tar in memory from (path, contents) file entries plus raw
    /// directory entries.
    fn tar_bytes(entries: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
        let mut builder = Builder::new(Vec::new());
        for (path, contents) in entries {
            match contents {
                None => {
                    let mut header = tar::Header::new_gnu();
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_size(0);
                    header.set_path(path).unwrap();
                    header.set_mode(0o755);
                    header.set_cksum();
                    builder.append(&header, std::io::empty()).unwrap();
                }
                Some(bytes) => {
                    let mut header = tar::Header::new_gnu();
                    header.set_size(bytes.len() as u64);
                    header.set_mode(0o644);
                    header.set_path(path).unwrap();
                    header.set_cksum();
                    builder.append(&header, bytes.as_ref()).unwrap();
                }
            }
        }
        builder.into_inner().unwrap()
    }

    fn write_layer(dir: &Path, bytes: &[u8]) -> std::path::PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let layer = dir.join("layer.tar");
        std::fs::write(&layer, bytes).unwrap();
        layer
    }

    #[test]
    fn extracts_files_and_directories() {
        let dir = temp_dir("basic");
        let rootfs = dir.join("rootfs");
        let layer = write_layer(
            &dir,
            &tar_bytes(&[
                ("bin", None),
                ("bin/app", Some(b"binary-bytes" as &[u8])),
                ("etc/marker.txt", Some(b"hello" as &[u8])),
            ]),
        );
        unpack_layer(&layer, "application/vnd.oci.image.layer.v1.tar", &rootfs).unwrap();

        assert_eq!(
            std::fs::read(rootfs.join("bin/app")).unwrap(),
            b"binary-bytes"
        );
        assert_eq!(
            std::fs::read(rootfs.join("etc/marker.txt")).unwrap(),
            b"hello"
        );
        assert!(rootfs.join("bin").is_dir());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn whiteouts_remove_lower_layer_content() {
        let dir = temp_dir("whiteout");
        let rootfs = dir.join("rootfs");

        // Lower layer: two files.
        let lower = write_layer(
            &dir.join("a"),
            &tar_bytes(&[
                ("keep.txt", Some(b"stay" as &[u8])),
                ("gone.txt", Some(b"vanish" as &[u8])),
                ("cache/old.txt", Some(b"opaque" as &[u8])),
            ]),
        );
        unpack_layer(&lower, "application/vnd.oci.image.layer.v1.tar", &rootfs).unwrap();

        // Upper layer: whiteout gone.txt, opaque cache dir, new content.
        let upper = write_layer(
            &dir.join("b"),
            &tar_bytes(&[
                (".wh.gone.txt", Some(b"" as &[u8])),
                ("cache/.wh..wh..opq", Some(b"")),
                ("cache/fresh.txt", Some(b"new" as &[u8])),
            ]),
        );
        unpack_layer(&upper, "application/vnd.oci.image.layer.v1.tar", &rootfs).unwrap();

        assert!(!rootfs.join("gone.txt").exists(), "whiteout must delete");
        assert!(rootfs.join("keep.txt").exists(), "untouched files stay");
        assert!(
            !rootfs.join("cache/old.txt").exists(),
            "opaque whiteout clears the directory"
        );
        assert_eq!(
            std::fs::read(rootfs.join("cache/fresh.txt")).unwrap(),
            b"new"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn traversal_entries_are_rejected() {
        let dir = temp_dir("traversal");
        let rootfs = dir.join("rootfs");
        // Builder refuses `..` paths, so craft the hostile archive by
        // patching the name field of the first header block and fixing
        // its checksum by hand.
        let mut bytes = tar_bytes(&[
            ("placeholder.txt", Some(b"pwn" as &[u8])),
            ("ok.txt", Some(b"fine" as &[u8])),
        ]);
        let hostile = b"../escaped.txt";
        bytes[..hostile.len()].copy_from_slice(hostile);
        let header = &mut bytes[..512];
        header[148..156].fill(b' ');
        let sum: u32 = header.iter().map(|&b| b as u32).sum();
        header[148..156].copy_from_slice(format!("{:06o}  ", sum).as_bytes());
        let layer = write_layer(&dir, &bytes);
        let err =
            unpack_layer(&layer, "application/vnd.oci.image.layer.v1.tar", &rootfs).unwrap_err();
        assert_eq!(err.kind, ErrorKind::StorageFailure);
        assert!(err.message.contains("escapes"));
        assert!(!dir.join("escaped.txt").exists(), "no escape onto the host");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn gzip_layers_decode() {
        let dir = temp_dir("gzip");
        let rootfs = dir.join("rootfs");
        let tar = tar_bytes(&[("hello.txt", Some(b"gz" as &[u8]))]);
        let layer = dir.join("layer.tar.gz");
        let mut encoder = flate2::write::GzEncoder::new(
            std::fs::File::create(&layer).unwrap(),
            flate2::Compression::default(),
        );
        std::io::Write::write_all(&mut encoder, &tar).unwrap();
        encoder.finish().unwrap();

        unpack_layer(
            &layer,
            "application/vnd.oci.image.layer.v1.tar+gzip",
            &rootfs,
        )
        .unwrap();
        assert_eq!(std::fs::read(rootfs.join("hello.txt")).unwrap(), b"gz");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn normalize_rejects_parent_traversal_and_keeps_leading_slash_relative() {
        assert_eq!(
            normalize_entry(Path::new("usr/bin/app")).unwrap(),
            "usr/bin/app"
        );
        // Leading slash is rootfs-relative by container convention.
        assert_eq!(
            normalize_entry(Path::new("/etc/passwd")).unwrap(),
            "etc/passwd"
        );
        assert_eq!(normalize_entry(Path::new("./a/./b")).unwrap(), "a/b");
        for hostile in ["../evil", "a/../../evil", "..\\evil"] {
            assert!(normalize_entry(Path::new(hostile)).is_err(), "{hostile}");
        }
    }

    #[test]
    fn symlink_escape_is_rejected_but_internal_links_pass() {
        let dir = temp_dir("symlink");
        let rootfs = dir.join("rootfs");
        let mut builder = Builder::new(Vec::new());

        // Internal link: bin/sh → busybox (stays inside the rootfs).
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_path("bin/sh").unwrap();
        header.set_link_name("busybox").unwrap();
        header.set_cksum();
        builder.append(&header, std::io::empty()).unwrap();

        // Escaping link: link → ../../../../etc/passwd.
        let mut escape = tar::Header::new_gnu();
        escape.set_entry_type(tar::EntryType::Symlink);
        escape.set_size(0);
        escape.set_path("link").unwrap();
        escape.set_link_name("../../../../etc/passwd").unwrap();
        escape.set_cksum();
        builder.append(&escape, std::io::empty()).unwrap();

        let layer = dir.join("layer.tar");
        std::fs::write(&layer, builder.into_inner().unwrap()).unwrap();

        let err =
            unpack_layer(&layer, "application/vnd.oci.image.layer.v1.tar", &rootfs).unwrap_err();
        assert_eq!(err.kind, ErrorKind::StorageFailure);
        assert!(err.message.contains("outside the rootfs"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
