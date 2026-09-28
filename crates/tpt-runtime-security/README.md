# tpt-runtime-security

[![crate](https://img.shields.io/badge/crate-tpt--runtime--security-orange)](https://crates.io/crates/tpt-runtime-security)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--security-blue)](https://docs.rs/tpt-runtime-security)

Secrets (SPEC §24) and audit attribution (SPEC §23).

Secrets are **not** ordinary environment variables by default. A workload
receives an ephemeral capability, and values are only released through
`SecretStore::resolve` — which *requires* the matching capability as
authorization.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-security = "0.1"
```

## The authorization model

Reading a secret value requires presenting a `Capability::Secret` for that
exact name. There is no method that returns a value without a capability, so a
workload that was never granted `secret:github-token` cannot obtain it.

## Creating and listing secrets

```rust
use tpt_runtime_security::SecretStore;

let path = std::env::temp_dir()
    .join(format!("tpt-secrets-{}.json", std::process::id()));

let mut store = SecretStore::open(&path).unwrap();
store.set("github-token", "ghp_example_value").unwrap();
store.set("db-password", "hunter2").unwrap();

// Listing shows names and creation times -- never values.
let summaries = store.list();
assert_eq!(summaries.len(), 2);
for summary in &summaries {
    println!("{} created at {:?}", summary.name, summary.created_at);
}
```

`SecretSummary` carries only a name and a timestamp, so values cannot leak
through a listing or an API response.

## Resolving requires the matching capability

```rust
use tpt_runtime_capability::Capability;
use tpt_runtime_core::error::ErrorKind;
use tpt_runtime_security::SecretStore;

let path = std::env::temp_dir()
    .join(format!("tpt-secrets-resolve-{}.json", std::process::id()));

let mut store = SecretStore::open(&path).unwrap();
store.set("github-token", "ghp_example_value").unwrap();

// With the right capability, the value is released.
let granted = Capability::parse("secret:github-token");
assert_eq!(store.resolve(&granted).unwrap(), "ghp_example_value");

// Without it, resolution fails.
let err = store.resolve(&Capability::parse("secret:db-password")).unwrap_err();
assert_ne!(err.kind, ErrorKind::Other);   // denied, not merely absent
```

Resolving a secret that does not exist is reported separately from being denied
access, so a misconfigured manifest is distinguishable from a missing grant.

## Deleting secrets

```rust
use tpt_runtime_security::SecretStore;

let path = std::env::temp_dir()
    .join(format!("tpt-secrets-delete-{}.json", std::process::id()));

let mut store = SecretStore::open(&path).unwrap();
store.set("temporary", "value").unwrap();

// delete reports whether anything was removed.
assert!(store.delete("temporary").unwrap());
assert!(!store.delete("temporary").unwrap());
assert!(store.list().is_empty());
```

## Using secrets from a workload

Set a secret through the CLI — the value is read from stdin, so it does not
appear in shell history or process arguments:

```console
tpt secret set github-token
```

Then reference it in a manifest:

```toml
[[capabilities]]
name = "secret:github-token"
```

## From the CLI

```console
tpt secret set github-token     # value read from stdin
tpt secret list                 # names only
tpt secret delete github-token
```

## Secrets at rest

The MVP store is a JSON file under the daemon state directory, and is the
fallback. A `tpt-boxcar` secret facility (DPAPI/TPM-backed) replaces the
persistence without changing the capability gate above.

## Audit attribution

Every secret resolution is attributable through the capability it required, so
the `workload → identity → capability → resource → operation` chain of
SPEC §23 stays intact.

## Testing

```console
cargo test -p tpt-runtime-security
```

Covers set/get/resolve, capability-gated access, listing redaction, delete
reporting, and persistence across store reopen.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
