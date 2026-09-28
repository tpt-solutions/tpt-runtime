# Changelog

All notable changes to `tpt-runtime-observe` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- Metric aggregation over time (rates, percentiles) rather than only the
  latest snapshot.
- An Archon accounting feed pushing the same `ResourceUsage` records
  (SPEC §16).
- Export to OpenTelemetry or Prometheus alongside the JSONL sink.
- Bounded event history with explicit retention policy.

## [0.1.0]

Initial release: structured events and per-workload usage.

### Added

- **`EventHub`** (`events`): `new` with an optional JSONL sink, `subscribe`
  returning a `tokio::sync::broadcast` receiver, `emit` for publishing, and
  `read_history` for replaying persisted events.
- **Broadcast fan-out**: every subscriber sees every event, and a slow
  subscriber does not stall the runtime.
- **Durable history**: events appended to a JSONL sink survive a daemon
  restart, so `tpt events` can show what happened before this process started.
- **`MetricsRegistry`** (`metrics`): `new`, `update`, `get`, `all` and
  `remove`, holding the latest `ResourceUsage` snapshot per workload.
- **`collect_from`**: samples a running workload through a closure and records
  the result, ignoring sampler failures so a transient error never breaks the
  metrics path.
- **Minimal telemetry overhead** (SPEC §47): backends push samples and the API
  serves them; nothing polls aggressively.
- CLI support for bounded history (`tpt events --limit`) and the live stream
  (`tpt events --follow`).

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
