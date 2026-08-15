//! Marked self-correction — "meet Tuesday, I mean Wednesday" keeps only the
//! correction. Marked phrases only; guessing at unmarked ones is out of scope
//! for the whole milestone (#36).

use serde::{Deserialize, Serialize};

use crate::Transform;

/// Implemented in issue #48. This is a deliberate no-op stub so the seam can
/// land first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SelfCorrectConfig {
    /// Off until #48 lands. Nothing here changes a byte of output today.
    pub enabled: bool,
}

/// Drops the retracted half of a marked correction.
///
/// Runs *before* [`crate::Fillers`] — "I mean" is both this transform's marker
/// and a hedge that filler removal strips. See `Polish::from_config`.
pub struct SelfCorrect {
    #[allow(dead_code)] // read once #48 implements `apply`
    cfg: SelfCorrectConfig,
}

impl SelfCorrect {
    pub fn new(cfg: SelfCorrectConfig) -> Self {
        Self { cfg }
    }
}

impl Transform for SelfCorrect {
    fn name(&self) -> &'static str {
        "self_correct"
    }

    /// Implemented in issue #48. This is a deliberate no-op stub so the seam
    /// can land first.
    fn apply(&self, text: &str) -> String {
        text.to_string()
    }

    /// Not prefix-stable, and the most obviously so: a streaming pass that has
    /// heard `"meet Tuesday"` has already typed it, and the finished
    /// `"meet Tuesday, I mean Wednesday"` polishes to `"meet Wednesday"`. The
    /// retracted words are on the user's screen before the marker that
    /// retracts them is ever spoken.
    fn prefix_stable(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::assert_noop;

    #[test]
    fn stub_is_a_byte_identical_no_op() {
        assert_noop(&SelfCorrect::new(SelfCorrectConfig { enabled: true }));
        assert_noop(&SelfCorrect::new(SelfCorrectConfig::default()));
    }

    #[test]
    fn disabled_by_default() {
        assert!(!SelfCorrectConfig::default().enabled);
    }

    /// #48: this one can never become true — a correction is only knowable
    /// after the marker, which always arrives later than the text it retracts.
    #[test]
    fn is_not_prefix_stable() {
        assert!(!SelfCorrect::new(SelfCorrectConfig::default()).prefix_stable());
    }

    #[test]
    fn nothing_to_validate_yet() {
        assert!(SelfCorrect::new(SelfCorrectConfig::default())
            .validate()
            .is_empty());
    }
}
