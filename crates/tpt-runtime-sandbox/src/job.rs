//! Job Object wrapper for sandboxed workloads (SPEC §13, §25).
//!
//! Two mechanisms are layered on top of what `tpt-runtime-windows` already
//! does for native processes:
//!
//! - `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` — when the last handle drops
//!   (daemon exit, instance drop) every remaining process in the job is
//!   terminated, so a crashed daemon cannot leak workloads.
//! - `JOB_OBJECT_LIMIT_ACTIVE_PROCESS` — a hard cap on how many processes the
//!   image may ever have alive, bounding fork bombs.
//!
//! All unsafe FFI is confined to this module; the rest of the crate uses the
//! safe [`JobObject`] API.

use std::sync::Arc;
use std::time::Duration;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_FAILED};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAndIoAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, ResumeThread, WaitForSingleObject,
};

/// Default cap on processes per job when the manifest requests none. A single
/// image is not expected to fork more than this; the limit exists to bound
/// runaway trees, not to be a tuning knob.
pub const DEFAULT_MAX_PROCESSES: u32 = 256;

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

/// A live Job Object handle. Kill-on-close is always set.
pub struct JobObject {
    handle: HANDLE,
}

// HANDLE is a raw pointer, but a job handle is shareable across threads.
unsafe impl Send for JobObject {}
unsafe impl Sync for JobObject {}

impl JobObject {
    /// Creates a job with an optional per-process memory limit in bytes and an
    /// optional cap on concurrent processes.
    pub fn with_limits(
        process_memory: Option<u64>,
        max_processes: Option<u32>,
    ) -> Result<Arc<Self>> {
        // SAFETY: both parameters are optional pointers; null means defaults.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(
                RuntimeError::new(ErrorKind::System, "CreateJobObjectW failed")
                    .with_backend("sandbox"),
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
        let max_processes = max_processes.unwrap_or(DEFAULT_MAX_PROCESSES).max(1);
        info.BasicLimitInformation.ActiveProcessLimit = max_processes;
        flags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
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
                    .with_backend("sandbox"),
            );
        }

        Ok(Arc::new(Self { handle }))
    }

    /// Adds a process (and its future children) to the job.
    ///
    /// `raw_handle` must be a live process handle the caller owns. It is
    /// passed as a bare pointer rather than a typed wrapper because the
    /// `CreateProcess*` family hands back raw `HANDLE`s; this module is the
    /// only place that dereferences them.
    ///
    /// This must run while the child is still suspended: once a process is
    /// running it may already have spawned children that escape the job.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn assign_process(&self, raw_handle: *mut core::ffi::c_void) -> Result<()> {
        // SAFETY: the caller passes a live process handle from a suspended
        // child; the job handle is owned and valid.
        let ok = unsafe { AssignProcessToJobObject(self.handle, raw_handle) };
        if ok == 0 {
            return Err(
                RuntimeError::new(ErrorKind::System, "AssignProcessToJobObject failed")
                    .with_backend("sandbox"),
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
                    .with_backend("sandbox"),
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
        // JOBOBJECT_EXTENDED_LIMIT_INFORMATION; only the peak field is read.
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
            user_cpu: Duration::from_nanos(info.BasicInfo.TotalUserTime as u64 * 100),
            kernel_cpu: Duration::from_nanos(info.BasicInfo.TotalKernelTime as u64 * 100),
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

/// RAII wrapper for the process and thread handles the spawn path returns.
///
/// Both handles are closed on every path; leaking a thread handle per
/// workload would exhaust the daemon's handle budget over time.
pub struct ProcessHandles {
    /// Process handle.
    pub process: HANDLE,
    /// Primary thread handle.
    pub thread: HANDLE,
}

// HANDLE is a raw pointer; process and thread handles are owned by this
// struct, which closes each exactly once, so moving one across threads is
// sound.
unsafe impl Send for ProcessHandles {}
unsafe impl Sync for ProcessHandles {}

impl ProcessHandles {
    /// Waits for the process to exit and returns its exit code.
    pub fn wait(&self) -> Result<u32> {
        // SAFETY: valid process handle; INFINITE blocks until it exits.
        let wait = unsafe { WaitForSingleObject(self.process, u32::MAX) };
        if wait == WAIT_FAILED {
            return Err(
                RuntimeError::new(ErrorKind::System, "WaitForSingleObject failed")
                    .with_backend("sandbox"),
            );
        }
        self.exit_code()
    }

    /// Reads the exit code without waiting.
    pub fn exit_code(&self) -> Result<u32> {
        let mut code: u32 = 0;
        // SAFETY: valid process handle and a correctly sized out-parameter.
        let ok = unsafe { GetExitCodeProcess(self.process, &mut code) };
        if ok == 0 {
            return Err(
                RuntimeError::new(ErrorKind::System, "GetExitCodeProcess failed")
                    .with_backend("sandbox"),
            );
        }
        Ok(code)
    }

    /// Resumes the primary thread after the process has been job-assigned.
    pub fn resume(&self) -> Result<()> {
        // SAFETY: valid thread handle; u32::MAX is the failure sentinel.
        let previous = unsafe { ResumeThread(self.thread) };
        if previous == u32::MAX {
            return Err(
                RuntimeError::new(ErrorKind::System, "ResumeThread failed").with_backend("sandbox")
            );
        }
        Ok(())
    }
}

impl Drop for ProcessHandles {
    fn drop(&mut self) {
        // SAFETY: these are the handles we own; each is closed exactly once.
        unsafe {
            if !self.process.is_null() {
                CloseHandle(self.process);
            }
            if !self.thread.is_null() {
                CloseHandle(self.thread);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_job_with_limits_and_terminates_empty() {
        let job = JobObject::with_limits(Some(64 * 1024 * 1024), Some(4)).unwrap();
        let usage = job.query_usage().expect("usage queryable");
        assert_eq!(usage.process_count, 0);
        // terminating an empty job succeeds
        job.terminate(0).unwrap();
    }

    #[test]
    fn zero_process_limit_is_clamped_to_one() {
        // A zero ActiveProcessLimit would make every spawn fail; the crate
        // clamps rather than handing an unusable job to the caller.
        let job = JobObject::with_limits(None, Some(0)).unwrap();
        assert!(job.query_usage().is_some());
    }

    #[test]
    fn dropping_a_job_does_not_panic() {
        let job = JobObject::with_limits(None, None).unwrap();
        drop(job);
    }
}
