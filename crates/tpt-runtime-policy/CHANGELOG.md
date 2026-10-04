# Changelog

All notable changes to `tpt-runtime-policy` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- Track currently allocated capacity so `Reservation` can genuinely reserve
  rather than behaving as a hard limit.
- Implement `Priority` reclaim ordering between workloads.
- Report per-class (`ResourceClass`) decisions so CPU, memory and GPU outcomes
  can be reported independently.

### Added

- `HostCapacity::discover(gpus)`: real host capacity for admission -
  CPU cores from the scheduler, RAM from the OS (`GlobalMemoryStatusEx`
  on Windows, /proc/meminfo elsewhere), GPU count as given. Discovery
  problems degrade to unbounded, never to failure; the daemon now admits
  against discovered capacity instead of assuming infinity.

## [0.1.0]

Initial release: pure admission control.

### Added

- **`PolicyEngine`** (`policy`): a pure engine taking host capacity plus a
  workload specification and returning a decision, with `new`,
  `with_default_policy`, `admit` and `enforce`. Applying a decision is left to
  the backends.
- **`ResourcePolicy`**: `HardLimit` (the default, denying unsatisfiable
  requests), `SoftLimit` (clamp and admit with an explanation), `Reservation`
  (currently a hard limit, since capacity is not yet reclaimable), `Priority`
  (advisory in the MVP) and `BestEffort`.
- **`HostCapacity`** (`policy`): CPU cores, memory and GPU count, plus
  `unknown()` reporting effectively unlimited CPU and memory and zero GPUs for
  use when discovery is unavailable.
- **`PolicyDecision`** (`policy`): `Admitted`, `Adjusted(Vec<String>)` carrying
  explanations, and `Denied(String)` carrying the reason, with `is_admitted`.
- **`ResourceClass`** (`policy`): `Cpu`, `Memory`, `Storage`, `Network`, `Gpu`
  and `ProcessCount`.
- Explicit failure over silent degradation (SPEC §48): denials name both the
  request and the available capacity, and a workload requesting an absent
  device is denied at admission.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
