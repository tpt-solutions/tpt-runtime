//! Pull pipeline: registry → content store → prepared bundle (SPEC §13).
//!
//! Everything entering the store is digest-verified (manifest, config,
//! every layer); unpacking applies whiteouts in layer order; the finished
//! bundle carries a runtime-spec `config.json` and the image's run
//! defaults, ready for an isolation provider to start.

use std::collections::BTreeMap;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_model::execution::oci_ref::ImageReference;

use crate::bundle::Bundle;
use crate::registry::{ImageConfig, RegistryClient};
use crate::store::{ImageDefaults, ImageStore};

/// Pulls (or reuses cached content for) `reference` and returns the
/// prepared bundle. Layers already in the store are not re-downloaded;
/// an unpacked bundle short-circuits the whole pull.
pub fn pull(
    client: &RegistryClient,
    store: &ImageStore,
    reference: &ImageReference,
    max_blob_bytes: u64,
) -> Result<Bundle> {
    // Cache hit: a pinned, unpacked bundle needs no registry at all.
    if let Ok(existing) = store.resolve(reference) {
        return Ok(existing);
    }

    let (host, repo) = RegistryClient::split_registry(&reference.repository);
    let _ = host; // the client was built against the right host already
    if repo.is_empty() {
        return Err(RuntimeError::new(
            ErrorKind::InvalidConfiguration,
            format!(
                "reference '{}' has no remote repository path",
                reference.repository
            ),
        )
        .with_backend("oci"));
    }

    // Manifest (index → platform selection happens inside the client).
    let pulled = client.pull_manifest(&repo, reference)?;
    let manifest_digest = store.store_blob(&pulled.bytes)?;

    // Image config blob: run defaults for the workload.
    let config = ImageConfig::fetch(
        client,
        &repo,
        &pulled.manifest.config,
        store.root(),
        max_blob_bytes,
    )?;

    // Layers, streamed and digest-verified into the content store.
    for layer in &pulled.manifest.layers {
        let dest = store.blob_path(&layer.digest)?;
        if !dest.is_file() {
            client.fetch_blob(&repo, &layer.digest, &dest, max_blob_bytes)?;
        }
    }

    // Unpack into a fresh bundle directory (layer order matters).
    let bundle_dir = store.bundle_dir(&manifest_digest)?;
    let _ = std::fs::remove_dir_all(&bundle_dir);
    std::fs::create_dir_all(bundle_dir.join("rootfs"))?;
    for layer in &pulled.manifest.layers {
        let blob = store.blob_path(&layer.digest)?;
        crate::unpack::unpack_layer(&blob, &layer.media_type, &bundle_dir.join("rootfs"))?;
    }

    // Record run defaults + a runtime-spec config for the provider.
    let defaults = ImageDefaults {
        entrypoint: config.config.entrypoint.clone(),
        cmd: config.config.cmd.clone(),
        env: config.config.env.clone(),
        working_dir: config.config.working_dir.clone(),
    };
    std::fs::write(
        bundle_dir.join("image.json"),
        serde_json::to_string_pretty(&defaults)?,
    )?;
    std::fs::write(bundle_dir.join("config.json"), runtime_spec(&defaults))?;

    store.tag(reference, &manifest_digest)?;
    store.resolve(reference)
}

/// A minimal OCI runtime-spec config for the prepared bundle: what an
/// isolation provider (Boxcar-compatible) needs to start the rootfs.
fn runtime_spec(defaults: &ImageDefaults) -> String {
    let mut env = BTreeMap::new();
    for entry in &defaults.env {
        if let Some((key, value)) = entry.split_once('=') {
            env.insert(key.to_owned(), value.to_owned());
        }
    }
    let mut args = defaults.entrypoint.clone();
    args.extend(defaults.cmd.clone());
    if args.is_empty() {
        args.push("/bin/sh".to_owned());
    }
    serde_json::json!({
        "ociVersion": "1.0.2",
        "process": {
            "terminal": false,
            "args": args,
            "env": defaults.env,
            "cwd": defaults.working_dir.clone().unwrap_or_else(|| "/".to_owned()),
        },
        "root": {
            "path": "rootfs",
            "readonly": false,
        },
    })
    .to_string()
}
