# Changelog

All notable changes to `tpt-runtime-capability` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- Enforce `Capability::Other` names against a registered vocabulary, so a typo
  in a manifest fails loudly instead of becoming an inert grant.
- Track grant provenance (which identity requested which capability) to
  complete the `workload → identity → capability → resource → operation`
  attribution chain of SPEC §23.

## [0.1.0]

Initial release: explicit, inspectable, revocable grants.

### Added

- **`Capability`** (`capability`): a typed grant enum covering
  `FilesystemRead`, `FilesystemWrite`, `NetworkOutbound`, `NetworkInbound`,
  `NetworkHost`, `Device { id }`, `Secret { name }`, `IpcService { service }`
  and `Other { name }`.
- **Name parsing**: `Capability::parse` maps manifest dotted names to typed
  variants, and `name()` round-trips them exactly so a capability can be
  serialized for audit and re-parsed without drift. Unrecognized names become
  `Other` rather than being dropped, keeping them auditable.
- **`CapabilitySet`** (`capability`): `empty`, `from_names`, `grant`, `revoke`,
  `grants`, `len`, `is_empty`, `is_granted` and `require`.
- **Deny by default**: an empty set grants nothing, and `require` returns a
  `CapabilityDenied` error naming the missing capability.
- **Revocation**: `revoke` reports whether a grant was actually removed, and
  backends consult the set before privileged work.
- **`CapabilityCheck`**: an event-ready check result carrying the capability
  and whether it was granted, for `capability.granted` / `capability.denied`
  events.
- `serde` support: `Capability` serializes as its name and deserializes by
  parsing; `CapabilitySet` serializes as an array of granted names.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
