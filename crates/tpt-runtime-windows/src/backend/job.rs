//! Job Object wrapper: the Windows mechanism for workload isolation,
//! kill-tree semantics and resource accounting (SPEC §12, §25).
//!
//! All unsafe FFI is confined to this module; the rest of the crate uses
//! the safe [`JobObject`] API.

use std::sync::Arc;
use std::time::Duration;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAndIoAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_LIMIT_PROCESS_MEMORY,
};

/// One resource-usage snapshot from the job.
#[derive(Clone, Copy, Debug, Default)]
pub struct JobUsage {
    /// User-mode CPU time.
    pub user_cpu: Duration,
    /// Kernel-mode CPU time.
    pub kernel_cpu: Duration,
    /// Peak commit charge of the job's processes, bytes.
    pub peak_memory_bytes: u64,
    /// Bytes read.
    pub read_bytes: u64,
    /// Bytes written.
    pub write_bytes: u64,
    /// Active processes in the job.
    pub process_count: u32,
}

/// A live Job Object handle. Kill-on-close is always set: when the last
/// handle drops (daemon exit, instance drop), every remaining process in
/// the job is terminated — a crashed daemon cannot leak workloads.
pub struct JobObject {
    handle: HANDLE,
}

// HANDLE is a raw pointer, but a job handle is shareable across threads.
unsafe impl Send for JobObject {}
unsafe impl Sync for JobObject {}

impl JobObject {
    /// Creates a job with an optional per-process memory limit in bytes.
    pub fn with_memory_limit(process_memory: Option<u64>) -> Result<Arc<Self>> {
        // SAFETY: both parameters are optional pointers; null means defaults.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(
                RuntimeError::new(ErrorKind::System, "CreateJobObjectW failed")
                    .with_backend("windows"),
            );
        }

        // SAFETY: zeroed POD struct is the documented initialization for
        // JOBOBJECT_EXTENDED_LIMIT_INFORMATION before filling fields.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        let mut flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if let Some(limit) = process_memory {
            info.ProcessMemoryLimit = limit as usize;
            flags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        }
        info.BasicLimitInformation.LimitFlags = flags;

        // SAFETY: `handle` is valid (just created), the class constant and
        // struct size match the JOBOBJECT_EXTENDED_LIMIT_INFORMATION type.
        let ok = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            // SAFETY: closing the handle we created; no further use.
            unsafe { CloseHandle(handle) };
            return Err(
                RuntimeError::new(ErrorKind::System, "SetInformationJobObject failed")
                    .with_backend("windows"),
            );
        }

        Ok(Arc::new(Self { handle }))
    }

    /// Adds a process (and its future children) to the job.
    pub fn assign_process(&self, raw_handle: *mut core::ffi::c_void) -> Result<()> {
        // SAFETY: the caller passes a live process handle from a spawned
        // child; the job handle is owned and valid.
        let ok = unsafe { AssignProcessToJobObject(self.handle, raw_handle) };
        if ok == 0 {
            return Err(
                RuntimeError::new(ErrorKind::System, "AssignProcessToJobObject failed")
                    .with_backend("windows"),
            );
        }
        Ok(())
    }

    /// Terminates every process in the job with the given exit code.
    pub fn terminate(&self, exit_code: u32) -> Result<()> {
        // SAFETY: job handle is owned and valid.
        let ok = unsafe { TerminateJobObject(self.handle, exit_code) };
        if ok == 0 {
            return Err(
                RuntimeError::new(ErrorKind::System, "TerminateJobObject failed")
                    .with_backend("windows"),
            );
        }
        Ok(())
    }

    /// Queries CPU, memory and I/O accounting; `None` when the query fails
    /// (e.g. after termination) — callers keep the previous snapshot.
    pub fn query_usage(&self) -> Option<JobUsage> {
        // SAFETY: valid handle, matching class constant, buffer and size for
        // JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION.
        let mut info: JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            QueryInformationJobObject(
                self.handle,
                JobObjectBasicAndIoAccountingInformation,
                &mut info as *mut _ as *mut core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return None;
        }

        // SAFETY: valid handle, matching class constant and buffer size for
        // JOBOBJECT_EXTENDED_LIMIT_INFORMATION; only peak fields are read.
        let mut extended: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        let peak_memory_bytes = unsafe {
            if QueryInformationJobObject(
                self.handle,
                JobObjectExtendedLimitInformation,
                &mut extended as *mut _ as *mut core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                std::ptr::null_mut(),
            ) != 0
            {
                extended.PeakProcessMemoryUsed as u64
            } else {
                0
            }
        };

        Some(JobUsage {
            user_cpu: Duration::from_100ns(info.BasicInfo.TotalUserTime as u64),
            kernel_cpu: Duration::from_100ns(info.BasicInfo.TotalKernelTime as u64),
            peak_memory_bytes,
            read_bytes: info.IoInfo.ReadTransferCount as u64,
            write_bytes: info.IoInfo.WriteTransferCount as u64,
            process_count: info.BasicInfo.ActiveProcesses,
        })
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        // SAFETY: last owned handle; kill-on-close terminates stragglers.
        unsafe { CloseHandle(self.handle) };
    }
}

