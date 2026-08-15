//! Filler removal — "um", "uh", "you know", "like". The lowest of Wispr's four
//! cleanup grades, and the one users notice first.

use serde::{Deserialize, Serialize};

use crate::Transform;

/// Implemented in issue #44. This is a deliberate no-op stub so the seam can
/// land first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FillerConfig {
    /// Off until #44 lands. Nothing here changes a byte of output today.
    pub enabled: bool,
}

/// Deletes hesitation words and hedges.
///
/// Runs *after* [`crate::SelfCorrect`]: "I mean" is a hedge this transform
/// strips and a correction marker #48 needs to see first. See
/// `Polish::from_config`.
pub struct Filler {
    #[allow(dead_code)] // read once #44 implements `apply`
    cfg: FillerConfig,
}

impl Filler {
    pub fn new(cfg: FillerConfig) -> Self {
        Self { cfg }
    }
}

impl Transform for Filler {
    fn name(&self) -> &'static str {
        "fillers"
    }

    /// Implemented in issue #44. This is a deliberate no-op stub so the seam
    /// can land first.
    fn apply(&self, text: &str) -> String {
        text.to_string()
    }

    /// NOT append-safe: it deletes words that a streaming pass may already
    /// have typed. Live streaming (#50) must skip it.
    fn append_safe(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::assert_noop;

    #[test]
    fn stub_is_a_byte_identical_no_op() {
        assert_noop(&Filler::new(FillerConfig { enabled: true }));
        assert_noop(&Filler::new(FillerConfig::default()));
    }

    #[test]
    fn disabled_by_default() {
        assert!(!FillerConfig::default().enabled);
    }

    #[test]
    fn is_not_append_safe() {
        assert!(!Filler::new(FillerConfig::default()).append_safe());
    }
}
