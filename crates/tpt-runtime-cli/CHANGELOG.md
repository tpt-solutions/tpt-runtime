# Changelog

All notable changes to `tpt-runtime-cli` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- Shell completions for PowerShell, bash and zsh.
- A machine-readable output mode (`--output json`) for scripting.
- Interactive log follow with filtering by event name and workload.
- `tpt wait`, blocking until a workload reaches a given state.
- Config file or environment defaults for the state directory and pipe.

## [0.1.0]

Initial release: the `tpt` developer CLI.

### Added

- **The `tpt` binary** (`main`): a pure client of the local runtime API,
  holding no runtime state of its own.
- **Daemon control**: `tpt daemon start` and `tpt daemon stop`, plus
  `tpt status`. `start` re-invokes the binary with a hidden worker argument so
  the daemon outlives the invoking shell.
- **Running workloads**: `tpt run --windows <EXE> -- <ARGS>` for native
  executables, `tpt run --wasm <MODULE>` for WASM modules, and
  `tpt run --manifest <FILE>` for `tpt.runtime/v1` manifests.
- **Flag-generated manifests**: `tpt run` builds a manifest from its flags, so
  an ad-hoc run and a committed manifest travel the same code path. Resource
  overrides (`--cpu`, `--memory`, `--timeout`) are applied on top.
- **Argument separation**: everything after `--` goes to the workload's own
  command line, keeping runtime options unambiguous.
- **Inspection**: `tpt list`, `tpt inspect <id>`, `tpt logs <id>` with
  `--stderr` and `--tail N`, and `tpt events` with `--limit` and `--follow`.
  `inspect` and `logs` accept a workload id or name.
- **Lifecycle control**: `tpt stop`, `tpt restart` and `tpt destroy`.
- **Volumes**: `tpt volume create|list|remove`.
- **Devices**: `tpt devices`, listing what the runtime discovered.
- **Secrets**: `tpt secret set|list|delete`, with the value read from stdin so
  it never reaches shell history or process arguments, and listings showing
  names only.
- **Typed error reporting**: daemon errors surface with their `ErrorKind`, and
  an unreachable daemon produces an actionable message instead of a hang.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
