//! Resolving a bundle's OCI `config.json` into concrete host values: which
//! executable to launch, which environment block to pass, and where to run.
//!
//! This is where a Linux image is rejected on a Windows host. OCI registries
//! index images by platform and `tpt-runtime-oci` resolves `linux/amd64` by
//! default, so `postgres:16` unpacks to an ELF rootfs. Windows cannot execute
//! ELF binaries: there is no `chroot`, no ELF loader and no Linux syscall
//! surface in a user-mode process. Rather than spawning something that fails
//! obscurely, [`resolve_entry_point`] identifies the executable's format and
//! [`plan`] returns an error naming the actual fix.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_oci::Bundle;

/// The executable format found at the head of a candidate file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutableFormat {
    /// DOS/PE image — the `MZ` stub. Runs natively on Windows.
    PortableExecutable,
    /// ELF image (`\x7fELF`). Runs on Linux only.
    Elf,
    /// A script or plain data file; runnable only through an interpreter.
    Other,
}

/// The resolved, host-ready form of a bundle's process configuration.
#[derive(Clone, Debug)]
pub struct LaunchPlan {
    /// Absolute path to the executable inside the rootfs.
    pub program: PathBuf,
    /// Arguments after `argv[0]`.
    pub args: Vec<String>,
    /// Environment for the child, already sanitized and merged.
    pub env: Vec<(String, String)>,
    /// Working directory on the host.
    pub working_dir: PathBuf,
    /// The format `program` turned out to be in.
    pub format: ExecutableFormat,
}

/// Sniffs the executable format of `path` by reading its first bytes.
pub fn sniff_format(path: &Path) -> Result<ExecutableFormat> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|err| {
        RuntimeError::new(
            ErrorKind::StorageFailure,
            format!("cannot read '{}': {err}", path.display()),
        )
    })?;
    let mut head = [0u8; 4];
    let read = file.read(&mut head).map_err(|err| {
        RuntimeError::new(
            ErrorKind::StorageFailure,
            format!("cannot read '{}': {err}", path.display()),
        )
    })?;
    Ok(match &head[..read] {
        [0x4d, 0x5a, ..] => ExecutableFormat::PortableExecutable,
        [0x7f, b'E', b'L', b'F'] => ExecutableFormat::Elf,
        _ => ExecutableFormat::Other,
    })
}

