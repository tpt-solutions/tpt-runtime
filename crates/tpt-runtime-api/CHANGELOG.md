# Changelog

All notable changes to `tpt-runtime-api` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- Schema-version negotiation in the handshake, so clients and daemons of
  different versions can detect incompatibility before a call fails.
- An Archon shared-memory transport, keeping the method set unchanged.
- Streaming log tailing over a long-lived subscription.

## [0.1.0]

Initial release: the local named-pipe JSON API.

### Added

- **`ApiClient`** (`client`): `connect` with a three-second retry window for
  on-demand pipe instances, `call` for one request/response exchange,
  `subscribe` to opt into the live event stream, and `recv_event`.
- **Actionable connect failure**: when the daemon is unreachable, the error
  names the pipe and tells the user to start it with `tpt daemon start`.
- **Interleaving safety**: `call` skips event lines and matches responses by
  correlation `id`, so a connection that did not subscribe is unaffected.
- **Typed errors**: a failed call returns a `RuntimeError` reconstructed from
  the wire `kind`, with the failed method attached as the operation, so callers
  branch on `ErrorKind` instead of parsing text.
- **`ApiState`** (`server`): the manager, start time and a shutdown
  `watch` channel.
- **`serve`** (`server`): accepts connections until `daemon.shutdown` or the
  shutdown watch fires, bound to `DaemonConfig::pipe_name` and
  `max_clients`.
- **First-pipe-instance ownership**, so a second daemon on the same pipe fails
  immediately rather than silently splitting clients.
- Full method set across `daemon`, `workloads`, `events`, `volumes`, `devices`
  and `secrets`.
- Cross-platform compilation: a loopback TCP fallback on non-Windows hosts, so
  the workspace builds and the API can be exercised during development.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
