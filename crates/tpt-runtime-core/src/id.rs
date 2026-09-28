//! Runtime identifiers (SPEC §8).
//!
//! Every identifier is an opaque newtype over a string so that different
//! backends can use natural key formats without leaking into the core model.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! string_id {
    ($(#[$doc:meta])* $name:ident, $prefix:expr) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// String prefix used when generating new identifiers.
            pub const PREFIX: &'static str = $prefix;

            /// Generates a fresh identifier with the crate's canonical format
            /// (`<prefix>-<uuid v4>`), e.g. `wl-6f96f5e2-...`.
            pub fn generate() -> Self {
                Self::from_raw(format!("{}-{}", $prefix, uuid::Uuid::new_v4()))
            }

            /// Wraps an existing string without validation of its contents.
            pub fn from_raw(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Borrows the inner string.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }
    };
}

string_id!(
    /// Unique identifier of a workload.
    WorkloadId,
    "wl"
);
string_id!(
    /// Identifier of a logical volume (SPEC §15).
    VolumeId,
    "vol"
);
string_id!(
    /// Identifier of a logical network (SPEC §18).
    NetworkId,
    "net"
);
string_id!(
    /// Identifier of a device such as `gpu:0` (SPEC §19).
    DeviceId,
    "dev"
);
string_id!(
    /// Identifier of a granted capability instance.
    CapabilityId,
    "cap"
);
string_id!(
    /// Identifier of a service reachable over IPC.
    ServiceId,
    "svc"
);
string_id!(
    /// Identifier of a tracked resource allocation (SPEC §25).
    ResourceId,
    "res"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_use_prefix() {
        let id = WorkloadId::generate();
        assert!(id.as_str().starts_with("wl-"));
        assert_eq!(id.as_str().len(), "wl-".len() + 36);
    }

    #[test]
    fn ids_round_trip_through_json() {
        let id = VolumeId::from_raw("vol-project");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"vol-project\"");
        assert_eq!(serde_json::from_str::<VolumeId>(&json).unwrap(), id);
    }

    #[test]
    fn ids_display_as_inner_string() {
        assert_eq!(DeviceId::from_raw("dev-gpu-0").to_string(), "dev-gpu-0");
    }
}