/// Resolves the bundle's `argv[0]` to an executable inside the rootfs.
///
/// OCI `argv[0]` is absolute *inside the container* (`/app/server.exe`), so a
/// leading `/` is stripped and the remainder joined onto the rootfs. The path
/// is re-validated here rather than trusting `Bundle::validate`: this is the
/// boundary where a container path becomes a host path, and an escape here
/// would be a sandbox escape.
///
/// Windows images conventionally name executables with an `.exe` suffix, so a
/// candidate without an extension is also probed with `.exe` appended.
pub fn resolve_entry_point(bundle: &Bundle) -> Result<(PathBuf, ExecutableFormat)> {
    let entry = bundle.args.first().map(String::as_str).unwrap_or("").trim();
    if entry.is_empty() {
        return Err(RuntimeError::new(
            ErrorKind::InvalidConfiguration,
            "bundle has no entry point (process.args is empty)",
        ));
    }

    let relative = entry.strip_prefix('/').unwrap_or(entry);
    // Reject traversal and Windows-hostile characters before touching the fs.
    if relative.contains('\\') || relative.contains(':') {
        return Err(RuntimeError::new(
            ErrorKind::InvalidConfiguration,
            format!("bundle entry point '{entry}' is not a valid rootfs path"),
        ));
    }
    let mut depth: i64 = 0;
    for component in relative.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return Err(RuntimeError::new(
                        ErrorKind::InvalidConfiguration,
                        format!("bundle entry point '{entry}' escapes the rootfs"),
                    ));
                }
            }
            _ => depth += 1,
        }
    }

    let rootfs = bundle.rootfs_path();
    let base = rootfs.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));

    let mut candidates = vec![base.clone()];
    if base.extension().is_none() {
        candidates.push(base.with_extension("exe"));
    }

    for candidate in &candidates {
        if candidate.is_file() {
            let format = sniff_format(candidate)?;
            return Ok((candidate.clone(), format));
        }
    }

    Err(RuntimeError::new(
        ErrorKind::NotFound,
        format!(
            "bundle entry point '{entry}' does not exist in the rootfs (looked for {})",
            candidates
                .iter()
                .map(|c| c.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ))
}

/// Builds the launch plan for a bundle.
///
/// Fails when the rootfs holds a Linux image: the host is Windows and cannot
/// execute ELF binaries. The error names the alternatives instead of leaving
/// the operator with an unexplained spawn failure.
pub fn plan(bundle: &Bundle) -> Result<LaunchPlan> {
    let (program, format) = resolve_entry_point(bundle)?;

    if format == ExecutableFormat::Elf {
        let image = bundle.id.as_str();
        return Err(RuntimeError::new(
            ErrorKind::BackendUnavailable,
            format!(
                "image '{image}' unpacks to a Linux (ELF) rootfs, which cannot execute on a \
                 Windows host. Use a Windows-based image (for example \
                 `mcr.microsoft.com/windows/...` or `nanoserver`), or run the image on a \
                 Linux host via the Linux backend."
            ),
        )
        .with_backend("sandbox")
        .with_operation("plan"));
    }

    let rootfs = bundle.rootfs_path();
    let working_dir = match &bundle.working_dir {
        Some(dir) => {
            let cleaned = dir.trim_start_matches('/');
            let resolved = rootfs.join(cleaned.replace('/', std::path::MAIN_SEPARATOR_STR));
            if resolved.is_dir() {
                resolved
            } else {
                rootfs.clone()
            }
        }
        None => rootfs.clone(),
    };

    Ok(LaunchPlan {
        args: bundle.args.iter().skip(1).cloned().collect(),
        env: merge_environment(&bundle.env),
        program,
        working_dir,
        format,
    })
}

/// The minimal Windows environment every sandboxed process receives.
///
/// The daemon's own environment is never inherited (SPEC §5.2): a workload
/// must not learn the host's configuration. These variables are the minimum
/// for a Windows process to run at all.
fn base_environment() -> Vec<(String, String)> {
    let mut vars = vec![
        ("SystemRoot".to_owned(), "C:\\Windows".to_owned()),
        ("windir".to_owned(), "C:\\Windows".to_owned()),
    ];
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned());
    vars.push(("SystemDrive".to_owned(), drive));
    if let Ok(processors) = std::env::var("NUMBER_OF_PROCESSORS") {
        vars.push(("NUMBER_OF_PROCESSORS".to_owned(), processors));
    }
    vars
}

/// Merges the base environment with the image's variables, later sources
/// winning, and normalizes values so Windows path joining never produces a
/// doubled separator.
fn merge_environment(image_env: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let mut merged: BTreeMap<String, String> = BTreeMap::new();
    for (key, value) in base_environment() {
        merged.insert(key, value);
    }
    for (key, value) in image_env {
        merged.insert(key.clone(), value.clone());
    }
    merged
        .into_iter()
        .map(|(key, value)| (key, value.trim_end_matches(['\\', '/']).to_owned()))
        .collect()
}

