//! The `CreateProcessW` path: pipes, restricted token, job assignment, resume
//! (SPEC §13, §23).
//!
//! All unsafe FFI lives here. The sequence is deliberate:
//!
//! 1. create two inheritable pipes for stdout/stderr;
//! 2. build a restricted token;
//! 3. describe *only* those two write handles in a
//!    `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, so nothing else the daemon holds is
//!    reachable from the workload even if it is marked inheritable;
//! 4. `CreateProcessW` with `CREATE_SUSPENDED` — the child does not run yet;
//! 5. assign the (still suspended) child to the job object, closing the race
//!    where a forked grandchild escapes before assignment;
//! 6. `ResumeThread`.
//!
//! Skipping step 5 and resuming early is the classic bug here: a process that
//! runs before it is in a job can already have children outside it.
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::path::Path;

use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use windows_sys::Win32::Foundation::{
    CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
    UpdateProcThreadAttribute, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use crate::job::{JobObject, ProcessHandles};
use crate::token::RestrictedToken;

/// A started sandboxed process plus the pipes its output arrives on.
pub struct StartedProcess {
    /// Process and thread handles, closed when dropped.
    pub handles: ProcessHandles,
    /// Process id.
    pub pid: u32,
    /// The read end of the child's stdout pipe.
    ///
    /// A `File` rather than `ChildStdout`: the pipe is created here by
    /// `CreatePipe`, not by `std::process`, so it is wrapped from the raw
    /// handle. `File` implements `Read`, which is all the log capture needs.
    pub stdout: std::fs::File,
    /// The read end of the child's stderr pipe.
    pub stderr: std::fs::File,
}

/// Owns an inheritable-pipe pair.
struct PipePair {
    read: HANDLE,
    write: HANDLE,
}

impl Drop for PipePair {
    fn drop(&mut self) {
        // SAFETY: we own both ends; the reader is closed explicitly by the
        // caller only after it has been moved into a ChildStdout.
        unsafe {
            if !self.read.is_null() {
                CloseHandle(self.read);
            }
            if !self.write.is_null() {
                CloseHandle(self.write);
            }
        }
    }
}

/// Creates a pipe whose write end is inheritable and whose read end is not.
fn inheritable_pipe() -> Result<PipePair> {
    let mut read: HANDLE = std::ptr::null_mut();
    let mut write: HANDLE = std::ptr::null_mut();

    // SECURITY_ATTRIBUTES with bInheritHandle = TRUE marks the write end
    // inheritable; the read end's inheritance is cleared below regardless.
    // SAFETY: zeroed struct then setting the documented fields.
    let mut attributes: SECURITY_ATTRIBUTES = unsafe { std::mem::zeroed() };
    attributes.bInheritHandle = 1;

    // SAFETY: valid out-parameters and attributes with the documented size.
    let ok = unsafe {
        CreatePipe(
            &mut read,
            &mut write,
            &attributes,
            0, // default buffer size
        )
    };
    if ok == 0 || read.is_null() || write.is_null() {
        return Err(
            RuntimeError::new(ErrorKind::System, "CreatePipe failed").with_backend("sandbox")
        );
    }

    // The read end must not leak into the child.
    // SAFETY: `read` is a valid handle we just created.
    unsafe { SetHandleInformation(read, HANDLE_FLAG_INHERIT, 0) };

    Ok(PipePair { read, write })
}

/// Quotes one argument for the Windows command line.
///
/// `CreateProcessW` parses a single command-line string, not an argv array.
/// Without quoting, an argument containing a space (or a quote) would split
/// into two arguments, so the workload would receive different arguments than
/// the manifest declared.
pub fn quote_argument(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '\n', '"']) {
        return argument.to_owned();
    }
    let mut quoted = String::with_capacity(argument.len() + 2);
    quoted.push('"');
    let mut backslashes = 0usize;
    for character in argument.chars() {
        match character {
            '\\' => {
                backslashes += 1;
            }
            '"' => {
                // Backslashes before a quote must be doubled so they are not
                // read as escapes; the closing quote needs its own escape.
                for _ in 0..backslashes {
                    quoted.push('\\');
                }
                backslashes = 0;
                quoted.push('\\');
                quoted.push('"');
            }
            _ => {
                for _ in 0..backslashes {
                    quoted.push('\\');
                }
                backslashes = 0;
                quoted.push(character);
            }
        }
    }
    // A non-empty run of trailing backslashes must be doubled before the
    // closing quote: a lone backslash would escape that quote and swallow
    // whatever follows on the command line. With no trailing backslashes the
    // run stays empty, so nothing extra is emitted.
    for _ in 0..backslashes.saturating_mul(2) {
        quoted.push('\\');
    }
    quoted.push('"');
    quoted
}

