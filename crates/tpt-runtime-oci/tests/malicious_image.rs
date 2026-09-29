//! Malicious image tests (SPEC §46): hostile digests, poisoned tag files
//! and escape attempts in bundle entry points cannot breach the store.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tpt_runtime_core::error::ErrorKind;
use tpt_runtime_model::execution::oci_ref::ImageReference;
use tpt_runtime_oci::{Bundle, ImageStore};

const DIGEST_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn store(tag: &str) -> (ImageStore, PathBuf) {
    let base = std::env::temp_dir().join(format!(
        "tpt-oci-sec-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    (ImageStore::open(&base).unwrap(), base)
}

/// Creates a structurally valid bundle directory (has a rootfs).
fn bundle_dir(base: &Path, name: &str) -> PathBuf {
    let dir = base.join(name);
    std::fs::create_dir_all(dir.join("rootfs")).unwrap();
    dir
}

#[test]
fn poisoned_tag_file_cannot_escape_the_store() {
    let (store, base) = store("poison");
    let dir = bundle_dir(&base, "real");
    store.register_digest(DIGEST, &dir).unwrap();

    // An attacker with refs/ write access points a tag outside the store.
    let ref_dir = base.join("refs").join("evil");
    std::fs::create_dir_all(&ref_dir).unwrap();
    std::fs::write(ref_dir.join("1"), "../../../outside").unwrap();
    let reference = ImageReference::parse("evil:1").unwrap();
    let err = store.resolve(&reference).unwrap_err();
    assert_eq!(
        err.kind,
        ErrorKind::InvalidConfiguration,
        "a traversal tag must be rejected as invalid, not followed: {err}"
    );

    // A well-formed digest with no blob behind it is a loud miss, too.
    std::fs::write(
        ref_dir.join("2"),
        format!("sha256:{}{}", "f".repeat(64), "\n"),
    )
    .unwrap();
    let reference = ImageReference::parse("evil:2").unwrap();
    assert_eq!(
        store.resolve(&reference).unwrap_err().kind,
        ErrorKind::NotFound
    );

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn malformed_digests_are_rejected_everywhere() {
    let (store, base) = store("digests");
    let dir = bundle_dir(&base, "b");
    let bad = [
        "".to_owned(),
        "md5:deadbeef".to_owned(),
        "sha256:short".to_owned(),
        format!("sha256:{}", "a".repeat(65)),
        format!("sha256:{}", "z".repeat(64)),
        "../../etc/passwd".to_owned(),
        format!("sha256:{} ", "a".repeat(63)),
    ];
    for digest in &bad {
        assert!(
            store.register_digest(digest, &dir).is_err(),
            "digest '{digest}' must be rejected"
        );
    }

    // Hex is normalized to lowercase: registering with uppercase hex pins
    // the same canonical blob.
    let upper = format!("sha256:{}", DIGEST_HEX.to_ascii_uppercase());
    let dir_upper = bundle_dir(&base, "upper");
    let blob = store.register_digest(&upper, &dir_upper).unwrap();
    assert_eq!(blob.file_name().unwrap().to_string_lossy(), DIGEST_HEX);

    let reference = ImageReference::parse("app:pinned").unwrap();
    store.tag(&reference, &upper).unwrap();
    let resolved = store.resolve(&reference).unwrap();
    assert_eq!(resolved.id, DIGEST_HEX[..16]);
    assert!(resolved.rootfs_path().is_dir());

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn bundle_entry_points_cannot_escape_the_rootfs() {
    let base = std::env::temp_dir().join(format!("tpt-oci-bundle-{}", std::process::id()));
    let path = bundle_dir(&base, "b");

    let bundle = |args: Vec<String>| Bundle {
        id: "b".to_owned(),
        path: path.clone(),
        args,
        env: BTreeMap::new(),
        working_dir: None,
    };

    let hostile = [
        "../../host/pwn",
        "/bin/sh",
        "",
        // Windows-style paths: not valid OCI entry points on any host.
        "C:\\Windows\\System32\\cmd.exe",
        "dir\\file",
        "C:/Users/x/app",
        "a/../../..",
    ];
    for entry in hostile {
        assert!(
            bundle(vec![entry.to_owned()]).validate().is_err(),
            "hostile entry point '{entry}' must be rejected"
        );
    }

    for entry in ["bin/app", "usr/local/bin/app", "./bin/app"] {
        assert!(
            bundle(vec![entry.to_owned()]).validate().is_ok(),
            "benign entry point '{entry}' must be accepted"
        );
    }

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn tagged_blob_without_rootfs_is_not_resolved() {
    let (store, base) = store("rootfs");
    let dir = bundle_dir(&base, "real");
    store.register_digest(DIGEST, &dir).unwrap();
    let reference = ImageReference::parse("postgres:16").unwrap();
    store.tag(&reference, DIGEST).unwrap();

    // The blob's rootfs disappears (torn unpack, tampering): resolution
    // must refuse to hand out the bundle.
    let blob = base.join("blobs").join("sha256").join(DIGEST_HEX);
    std::fs::remove_dir_all(blob.join("rootfs")).unwrap();
    let err = store.resolve(&reference).unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    assert!(err.message.contains("rootfs"));

    std::fs::remove_dir_all(&base).ok();
}
