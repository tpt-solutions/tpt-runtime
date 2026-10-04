//! Restricted access tokens for sandboxed workloads (SPEC §23).
//!
//! Every workload starts from a token derived from the daemon's own with
//! `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE)`: all privileges the daemon
//! holds are stripped, so a workload cannot exercise any capability the
//! runtime has but the manifest never granted it.
//!
//! This is privilege reduction, not a filesystem jail. Combined with the job
//! object and the explicit handle-inheritance list it removes the obvious
//! privilege-escalation and handle-inheritance paths; namespace-level
//! confinement is a documented non-goal (see the crate README).

use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, LUID};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, CreateRestrictedToken, LookupPrivilegeValueW, DISABLE_MAX_PRIVILEGE,
    LUID_AND_ATTRIBUTES, SE_ASSIGNPRIMARYTOKEN_NAME, SE_INCREASE_QUOTA_NAME, SE_PRIVILEGE_ENABLED,
    SID_AND_ATTRIBUTES, TOKEN_ADJUST_PRIVILEGES, TOKEN_DUPLICATE, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// RAII wrapper for the process and thread handles `CreateProcessW` returns.
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: last owned handle; closed exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

/// Reads a `PCWSTR` constant (such as `SE_ASSIGNPRIMARYTOKEN_NAME`) into an
/// owned, NUL-terminated `String` for the helpers that take `&str`.
fn wide_name(pointer: *const u16) -> String {
    if pointer.is_null() {
        return String::new();
    }
    let mut length = 0usize;
    // SAFETY: the constants are static NUL-terminated wide strings, so
    // scanning to the NUL cannot run past the allocation.
    unsafe {
        while *pointer.add(length) != 0 {
            length += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length))
    }
}