/// Renders the command line for `CreateProcessW` from the program and args.
pub fn build_command_line(program: &Path, args: &[String]) -> Vec<u16> {
    let mut parts: Vec<String> = vec![quote_argument(&program.to_string_lossy())];
    parts.extend(args.iter().map(|arg| quote_argument(arg)));
    let mut wide: Vec<u16> = Vec::new();
    for part in parts {
        for unit in part.encode_utf16() {
            wide.push(unit);
        }
        wide.push(b' ' as u16);
    }
    wide.push(0);
    wide
}

/// Frees an initialized attribute list and its allocation exactly once.
struct AttributeListGuard {
    pointer: *mut u8,
    layout: std::alloc::Layout,
}

impl Drop for AttributeListGuard {
    fn drop(&mut self) {
        if self.pointer.is_null() {
            return;
        }
        // SAFETY: the list was initialized by InitializeProcThreadAttributeList
        // before this guard was armed, and the allocation matches `layout`.
        unsafe {
            DeleteProcThreadAttributeList(self.pointer as _);
            std::alloc::dealloc(self.pointer, self.layout);
        }
    }
}

/// Starts `program` inside `job` under a restricted token, suspended, with
/// only the two log pipes inheritable.
#[allow(clippy::too_many_arguments)]
pub fn spawn_suspended(
    program: &Path,
    args: &[String],
    env_block: &[u16],
    working_dir: &Path,
    job: &JobObject,
) -> Result<StartedProcess> {
    let stdout_pipe = inheritable_pipe()?;
    let stderr_pipe = inheritable_pipe()?;
    let token = RestrictedToken::from_current_process()?;

    let mut command_line = build_command_line(program, args);
    // `encode_wide` is the Windows spelling of UTF-16 encoding; Windows is
    // UTF-16 natively, so the code units map straight onto WCHARs.
    let mut application_name: Vec<u16> = program.as_os_str().encode_wide().collect();
    application_name.push(0);
    let mut current_directory: Vec<u16> = working_dir.as_os_str().encode_wide().collect();
    current_directory.push(0);

    // Two-pass attribute list sizing: a null buffer returns the required size.
    let mut attribute_bytes: usize = 0;
    // SAFETY: null buffer asks for the size only; the count is what we need.
    let sized = unsafe {
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut attribute_bytes)
    };
    if sized != 0 && attribute_bytes == 0 {
        return Err(RuntimeError::new(
            ErrorKind::System,
            "InitializeProcThreadAttributeList failed while sizing",
        )
        .with_backend("sandbox"));
    }

    // Over-aligned allocation for the opaque attribute list.
    let layout = std::alloc::Layout::from_size_align(attribute_bytes.max(1), 8).map_err(|_| {
        RuntimeError::new(ErrorKind::System, "attribute list layout is invalid")
            .with_backend("sandbox")
    })?;
    let attribute_list = unsafe { std::alloc::alloc(layout) };
    if attribute_list.is_null() {
        return Err(RuntimeError::new(
            ErrorKind::System,
            "cannot allocate the process attribute list",
        )
        .with_backend("sandbox"));
    }
    // The list must be deleted and freed on every exit path from here on.
    let _guard = AttributeListGuard {
        pointer: attribute_list,
        layout,
    };

    // SAFETY: freshly allocated buffer of the size the sizing call reported,
    // and one attribute is expected, matching the count used above.
    let initialized = unsafe {
        InitializeProcThreadAttributeList(attribute_list as _, 1, 0, &mut attribute_bytes)
    };
    if initialized == 0 {
        return Err(RuntimeError::new(
            ErrorKind::System,
            "InitializeProcThreadAttributeList failed",
        )
        .with_backend("sandbox"));
    }

    // Restrict inheritance to exactly the two log pipe write ends.
    let handles: [HANDLE; 2] = [stdout_pipe.write, stderr_pipe.write];
    // SAFETY: the list was initialized for one attribute; the value is an
    // array of HANDLE of exactly the size declared.
    let updated = unsafe {
        UpdateProcThreadAttribute(
            attribute_list as _,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handles.as_ptr() as *const core::ffi::c_void,
            std::mem::size_of_val(&handles),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if updated == 0 {
        return Err(RuntimeError::new(
            ErrorKind::System,
            "UpdateProcThreadAttribute (handle list) failed",
        )
        .with_backend("sandbox"));
    }

    // SAFETY: zeroed STARTUPINFOEXW is the documented initialization; the
    // standard handles are the two pipe write ends, inheriting through the
    // handle list above.
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdOutput = stdout_pipe.write;
    startup.StartupInfo.hStdError = stderr_pipe.write;
    startup.lpAttributeList = attribute_list as _;

    // SAFETY: zeroed PROCESS_INFORMATION is the documented initialization.
    let mut information: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let creation_flags = EXTENDED_STARTUPINFO_PRESENT
        | CREATE_SUSPENDED
        | CREATE_UNICODE_ENVIRONMENT
        | CREATE_NO_WINDOW;

    // SAFETY: the token is open for TOKEN_ADJUST_PRIVILEGES; program name,
    // command line and directory are NUL-terminated; the attribute list is
    // populated; `information` is a valid out-parameter.
    // `bInheritHandles` is TRUE because the handle list — not
    // inheritance-by-default — is what restricts what crosses.
    let created = unsafe {
        CreateProcessAsUserW(
            token.handle(),
            application_name.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            creation_flags,
            env_block.as_ptr() as *const core::ffi::c_void,
            current_directory.as_ptr(),
            &startup.StartupInfo,
            &mut information,
        )
    };
    if created == 0 {
        return Err(RuntimeError::new(
            ErrorKind::System,
            format!("CreateProcessW failed for '{}'", program.display()),
        )
        .with_backend("sandbox")
        .with_operation("spawn"));
    }

    let handles = ProcessHandles {
        process: information.hProcess,
        thread: information.hThread,
    };
    let pid = information.dwProcessId;

    // Assign to the job while still suspended, then let it run. Doing this
    // after resume would let a fork escape the job.
    job.assign_process(information.hProcess)?;

    // Take ownership of the read ends: `File` closes them on drop, so the
    // PipePair must not close them itself.
    let mut stdout_pipe = stdout_pipe;
    let mut stderr_pipe = stderr_pipe;
    let stdout = unsafe { std::fs::File::from_raw_handle(stdout_pipe.read as _) };
    let stderr = unsafe { std::fs::File::from_raw_handle(stderr_pipe.read as _) };

    // The write ends are no longer needed here; the child holds its own
    // inherited copies. Leaving ours open would keep the pipes from ever
    // signalling EOF, so the log readers would hang until the daemon exits.
    unsafe {
        if !stdout_pipe.write.is_null() {
            CloseHandle(stdout_pipe.write);
        }
        if !stderr_pipe.write.is_null() {
            CloseHandle(stderr_pipe.write);
        }
    }
    // Hand ownership to the `File`s above; `Drop` skips null handles.
    stdout_pipe.read = std::ptr::null_mut();
    stdout_pipe.write = std::ptr::null_mut();
    stderr_pipe.read = std::ptr::null_mut();
    stderr_pipe.write = std::ptr::null_mut();

    handles.resume()?;

    Ok(StartedProcess {
        handles,
        pid,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_arguments_with_spaces() {
        assert_eq!(quote_argument("simple"), "simple");
        assert_eq!(quote_argument("has space"), "\"has space\"");
        assert_eq!(quote_argument(""), "\"\"");
        assert_eq!(quote_argument("say \"hi\""), "\"say \\\"hi\\\"\"");
    }

    #[test]
    fn quotes_backslashes_before_a_quote() {
        // A trailing backslash is only a problem once the argument is quoted:
        // an unquoted `C:\path\` is passed through verbatim and Windows reads
        // it correctly.
        assert_eq!(quote_argument("C:\\path\\"), "C:\\path\\");
        // Once quoting is forced (here by the space), the trailing backslash
        // must be doubled or it escapes the closing quote and swallows the
        // rest of the command line.
        assert_eq!(quote_argument("C:\\path dir\\"), "\"C:\\path dir\\\\\"");
        // `a\b c` contains a space, so it is quoted; the interior backslash is
        // not adjacent to the closing quote and is left alone.
        assert_eq!(quote_argument("a\\b c"), "\"a\\b c\"");
    }

    #[test]
    fn command_line_ends_with_a_terminator() {
        let line = build_command_line(
            Path::new("C:\\rootfs\\app.exe"),
            &["--flag".to_owned(), "value with space".to_owned()],
        );
        assert_eq!(*line.last().unwrap(), 0);
        let text: String = String::from_utf16_lossy(&line[..line.len() - 1]);
        assert!(text.contains("app.exe"));
        assert!(text.contains("\"value with space\""));
    }
}
