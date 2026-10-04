//! End-to-end tests for the Windows isolation provider.
//!
//! These spawn real processes through the full sandbox path — restricted
//! token, job object, handle inheritance list, suspended spawn — rather than
//! stubbing the platform layer, because the bugs that matter here (job
//! assignment racing the resume, lost stdout, handles leaking into the child)
//! only appear when a real image actually runs.
//!
//! Every test copies the host `cmd.exe` into a throwaway rootfs, so no
//! network access and no registry pulls are involved.

#![cfg(windows)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use tpt_runtime_capability::CapabilitySet;
use tpt_runtime_core::error::ErrorKind;
use tpt_runtime_core::id::WorkloadId;
use tpt_runtime_model::network::NetworkMode;
use tpt_runtime_model::volume::VolumeAccessMode;
use tpt_runtime_oci::Bundle;
use tpt_runtime_process::{ResolvedMount, StartContext, StopMode};
use tpt_runtime_sandbox::{SandboxLimits, SandboxProvider, WindowsSandbox};

/// A bundle whose rootfs holds a copy of the host shell.
fn real_bundle(tag: &str, args: &[&str]) -> (PathBuf, Bundle) {
    let base = std::env::temp_dir().join(format!("tpt-sandbox-e2e-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let rootfs = base.join("rootfs");
    std::fs::create_dir_all(&rootfs).expect("rootfs");
    std::fs::copy(
        std::env::var("COMSPEC").unwrap_or_else(|_| "C:\\Windows\\System32\\cmd.exe".to_owned()),
        rootfs.join("host.exe"),
    )
    .expect("copying the host shell into the rootfs");

    let bundle = Bundle {
        id: format!("e2e-{tag}"),
        path: base.clone(),
        args: std::iter::once("/host.exe".to_owned())
            .chain(args.iter().map(|a| (*a).to_owned()))
            .collect(),
        env: BTreeMap::new(),
        working_dir: None,
    };
    (base, bundle)
}

fn context(log_dir: &Path) -> StartContext {
    StartContext {
        workload_id: WorkloadId::generate(),
        mounts: vec![],
        log_dir: log_dir.to_path_buf(),
        capabilities: tpt_runtime_capability::CapabilitySet::empty(),
        network_mode: tpt_runtime_model::network::NetworkMode::None,
        exposed_ports: vec![],
    }
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

fn sandbox() -> WindowsSandbox {
    WindowsSandbox::new()
}

/// Whether this host can run the sandboxed path at all.
///
/// `CreateProcessAsUserW` requires `SeAssignPrimaryTokenPrivilege` and
/// `SeIncreaseQuotaPrivilege`. A non-administrator `cargo test` does not hold
/// them, so the end-to-end tests below cannot spawn there. Skipping is the
/// right behaviour: the sandbox deliberately refuses to start an
/// *unisolated* process as a fallback, and a test failure would read as a
/// code defect when it is a deployment requirement (run the daemon elevated).
///
/// Run `cargo test` from an elevated shell to exercise these.
fn elevated_host() -> bool {
    use tpt_runtime_sandbox::RestrictedToken;
    match RestrictedToken::from_current_process() {
        Ok(_) => true,
        Err(err) if err.operation.as_deref() == Some("enable_privilege") => {
            eprintln!("skipping sandbox e2e test: {}", err.message);
            false
        }
        Err(err) => panic!("unexpected token failure: {err}"),
    }
}
#[test]
fn granted_mounts_reach_the_workload_environment() {
    if !elevated_host() {
        return;
    }
    let (base, bundle) = real_bundle("mounts", &["/C", "echo", "%TPT_VOLUME_PROJ%"]);
    let logs = base.join("logs");
    // The mount must be granted with the capabilities that back it, and the
    // host path must exist — validate_mounts checks both before any spawn.
    let mut ctx = context(&logs);
    ctx.mounts = vec![ResolvedMount {
        name: "proj".to_owned(),
        mount: "/data".to_owned(),
        host_path: base.clone(),
        mode: VolumeAccessMode::ReadWrite,
    }];
    ctx.capabilities = CapabilitySet::from_names(["filesystem.read", "filesystem.write"]);

    let instance = sandbox()
        .start_bundle(&bundle, &ctx, SandboxLimits::default())
        .expect("spawn should succeed");

    // `inspect` must show the mount, and must not claim it is enforced.
    let described = instance.describe();
    assert_eq!(described["mounts"][0]["name"], "proj");
    assert_eq!(described["mounts"][0]["mode"], "read-write");
    assert_eq!(described["mounts"][0]["enforced"], false);
    // Network intent is reported too, with its unenforced status stated.
    assert_eq!(described["network"]["mode"], "none");
    assert_eq!(described["network"]["enforced"], false);

    block_on(instance.exit()).expect("exit resolves");
    let log = logs.join("stdout.log");
    for _ in 0..100 {
        if std::fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("TPT_SANDBOX")
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let contents = std::fs::read_to_string(&log).unwrap_or_default();
    // Echoing %TPT_VOLUME_PROJ% on an undefined variable leaves the literal,
    // so a real path here proves the variable crossed into the child.
    assert!(
        !contents.contains("%TPT_VOLUME_PROJ%"),
        "the mount variable did not reach the workload: {contents:?}"
    );
    assert!(
        contents.contains("tpt-sandbox-e2e-mounts"),
        "the mount path was not passed through: {contents:?}"
    );
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn a_mount_without_its_capability_is_refused_before_spawning() {
    if !elevated_host() {
        return;
    }
    let (base, bundle) = real_bundle("nocap", &["/C", "echo", "should not run"]);
    let mut ctx = context(&base.join("logs"));
    ctx.mounts = vec![ResolvedMount {
        name: "proj".to_owned(),
        mount: "/data".to_owned(),
        host_path: base.clone(),
        mode: VolumeAccessMode::ReadWrite,
    }];
    // A read-write mount promised by the manifest with only a read grant
    // must fail loudly, not be silently downgraded.
    ctx.capabilities = CapabilitySet::from_names(["filesystem.read"]);

    let err = match sandbox().start_bundle(&bundle, &ctx, SandboxLimits::default()) {
        Err(err) => err,
        Ok(_) => panic!("a read-write mount without filesystem.write must be refused"),
    };
    assert!(err.message.contains("filesystem.write"), "{err}");
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn ports_granted_to_a_denied_network_mode_are_refused() {
    if !elevated_host() {
        return;
    }
    let (base, bundle) = real_bundle("netbad", &["/C", "echo", "should not run"]);
    let mut ctx = context(&base.join("logs"));
    // The manager would not hand out ports for `none`; if one appears anyway,
    // the sandbox refuses rather than exposing it silently.
    ctx.network_mode = NetworkMode::None;
    ctx.exposed_ports = vec![("http".to_owned(), 8080)];

    let err = match sandbox().start_bundle(&bundle, &ctx, SandboxLimits::default()) {
        Err(err) => err,
        Ok(_) => panic!("exposed ports with a deny-all network mode must be refused"),
    };
    assert_eq!(err.kind, ErrorKind::NetworkFailure);
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn runs_a_real_image_and_captures_stdout() {
    if !elevated_host() {
        return;
    }
    let (base, bundle) = real_bundle("run", &["/C", "echo", "sandboxed"]);
    let logs = base.join("logs");
    let instance = sandbox()
        .start_bundle(&bundle, &context(&logs), SandboxLimits::default())
        .expect("a PE image should start");

    let described = instance.describe();
    assert_eq!(described["backend"], "sandbox");
    assert_eq!(
        described["isolation"]["restricted_token"].as_bool(),
        Some(true)
    );
    assert_eq!(described["isolation"]["job_object"].as_bool(), Some(true));
    assert!(described["pid"].as_u64().expect("pid") > 0);

    let status = block_on(instance.exit()).expect("the receiver always completes");
    assert_eq!(status.code, Some(0), "echo should exit cleanly");
    assert!(!status.failed);
    assert!(!status.killed);

    // The capture thread appends to the log file; poll briefly for it.
    let log = logs.join("stdout.log");
    for _ in 0..100 {
        if std::fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("sandboxed")
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let contents = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        contents.contains("sandboxed"),
        "stdout not captured: {contents:?}"
    );
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn a_nonzero_exit_is_reported_as_failed() {
    if !elevated_host() {
        return;
    }
    let (base, bundle) = real_bundle("fail", &["/C", "exit", "3"]);
    let instance = sandbox()
        .start_bundle(
            &bundle,
            &context(&base.join("logs")),
            SandboxLimits::default(),
        )
        .expect("spawn should succeed");
    let status = block_on(instance.exit()).expect("the receiver always completes");
    assert_eq!(status.code, Some(3));
    assert!(status.failed, "a non-zero exit must be a failure");
    assert!(!status.killed);
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn stop_terminates_the_job_and_marks_the_workload_killed() {
    if !elevated_host() {
        return;
    }
    let (base, bundle) = real_bundle("stop", &["/C", "pause"]);
    let instance = sandbox()
        .start_bundle(
            &bundle,
            &context(&base.join("logs")),
            SandboxLimits::default(),
        )
        .expect("spawn should succeed");

    instance
        .stop(StopMode::Kill)
        .expect("stop terminates the job");
    let status = block_on(instance.exit()).expect("the receiver always completes");
    assert!(status.killed, "a runtime stop must be attributed as killed");
    // A requested termination is not a crash.
    assert!(!status.failed);
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn a_second_exit_request_still_resolves() {
    if !elevated_host() {
        return;
    }
    // The trait contract is that `exit` always yields a status, even when
    // called twice; the second call must not hang.
    let (base, bundle) = real_bundle("double", &["/C", "echo", "once"]);
    let instance = sandbox()
        .start_bundle(
            &bundle,
            &context(&base.join("logs")),
            SandboxLimits::default(),
        )
        .expect("spawn should succeed");
    let _first = block_on(instance.exit()).expect("first exit resolves");
    let second = block_on(instance.exit()).expect("second exit resolves");
    assert!(second.killed || second.code.is_some());
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn linux_bundles_are_refused_before_spawning() {
    let base = std::env::temp_dir().join(format!("tpt-sandbox-e2e-elf-{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let rootfs = base.join("rootfs");
    std::fs::create_dir_all(rootfs.join("bin")).expect("rootfs");
    std::fs::write(rootfs.join("bin").join("sh"), b"\x7fELF\x02\x01\x01\x00").unwrap();

    let bundle = Bundle {
        id: "linux-image".to_owned(),
        path: base.clone(),
        args: vec!["/bin/sh".to_owned()],
        env: BTreeMap::new(),
        working_dir: None,
    };
    let ctx = context(&base.join("logs"));
    let err = match sandbox().start_bundle(&bundle, &ctx, SandboxLimits::default()) {
        Err(err) => err,
        Ok(_) => panic!("a Linux rootfs must not be spawned on a Windows host"),
    };
    assert_eq!(err.kind, ErrorKind::BackendUnavailable);
    assert!(
        err.message.contains("Windows-based image"),
        "the error must name the fix: {err}"
    );
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn a_missing_entry_point_never_reaches_the_spawn() {
    let base = std::env::temp_dir().join(format!("tpt-sandbox-e2e-miss-{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    std::fs::create_dir_all(base.join("rootfs")).expect("rootfs");
    let bundle = Bundle {
        id: "missing".to_owned(),
        path: base.clone(),
        args: vec!["/nope.exe".to_owned()],
        env: BTreeMap::new(),
        working_dir: None,
    };
    let ctx = context(&base.join("logs"));
    let err = match sandbox().start_bundle(&bundle, &ctx, SandboxLimits::default()) {
        Err(err) => err,
        Ok(_) => panic!("a missing entry point must fail before spawning"),
    };
    assert_eq!(err.kind, ErrorKind::NotFound);
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn arguments_with_spaces_reach_the_process_intact() {
    if !elevated_host() {
        return;
    }
    // Command-line quoting is easy to get wrong; verify a spaced argument
    // arrives as a single argument by echoing it back.
    let (base, bundle) = real_bundle("args", &["/C", "echo", "two words"]);
    let logs = base.join("logs");
    let instance = sandbox()
        .start_bundle(&bundle, &context(&logs), SandboxLimits::default())
        .expect("spawn should succeed");
    let status = block_on(instance.exit()).expect("exit resolves");
    assert_eq!(status.code, Some(0));

    let log = logs.join("stdout.log");
    for _ in 0..100 {
        if std::fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("two words")
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let contents = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        contents.contains("two words"),
        "the spaced argument was mangled: {contents:?}"
    );
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn the_daemon_environment_does_not_leak_into_the_workload() {
    if !elevated_host() {
        return;
    }
    // Set a sentinel in this process; it must not appear in the child.
    std::env::set_var("TPT_SANDBOX_SENTINEL", "leaked");
    let (base, bundle) = real_bundle("env", &["/C", "echo", "%TPT_SANDBOX_SENTINEL%"]);
    let logs = base.join("logs");
    let instance = sandbox()
        .start_bundle(&bundle, &context(&logs), SandboxLimits::default())
        .expect("spawn should succeed");
    block_on(instance.exit()).expect("exit resolves");
    std::env::remove_var("TPT_SANDBOX_SENTINEL");

    let log = logs.join("stdout.log");
    for _ in 0..100 {
        if std::fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("SENTINEL")
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let contents = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !contents.contains("leaked"),
        "the daemon environment leaked into the workload: {contents:?}"
    );
    std::fs::remove_dir_all(&base).ok();
}
