//! Translating granted mounts into something a Windows workload can use
//! (SPEC §15, §23).
//!
//! An OCI manifest names a mount point like `/data`, but the sandboxed process
//! is a native Windows program: there is no `/data`, and Windows has no
//! `chroot` to create one. So the mount point is unusable as-is and the host
//! path is passed through the environment instead, under the same
//! `TPT_VOLUME_<NAME>` convention [`tpt_runtime_linux`] uses, so a workload
//! reads its volumes the same way whichever backend runs it.
//!
//! Two things are deliberately *not* claimed here:
//!
//! - **No filesystem confinement.** The host path is reachable because the
//!   workload is an ordinary user-mode process; a read-only mount is a
//!   declaration, not an enforced restriction. Real confinement needs Hyper-V
//!   or WSL.
//! - **No silent downgrade.** A read-write mount whose `filesystem.write`
//!   grant is missing is refused, not quietly served read-only. Handing back
//!   less than the manifest promised surfaces much later as a confusing write
//!   error deep inside the workload.

use tpt_runtime_capability::Capability;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_model::volume::VolumeAccessMode;
use tpt_runtime_process::{ResolvedMount, StartContext};

/// Prefix for the environment variable carrying each mount's host path.
const VOLUME_ENV_PREFIX: &str = "TPT_VOLUME_";

/// The env var name carrying a mount's host path (`proj` → `TPT_VOLUME_PROJ`).
pub fn volume_env_var(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{VOLUME_ENV_PREFIX}{}", sanitized.to_ascii_uppercase())
}

/// Whether an access mode permits writes.
pub fn allows_write(mode: VolumeAccessMode) -> bool {
    mode == VolumeAccessMode::ReadWrite
}

/// Checks that every granted mount exists on disk and that the capabilities
/// backing it are present.
///
/// Read-only mounts need `filesystem.read`; read-write mounts need
/// `filesystem.write` as well. This validates the declaration only — see the
/// module docs on confinement.
pub fn validate_mounts(ctx: &StartContext) -> Result<()> {
    for mount in &ctx.mounts {
        if !mount.host_path.is_dir() {
            return Err(RuntimeError::new(
                ErrorKind::StorageFailure,
                format!(
                    "volume '{}' host path '{}' is missing",
                    mount.name,
                    mount.host_path.display()
                ),
            )
            .with_backend("sandbox")
            .with_workload(ctx.workload_id.to_string()));
        }
        require(ctx, Capability::FilesystemRead, mount)?;
        if allows_write(mount.mode) {
            require(ctx, Capability::FilesystemWrite, mount)?;
        }
    }
    Ok(())
}

/// Reports a missing capability in terms of the mount that needed it, so the
/// operator can tell which grant to add.
fn require(ctx: &StartContext, capability: Capability, mount: &ResolvedMount) -> Result<()> {
    ctx.capabilities.require(&capability).map_err(|err| {
        err.with_backend("sandbox")
            .with_workload(ctx.workload_id.to_string())
            .with_operation(format!("mount '{}'", mount.name))
    })
}

/// The mount variables a workload sees, one per granted volume.
pub fn mount_environment(ctx: &StartContext) -> Vec<(String, String)> {
    ctx.mounts
        .iter()
        .map(|mount| {
            (
                volume_env_var(&mount.name),
                mount.host_path.display().to_string(),
            )
        })
        .collect()
}

