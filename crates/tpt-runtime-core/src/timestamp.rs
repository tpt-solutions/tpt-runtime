//! UTC timestamps serialized as RFC 3339 strings.

use std::fmt;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// A UTC instant serialized as an RFC 3339 string (`2026-01-31T12:00:00Z`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(OffsetDateTime);

impl serde::Serialize for Timestamp {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        time::serde::rfc3339::serialize(&self.0, serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Timestamp {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        time::serde::rfc3339::deserialize(deserializer).map(Self)
    }
}

impl Timestamp {
    /// Current UTC time.
    pub fn now() -> Self {
        Self(OffsetDateTime::now_utc())
    }

    /// Borrows the underlying [`OffsetDateTime`].
    pub fn as_offset(&self) -> OffsetDateTime {
        self.0
    }

    /// Unix epoch seconds.
    pub fn unix_seconds(&self) -> i64 {
        self.0.unix_timestamp()
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.format(&Rfc3339).map_err(|_| fmt::Error)?)
    }
}

impl From<OffsetDateTime> for Timestamp {
    fn from(value: OffsetDateTime) -> Self {
        Self(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_rfc3339_json() {
        let ts = Timestamp::now();
        let json = serde_json::to_string(&ts).unwrap();
        let parsed: Timestamp = serde_json::from_str(&json).unwrap();
        assert_eq!(ts, parsed);
        // string form must itself parse as RFC 3339
        let raw = json.trim_matches('"');
        OffsetDateTime::parse(raw, &Rfc3339).unwrap();
    }

    #[test]
    fn displays_as_utc_rfc3339() {
        let ts = Timestamp::from(OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap());
        assert_eq!(ts.to_string(), "2023-11-14T22:13:20Z");
    }
}
