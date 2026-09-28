//! Every manifest shipped in `examples/` must parse and validate.
//!
//! This guards against the mistakes that are easy to make in TOML: duplicate
//! keys, misspelled tables (rejected by `deny_unknown_fields`) and manifests
//! that are syntactically fine but semantically invalid.

use std::path::{Path, PathBuf};

use tpt_runtime_config::Manifest;

fn examples_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR = <repo>/crates/tpt-runtime-config
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
}

#[test]
fn every_example_manifest_parses_and_validates() {
    let dir = examples_dir();
    assert!(
        dir.is_dir(),
        "examples directory not found at {}",
        dir.display()
    );

    let mut checked = 0;

    for entry in std::fs::read_dir(&dir).expect("read examples directory") {
        let path = entry.expect("read directory entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            continue;
        }

        let manifest = Manifest::from_path(&path)
            .unwrap_or_else(|err| panic!("{} failed to parse: {err}", path.display()));

        let spec = manifest
            .into_workload_spec()
            .unwrap_or_else(|err| panic!("{} failed to convert: {err}", path.display()));

        spec.validate()
            .unwrap_or_else(|err| panic!("{} failed to validate: {err}", path.display()));

        checked += 1;
    }

    assert!(
        checked >= 6,
        "expected the shipped example manifests to be found, saw {checked}"
    );
}

#[test]
fn duplicate_capability_keys_are_rejected() {
    // A repeated `name` inside one [[capabilities]] table is a TOML duplicate
    // key, not two capabilities; the two real examples guard against writing
    // it that way by mistake.
    let manifest = r#"
api = "tpt.runtime/v1"

[workload]
name = "dup"

[execution]
backend = "windows"
program = "cmd.exe"

[[capabilities]]
name = "network.outbound"
name = "filesystem.read"
"#;

    let err = Manifest::parse(manifest).expect_err("duplicate key must be rejected");
    assert!(
        err.message.contains("duplicate key"),
        "expected a duplicate-key error, got: {err}"
    );
}
