use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Wire protocol version. Deliberately unrelated to any plan-document
/// version: this is the first protocol ever on the wire, so it starts at
/// major 1 (Plan A V3 §0.1 item 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 1, minor: 0 };

/// Why a peer's version is unacceptable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionError {
    /// Different major: incompatible envelopes or semantics. Server rejects.
    MajorMismatch { ours: u16, theirs: u16 },
}

impl ProtocolVersion {
    /// Server-side acceptance check: same major required; a client minor
    /// less than or equal to ours is fine (we simply never emit the newer
    /// optional content); a *higher* client minor is also accepted — minor
    /// additions are ignorable by construction.
    pub fn check_compatible(&self, theirs: &ProtocolVersion) -> Result<(), VersionError> {
        if self.major != theirs.major {
            return Err(VersionError::MajorMismatch {
                ours: self.major,
                theirs: theirs.major,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_major_accepts_any_minor() {
        let ours = PROTOCOL_VERSION;
        assert!(ours.check_compatible(&ProtocolVersion { major: 1, minor: 0 }).is_ok());
        assert!(ours.check_compatible(&ProtocolVersion { major: 1, minor: 9 }).is_ok());
    }

    #[test]
    fn different_major_rejected() {
        let ours = PROTOCOL_VERSION;
        assert_eq!(
            ours.check_compatible(&ProtocolVersion { major: 2, minor: 0 }),
            Err(VersionError::MajorMismatch { ours: 1, theirs: 2 })
        );
    }
}
