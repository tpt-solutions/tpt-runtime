# Changelog

All notable changes to `tpt-runtime-security` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- A `tpt-boxcar` secret facility (DPAPI/TPM-backed) replacing the JSON
  persistence, without changing the capability gate.
- Value rotation, keeping the previous version valid for a grace period.
- Audit records for every resolution, persisted alongside the runtime event log.

## [0.1.0]

Initial release: a capability-gated secret store.

### Added

- **`SecretStore`** (`secrets`): `open`, `set`, `delete`, `list` and `resolve`,
  persisted as a JSON file under the daemon state directory.
- **Capability-gated resolution**: `resolve` takes a `Capability::Secret` and
  refuses to release a value without it. There is no method returning a secret
  value that bypasses this check.
- **Redacted listings**: `SecretSummary` carries only a name and a creation
  timestamp, so values cannot leak through listings or API responses.
- **Distinguishable failures**: a denied resolution and a missing secret are
  reported differently, separating a misconfigured manifest from a missing
  grant.
- **`delete` reporting**: returns whether a secret was actually removed.
- **Audit attribution** (SPEC §23): every resolution is attributable through
  the capability it required, preserving the
  `workload → identity → capability → resource → operation` chain.
- Secrets are not delivered as ordinary environment variables; a workload
  receives an ephemeral capability instead.
- CLI support with the value read from stdin, keeping it out of shell history
  and process arguments.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