trait From100ns {
    fn from_100ns(hundreds: u64) -> Duration;
}

impl From100ns for Duration {
    fn from_100ns(hundreds: u64) -> Duration {
        Duration::from_nanos(hundreds * 100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_terminates_job() {
        let job = JobObject::with_memory_limit(Some(64 * 1024 * 1024)).unwrap();
        // no processes assigned: usage is zeroed but queryable
        let usage = job.query_usage();
        assert!(usage.is_some());
        assert_eq!(usage.unwrap().process_count, 0);
        // terminating an empty job succeeds
        job.terminate(0).unwrap();
    }

    #[test]
    fn tracks_assigned_process() {
        use std::os::windows::io::AsRawHandle;
        use std::process::{Command, Stdio};

        let job = JobObject::with_memory_limit(None).unwrap();
        let mut child = Command::new("ping.exe")
            .args(["-n", "3", "127.0.0.1"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        job.assign_process(child.as_raw_handle()).unwrap();

        std::thread::sleep(Duration::from_millis(200));
        let usage = job.query_usage().unwrap();
        assert_eq!(usage.process_count, 1);

        job.terminate(1).unwrap();
        let _ = child.wait();
        std::thread::sleep(Duration::from_millis(100));
        let usage = job.query_usage().unwrap();
        assert_eq!(usage.process_count, 0);
    }

    /// SPEC §45/§46 failure + isolation: when the runtime loses its handle
    /// (daemon crash, instance drop), KILL_ON_JOB_CLOSE must reap the
    /// workload without an explicit stop.
    #[test]
    fn kill_on_close_reaps_when_last_handle_drops() {
        use std::os::windows::io::AsRawHandle;
        use std::process::{Command, Stdio};

        let job = JobObject::with_memory_limit(None).unwrap();
        let mut child = Command::new("ping.exe")
            .args(["-n", "60", "127.0.0.1"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        job.assign_process(child.as_raw_handle()).unwrap();

        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(job.query_usage().unwrap().process_count, 1);

        drop(job);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "workload process outlived its job object"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        // ping -n 60 cannot finish in 10s: the death above is the
        // kill-on-close. (Windows reports exit code 0 for this path, so
        // the code itself is not asserted.)
    }

    /// SPEC §45 resource exhaustion at the OS level: a per-process commit
    /// ceiling makes an allocating workload die while an unlimited control
    /// workload keeps running.
    #[test]
    fn process_memory_limit_starves_allocating_workload() {
        use std::os::windows::io::AsRawHandle;
        use std::process::{Command, Stdio};

        let spawn_under = |job: &JobObject| {
            let child = Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-Command",
                    "$b = New-Object byte[] 67108864; Start-Sleep -Seconds 60",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            job.assign_process(child.as_raw_handle()).unwrap();
            child
        };

        let limited = JobObject::with_memory_limit(Some(32 * 1024 * 1024)).unwrap();
        let mut starved = spawn_under(&limited);
        let generous = JobObject::with_memory_limit(None).unwrap();
        let mut control = spawn_under(&generous);

        // The control workload (same command, no commit ceiling) survives.
        std::thread::sleep(Duration::from_secs(5));
        assert!(
            control.try_wait().unwrap().is_none(),
            "unlimited control workload died unexpectedly"
        );

        // The 32 MiB ceiling kills the 64 MiB allocator quickly.
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        let status = loop {
            if let Some(status) = starved.try_wait().unwrap() {
                break status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "process memory limit was not enforced"
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        assert_ne!(status.code(), Some(0), "OOM death is not a clean exit");

        limited.terminate(1).unwrap();
        generous.terminate(1).unwrap();
        let _ = starved.wait();
        let _ = control.wait();
    }
}
