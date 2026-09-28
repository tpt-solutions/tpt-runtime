//! Workload lifecycle states and the transition table (SPEC §3.1, §26).

use serde::{Deserialize, Serialize};
use std::fmt;

/// Lifecycle state of a workload.
///
/// Canonical progression (SPEC §3.1):
///
/// ```text
/// defined → resolved → prepared → created → started → running
///         → paused → stopped → destroyed
/// ```
///
/// `failed` is reachable from every state; `destroyed` is terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadState {
    /// The workload definition exists but has not been validated/resolved.
    Defined,
    /// Manifest resolved: images, volumes and networks located.
    Resolved,
    /// Execution environment prepared (filesystem, capabilities, resources).
    Prepared,
    /// Workload instantiated but not yet started.
    Created,
    /// Start requested; the backend is bringing the workload up.
    Starting,
    /// Workload is executing.
    Running,
    /// Workload temporarily suspended.
    Paused,
    /// Stop requested; the backend is tearing the workload down.
    Stopping,
    /// Workload terminated (by request, exit or failure) and not running.
    Stopped,
    /// Workload terminated abnormally.
    Failed,
    /// Workload removed; all runtime resources released. Terminal.
    Destroyed,
}

impl WorkloadState {
    /// True while the workload is (transitively) occupying execution resources.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            WorkloadState::Created
                | WorkloadState::Starting
                | WorkloadState::Running
                | WorkloadState::Paused
                | WorkloadState::Stopping
        )
    }

    /// True when the workload has finished executing and only records remain.
    pub fn is_terminal(self) -> bool {
        matches!(self, WorkloadState::Stopped | WorkloadState::Failed | WorkloadState::Destroyed)
    }

    /// Returns the state reachable from `self` via `transition`, or `None`
    /// when the transition is not permitted by the lifecycle table.
    pub fn transition(self, target: WorkloadState) -> Option<WorkloadState> {
        if LIFECYCLE_TRANSITIONS.contains(&(self, target)) {
            Some(target)
        } else {
            None
        }
    }
}

impl fmt::Display for WorkloadState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            WorkloadState::Defined => "defined",
            WorkloadState::Resolved => "resolved",
            WorkloadState::Prepared => "prepared",
            WorkloadState::Created => "created",
            WorkloadState::Starting => "starting",
            WorkloadState::Running => "running",
            WorkloadState::Paused => "paused",
            WorkloadState::Stopping => "stopping",
            WorkloadState::Stopped => "stopped",
            WorkloadState::Failed => "failed",
            WorkloadState::Destroyed => "destroyed",
        };
        f.write_str(name)
    }
}

/// Every transition the runtime permits, in `(from, to)` form.
///
/// Derived from the SPEC §3.1 lifecycle plus the operational reality that a
/// stopped workload may be started again and every state may fail or be
/// destroyed (SPEC §26: create/start/pause/resume/restart/stop/kill/destroy).
pub const LIFECYCLE_TRANSITIONS: &[(WorkloadState, WorkloadState)] = &[
    (WorkloadState::Defined, WorkloadState::Resolved),
    (WorkloadState::Resolved, WorkloadState::Prepared),
    (WorkloadState::Prepared, WorkloadState::Created),
    (WorkloadState::Created, WorkloadState::Starting),
    (WorkloadState::Starting, WorkloadState::Running),
    (WorkloadState::Running, WorkloadState::Paused),
    (WorkloadState::Paused, WorkloadState::Running),
    (WorkloadState::Running, WorkloadState::Stopping),
    (WorkloadState::Paused, WorkloadState::Stopping),
    (WorkloadState::Starting, WorkloadState::Stopping),
    (WorkloadState::Stopping, WorkloadState::Stopped),
    (WorkloadState::Stopped, WorkloadState::Starting),
    // failure can interrupt any active or transitional state
    (WorkloadState::Defined, WorkloadState::Failed),
    (WorkloadState::Resolved, WorkloadState::Failed),
    (WorkloadState::Prepared, WorkloadState::Failed),
    (WorkloadState::Created, WorkloadState::Failed),
    (WorkloadState::Starting, WorkloadState::Failed),
    (WorkloadState::Running, WorkloadState::Failed),
    (WorkloadState::Paused, WorkloadState::Failed),
    (WorkloadState::Stopping, WorkloadState::Failed),
    // terminal cleanup (a never-started workload may be discarded directly)
    (WorkloadState::Created, WorkloadState::Destroyed),
    (WorkloadState::Stopped, WorkloadState::Destroyed),
    (WorkloadState::Failed, WorkloadState::Destroyed),
];

#[cfg(test)]
mod tests {
    use super::*;
    use WorkloadState as S;

    #[test]
    fn canonical_progression_is_permitted() {
        let path = [
            S::Defined,
            S::Resolved,
            S::Prepared,
            S::Created,
            S::Starting,
            S::Running,
            S::Paused,
            S::Running,
            S::Stopping,
            S::Stopped,
            S::Destroyed,
        ];
        for pair in path.windows(2) {
            assert_eq!(
                pair[0].transition(pair[1]),
                Some(pair[1]),
                "{} → {} must be allowed",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn restart_is_permitted_from_stopped() {
        assert_eq!(S::Stopped.transition(S::Starting), Some(S::Starting));
    }

    #[test]
    fn illegal_transitions_are_rejected() {
        assert_eq!(S::Defined.transition(S::Running), None);
        assert_eq!(S::Running.transition(S::Created), None);
        assert_eq!(S::Destroyed.transition(S::Starting), None);
        assert_eq!(S::Failed.transition(S::Running), None);
    }

    #[test]
    fn failure_is_reachable_from_active_states() {
        for state in [S::Created, S::Starting, S::Running, S::Paused, S::Stopping] {
            assert_eq!(state.transition(S::Failed), Some(S::Failed), "{state} → failed");
        }
    }

    #[test]
    fn destroyed_is_terminal() {
        assert!(S::Destroyed.is_terminal());
        assert!(!S::Destroyed.is_active());
    }

    #[test]
    fn serializes_as_snake_case() {
        assert_eq!(serde_json::to_string(&S::Starting).unwrap(), "\"starting\"");
    }
}