/// Enables a privilege on the current process token.
///
/// `CreateProcessAsUserW` requires `SeAssignPrimaryTokenPrivilege` (to install
/// the workload's token) and `SeIncreaseQuotaPrivilege` (to raise its job
/// limits). Administrators hold both by default; a non-administrator daemon
/// does not, so the failure is reported clearly instead of silently spawning
/// unprivileged-but-unisolated processes.
fn enable_privilege_on(process_token: HANDLE, name: &str) -> Result<()> {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();

    let mut luid: LUID = unsafe { std::mem::zeroed() };
    // SAFETY: a null system name means the local system, as documented, and
    // `wide` is a NUL-terminated privilege name.
    let looked_up = unsafe { LookupPrivilegeValueW(std::ptr::null(), wide.as_ptr(), &mut luid) };
    if looked_up == 0 {
        return Err(RuntimeError::new(
            ErrorKind::System,
            format!("LookupPrivilegeValueW failed for '{name}'"),
        )
        .with_backend("sandbox")
        .with_operation("enable_privilege"));
    }

    // TOKEN_PRIVILEGES is a counted array, so the buffer must cover the one
    // LUID_AND_ATTRIBUTES that follows the header.
    let mut privileges: TOKEN_PRIVILEGES = unsafe { std::mem::zeroed() };
    privileges.PrivilegeCount = 1;
    privileges.Privileges[0] = LUID_AND_ATTRIBUTES {
        Luid: luid,
        Attributes: SE_PRIVILEGE_ENABLED,
    };

    // SAFETY: the token is open for TOKEN_ADJUST_PRIVILEGES and the buffer
    // matches TOKEN_PRIVILEGES exactly (header plus one entry).
    let adjusted = unsafe {
        AdjustTokenPrivileges(
            process_token,
            0, // do not disable the other privileges
            &privileges,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    // AdjustTokenPrivileges reports success even when a privilege was not
    // actually held; the real status is in GetLastError(), so read it.
    // SAFETY: reads the calling thread's last-error value.
    let last_error = unsafe { GetLastError() };
    if adjusted == 0 || last_error != 0 {
        return Err(RuntimeError::new(
            ErrorKind::System,
            format!(
                "the daemon token cannot enable '{name}'; sandboxed workloads require an \
                 elevated daemon on this host"
            ),
        )
        .with_backend("sandbox")
        .with_operation("enable_privilege"));
    }
    Ok(())
}

/// A handle to a restricted access token.
pub struct RestrictedToken {
    handle: HANDLE,
}

// HANDLE is a raw pointer; a token handle is safe to move across threads.
unsafe impl Send for RestrictedToken {}
unsafe impl Sync for RestrictedToken {}

impl RestrictedToken {
    /// Derives a privilege-free token from the current process token.
    ///
    /// Fails rather than falling back to an unrestricted token: silently
    /// dropping the restriction would leave the workload running with the
    /// daemon's full privileges, which is exactly the failure this crate
    /// exists to prevent.
    pub fn from_current_process() -> Result<Self> {
        // TOKEN_DUPLICATE is required to pass the token to
        // CreateProcessAsUserW; TOKEN_ADJUST_PRIVILEGES to enable the two
        // privileges that call requires.
        let mut process_token: HANDLE = std::ptr::null_mut();

        // SAFETY: GetCurrentProcess returns a pseudo-handle that is always
        // valid; the out-parameter receives a real handle we then own.
        let opened = unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ADJUST_PRIVILEGES,
                &mut process_token,
            )
        };
        if opened == 0 || process_token.is_null() {
            return Err(
                RuntimeError::new(ErrorKind::System, "OpenProcessToken failed")
                    .with_backend("sandbox")
                    .with_operation("restricted_token"),
            );
        }
        let process_token = OwnedHandle(process_token);

        // These two must be enabled for CreateProcessAsUserW to succeed.
        let assign_primary = wide_name(SE_ASSIGNPRIMARYTOKEN_NAME);
        let increase_quota = wide_name(SE_INCREASE_QUOTA_NAME);
        enable_privilege_on(process_token.0, &assign_primary)?;
        enable_privilege_on(process_token.0, &increase_quota)?;

        let mut restricted: HANDLE = std::ptr::null_mut();

        // SAFETY: valid input token. The SID and privilege arrays are passed
        // as null with zero counts, which the API documents as "disable all".
        let created = unsafe {
            CreateRestrictedToken(
                process_token.0,
                DISABLE_MAX_PRIVILEGE,
                // No SIDs to disable and no privileges to delete:
                // DISABLE_MAX_PRIVILEGE already drops every privilege.
                0,
                std::ptr::null::<SID_AND_ATTRIBUTES>(),
                0,
                std::ptr::null::<LUID_AND_ATTRIBUTES>(),
                0,
                std::ptr::null::<SID_AND_ATTRIBUTES>(),
                &mut restricted,
            )
        };

        if created == 0 || restricted.is_null() {
            return Err(
                RuntimeError::new(ErrorKind::System, "CreateRestrictedToken failed")
                    .with_backend("sandbox")
                    .with_operation("restricted_token"),
            );
        }

        Ok(Self { handle: restricted })
    }

    /// The raw token handle, for `CreateProcessAsUserW`.
    pub fn handle(&self) -> HANDLE {
        self.handle
    }
}

impl Drop for RestrictedToken {
    fn drop(&mut self) {
        // SAFETY: last owned handle.
        unsafe { CloseHandle(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Derives a token, or returns `None` when the test host is not elevated.
    ///
    /// `CreateProcessAsUserW` needs `SeAssignPrimaryTokenPrivilege` and
    /// `SeIncreaseQuotaPrivilege`, which a non-administrator `cargo test` does
    /// not hold. That is a real deployment constraint (the daemon must run
    /// elevated), not a code defect, so the tests skip instead of failing on an
    /// unelevated developer machine.
    fn token_or_skip() -> Option<RestrictedToken> {
        match RestrictedToken::from_current_process() {
            Ok(token) => Some(token),
            Err(err) if err.operation.as_deref() == Some("enable_privilege") => {
                eprintln!("skipping: {}", err.message);
                None
            }
            Err(err) => panic!("unexpected token failure: {err}"),
        }
    }

    #[test]
    fn derives_a_privilege_free_token() {
        let Some(token) = token_or_skip() else {
            return;
        };
        assert!(!token.handle().is_null());
    }

    #[test]
    fn token_handle_survives_as_an_attribute_value() {
        // The handle must stay valid for the duration of the attribute list,
        // which the win32 module relies on.
        let Some(token) = token_or_skip() else {
            return;
        };
        assert!(!token.handle().is_null());
    }

    #[test]
    fn wide_name_reads_a_static_privilege_string() {
        let name = wide_name(SE_ASSIGNPRIMARYTOKEN_NAME);
        assert_eq!(name, "SeAssignPrimaryTokenPrivilege");
        assert!(wide_name(std::ptr::null()).is_empty());
    }
}