/// A compact summary of the mounts, for `tpt inspect`.
pub fn describe_mounts(ctx: &StartContext) -> Vec<serde_json::Value> {
    ctx.mounts
        .iter()
        .map(|mount| {
            serde_json::json!({
                "name": mount.name,
                "requested_mount_point": mount.mount,
                "host_path": mount.host_path.display().to_string(),
                "mode": mount.mode.to_string(),
                // Stated plainly so an operator is not misled about the
                // strength of this mount.
                "enforced": false,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_runtime_capability::CapabilitySet;

    fn context_with(mounts: Vec<ResolvedMount>, capabilities: CapabilitySet) -> StartContext {
        StartContext {
            workload_id: tpt_runtime_core::id::WorkloadId::generate(),
            mounts,
            log_dir: std::env::temp_dir(),
            capabilities,
            network_mode: tpt_runtime_model::network::NetworkMode::None,
            exposed_ports: vec![],
        }
    }

    fn mount_at(name: &str, mode: VolumeAccessMode) -> ResolvedMount {
        ResolvedMount {
            name: name.to_owned(),
            mount: "/data".to_owned(),
            host_path: std::env::temp_dir(),
            mode,
        }
    }

    #[test]
    fn volume_env_names_match_the_linux_convention() {
        assert_eq!(volume_env_var("proj"), "TPT_VOLUME_PROJ");
        // non-alphanumeric characters collapse to underscores
        assert_eq!(volume_env_var("my-data"), "TPT_VOLUME_MY_DATA");
        assert_eq!(volume_env_var("a.b"), "TPT_VOLUME_A_B");
    }

    #[test]
    fn read_only_mounts_need_only_read() {
        let ctx = context_with(
            vec![mount_at("data", VolumeAccessMode::ReadOnly)],
            CapabilitySet::from_names(["filesystem.read"]),
        );
        assert!(validate_mounts(&ctx).is_ok());
    }

    #[test]
    fn read_write_mounts_require_the_write_grant() {
        let ctx = context_with(
            vec![mount_at("data", VolumeAccessMode::ReadWrite)],
            CapabilitySet::from_names(["filesystem.read"]),
        );
        let err = validate_mounts(&ctx).unwrap_err();
        // A read-write mount promised by the manifest must not be silently
        // downgraded to read-only.
        assert!(err.message.contains("filesystem.write"), "{err}");
        assert_eq!(err.operation.as_deref(), Some("mount 'data'"));

        let both = CapabilitySet::from_names(["filesystem.read", "filesystem.write"]);
        let ctx = context_with(vec![mount_at("data", VolumeAccessMode::ReadWrite)], both);
        assert!(validate_mounts(&ctx).is_ok());
    }

    #[test]
    fn mounts_without_the_read_grant_are_refused() {
        let ctx = context_with(
            vec![mount_at("data", VolumeAccessMode::ReadOnly)],
            CapabilitySet::empty(),
        );
        assert!(validate_mounts(&ctx).is_err());
    }

    #[test]
    fn a_missing_host_path_is_a_storage_failure() {
        let mut mount = mount_at("gone", VolumeAccessMode::ReadOnly);
        mount.host_path = std::env::temp_dir().join("tpt-definitely-not-here-xyz");
        let ctx = context_with(vec![mount], CapabilitySet::from_names(["filesystem.read"]));
        assert_eq!(
            validate_mounts(&ctx).unwrap_err().kind,
            ErrorKind::StorageFailure
        );
    }

    #[test]
    fn no_mounts_means_nothing_to_validate() {
        let ctx = context_with(vec![], CapabilitySet::empty());
        assert!(validate_mounts(&ctx).is_ok());
        assert!(mount_environment(&ctx).is_empty());
    }

    #[test]
    fn mount_paths_reach_the_environment() {
        let ctx = context_with(
            vec![mount_at("proj", VolumeAccessMode::ReadWrite)],
            CapabilitySet::empty(),
        );
        let env = mount_environment(&ctx);
        assert_eq!(env.len(), 1);
        assert_eq!(env[0].0, "TPT_VOLUME_PROJ");
        assert!(env[0].1.contains("Temp"), "{}", env[0].1);
    }

    #[test]
    fn describe_states_that_mounts_are_not_enforced() {
        let ctx = context_with(
            vec![mount_at("proj", VolumeAccessMode::ReadWrite)],
            CapabilitySet::empty(),
        );
        let described = describe_mounts(&ctx);
        assert_eq!(described[0]["name"], "proj");
        assert_eq!(described[0]["requested_mount_point"], "/data");
        // The declaration is not enforcement, and `inspect` must not imply it is.
        assert_eq!(described[0]["enforced"], false);
    }
}
