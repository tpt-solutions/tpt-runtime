# Changelog

All notable changes to `tpt-runtime-ipc` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- An Archon shared-memory transport replacing the named-pipe byte stream,
  without changing API semantics (SPEC §16).
- Batched requests, to amortize per-message overhead for high-frequency
  clients.
- Schema-version negotiation in the handshake.

## [0.1.0]

Initial release: the local API envelope and framing.

### Added

- **`Request`** (`envelope`): a correlation `id`, a method name and JSON
  params, with serde round-tripping.
- **`Response`** (`envelope`): carrying exactly one of `result` or `error`,
  enforced by construction through `ok`, `err` and `error`.
- **`ResponseError`** (`envelope`): a snake_case `kind`, a human-readable
  message and optional workload, backend and operation attribution, with
  `From<&RuntimeError>` so core errors map onto the wire without loss.
- **`read_message` / `write_message`** (`framing`): newline-delimited UTF-8 JSON
  over any async byte stream, generic over the transport so the same framing
  serves named pipes today and a shared-memory channel later.
- **Clean end-of-stream handling**: `read_message` returns `None` rather than
  an error when the peer closes the connection.
- **Event messages**: `RuntimeEvent` values are streamed as their own messages
  with no `id`, server to client only.
- Documented the wire protocol: transport, framing, the method list, and the
  request/response/event shapes, shared by the daemon, the CLI, manual probing
  and the planned VS Code extension.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