/// Renders an environment block for `CreateProcessW`: `KEY=VALUE` pairs
/// separated by NUL, with a trailing extra NUL as the API requires.
pub fn environment_block(env: &[(String, String)]) -> Vec<u16> {
    let mut block: Vec<u16> = Vec::new();
    for (key, value) in env {
        for unit in format!("{key}={value}").encode_utf16() {
            block.push(unit);
        }
        block.push(0);
    }
    // Double NUL terminates the block.
    block.push(0);
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("tpt-sandbox-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn bundle_at(root: &Path, entry: &str) -> Bundle {
        let path = root.join("b");
        std::fs::create_dir_all(path.join("rootfs")).unwrap();
        Bundle {
            id: "test".to_owned(),
            path,
            args: vec![entry.to_owned()],
            env: BTreeMap::new(),
            working_dir: None,
        }
    }

    #[test]
    fn sniffs_executable_formats() {
        let root = temp_root("sniff");
        std::fs::write(root.join("pe.exe"), b"MZ\x90\x00").unwrap();
        std::fs::write(root.join("elf"), b"\x7fELFxxxx").unwrap();
        std::fs::write(root.join("data.txt"), b"hello world").unwrap();
        assert_eq!(
            sniff_format(&root.join("pe.exe")).unwrap(),
            ExecutableFormat::PortableExecutable
        );
        assert_eq!(
            sniff_format(&root.join("elf")).unwrap(),
            ExecutableFormat::Elf
        );
        assert_eq!(
            sniff_format(&root.join("data.txt")).unwrap(),
            ExecutableFormat::Other
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolves_absolute_and_relative_entries() {
        let root = temp_root("resolve");
        let bundle = bundle_at(&root, "/app/server.exe");
        let rootfs = bundle.rootfs_path();
        std::fs::create_dir_all(rootfs.join("app")).unwrap();
        std::fs::write(rootfs.join("app").join("server.exe"), b"MZ").unwrap();
        let (program, format) = resolve_entry_point(&bundle).unwrap();
        assert!(program.ends_with("server.exe"));
        assert_eq!(format, ExecutableFormat::PortableExecutable);

        // extensionless names probe for .exe, as Windows images use
        let bundle = bundle_at(&root, "/app/tool");
        std::fs::write(rootfs.join("app").join("tool.exe"), b"MZ").unwrap();
        let (program, _) = resolve_entry_point(&bundle).unwrap();
        assert!(program.ends_with("tool.exe"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rejects_entry_escapes() {
        let root = temp_root("escape");
        for entry in [
            "../../etc/passwd",
            "C:\\Windows\\System32\\cmd.exe",
            "..\\x",
        ] {
            let bundle = bundle_at(&root, entry);
            let err = resolve_entry_point(&bundle).unwrap_err();
            assert_eq!(err.kind, ErrorKind::InvalidConfiguration, "{entry}");
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_entry_point_is_not_found() {
        let root = temp_root("missing");
        let bundle = bundle_at(&root, "/nope/app.exe");
        let err = resolve_entry_point(&bundle).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn linux_images_are_refused_with_guidance() {
        let root = temp_root("elf");
        let bundle = bundle_at(&root, "/bin/sh");
        let rootfs = bundle.rootfs_path();
        std::fs::create_dir_all(rootfs.join("bin")).unwrap();
        std::fs::write(rootfs.join("bin").join("sh"), b"\x7fELFxxxx").unwrap();
        let err = plan(&bundle).unwrap_err();
        assert_eq!(err.kind, ErrorKind::BackendUnavailable);
        assert!(err.message.contains("Linux"), "{}", err.message);
        assert!(err.message.contains("Windows"), "{}", err.message);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn working_directory_falls_back_to_rootfs() {
        let root = temp_root("cwd");
        let bundle = bundle_at(&root, "/app.exe");
        let rootfs = bundle.rootfs_path();
        std::fs::write(rootfs.join("app.exe"), b"MZ\x00").unwrap();
        assert_eq!(plan(&bundle).unwrap().working_dir, rootfs);

        let mut with_cwd = bundle.clone();
        with_cwd.working_dir = Some("/data".to_owned());
        std::fs::create_dir_all(rootfs.join("data")).unwrap();
        assert_eq!(plan(&with_cwd).unwrap().working_dir, rootfs.join("data"));

        // a declared but absent directory falls back rather than failing
        let mut ghost = bundle;
        ghost.working_dir = Some("/missing".to_owned());
        assert_eq!(plan(&ghost).unwrap().working_dir, rootfs);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn environment_block_is_double_nul_terminated() {
        let block = environment_block(&[("A".to_owned(), "1".to_owned())]);
        assert_eq!(*block.last().unwrap(), 0);
        assert_eq!(block[block.len() - 2], 0);
        assert_eq!(block[0], u16::from(b'A'));
    }

    #[test]
    fn environment_never_inherits_host_vars() {
        std::env::set_var("TPT_SHOULD_NOT_LEAK", "secret");
        let env = merge_environment(&BTreeMap::new());
        assert!(
            !env.iter().any(|(key, _)| key == "TPT_SHOULD_NOT_LEAK"),
            "daemon environment leaked into the workload environment"
        );
        assert!(env.iter().any(|(key, _)| key == "SystemRoot"));
        std::env::remove_var("TPT_SHOULD_NOT_LEAK");
    }

    #[test]
    fn image_env_overrides_base_environment() {
        let mut image_env = BTreeMap::new();
        image_env.insert("SystemRoot".to_owned(), "D:\\Windows".to_owned());
        image_env.insert("APP_MODE".to_owned(), "prod".to_owned());
        let env = merge_environment(&image_env);
        let root = env.iter().find(|(key, _)| key == "SystemRoot").unwrap();
        assert_eq!(root.1, "D:\\Windows");
        assert!(env
            .iter()
            .any(|(key, value)| key == "APP_MODE" && value == "prod"));
    }
}
