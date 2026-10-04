# Changelog

All notable changes to `tpt-runtime-sandbox` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **The runtime's own Windows OCI isolation provider** (SPEC §13, §17),
  replacing the wait on `tpt-boxcar`. Boxcar's `origin` sandbox documents
  `type: oci` as bookkeeping-only with no containerd integration and has no
  Windows isolation, so `tpt-runtime-oci::start` could never be unblocked by
  it.
- **`SandboxProvider`** (`provider`): the isolation seam, with
  **`SandboxLimits`** carrying manifest-resolved resource ceilings so no
  provider has to re-read policy. `WindowsSandbox` implements it, and also
  implements `tpt_runtime_oci::IsolationProvider` so the OCI backend can hold
  it without depending on this crate.
- **`JobObject::with_limits`** (`job`): job objects with kill-on-close plus an
  `ActiveProcessLimit` cap on concurrent processes, bounding runaway process
  trees. `ProcessHandles` is the RAII wrapper for the process and thread
  handles, closing both on every path including success.
- **`RestrictedToken`** (`token`): `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE)`
  derived from the daemon's token, with `SeAssignPrimaryTokenPrivilege` and
  `SeIncreaseQuotaPrivilege` enabled as `CreateProcessAsUserW` requires.
  Failure to enable them is reported, never worked around by spawning an
  unisolated process.
- **`spawn_suspended`** (`win32`): the full spawn path — two inheritable log
  pipes, a restricted token, a `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` naming only
  those two handles, `CREATE_SUSPENDED`, job assignment *while still
  suspended*, then `ResumeThread`. Assigning after the resume would let a
  forked grandchild escape the job.
- **Bundle resolution and environment** (`bundle_env`): rootfs-scoped entry
  point resolution with `.exe` probing for extensionless names, an
  executable-format sniffer, and a sanitized environment that never inherits
  the daemon's.
- **End-to-end test suite** (`tests/isolation.rs`): spawns a real copy of the
  host shell through the whole path and asserts on exit codes, captured
  stdout, stop-vs-crash attribution, double `exit` calls, command-line
  quoting of spaced arguments, and environment containment.
- **Mount handling** (`mounts`): granted volumes are validated (the host path
  must exist and be backed by `filesystem.read`, plus `filesystem.write` for a
  read-write mount) and passed to the workload as `TPT_VOLUME_<NAME>`
  environment variables holding the host path, matching the
  `tpt-runtime-linux` convention. A read-write mount missing its write grant is
  refused rather than silently downgraded to read-only.
- **Network handling** (`network`): validates that exposed ports were only
  granted to a mode that accepts inbound traffic, and that an outbound-capable
  mode carries the `network.outbound` grant. The granted mode is passed as
  `TPT_NETWORK_MODE`.

### Fixed

- Command-line quoting doubles a run of trailing backslashes before the
  closing quote; a lone backslash would otherwise escape that quote and
  swallow the rest of the command line.

### Known limitations

- **Linux images are refused on Windows hosts.** `tpt-runtime-oci` resolves
  `linux/amd64` by default, and Windows has no ELF loader. The error names a
  Windows-based image as the fix rather than failing obscurely at spawn time.
- **Process-level isolation only.** Windows has no `chroot`, so the rootfs is
  a source tree for the entry point, not a filesystem jail; an image can still
  read the host filesystem. Granted mounts are validated and passed through the
  environment, but a `read-only` mount is a declaration rather than an enforced
  restriction. Namespace confinement needs Hyper-V or WSL.
- **Network intent is advisory, not enforced.** Per-workload networking needs
  HNS or per-app firewall rules, which this provider does not manage. `tpt
  inspect` reports `"enforced": false` on both mounts and network so the
  posture is not overstated.
- **The daemon must run elevated**, because `CreateProcessAsUserW` needs
  `SeAssignPrimaryTokenPrivilege` and `SeIncreaseQuotaPrivilege`. The
  end-to-end tests skip with an explanatory message when not elevated.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD