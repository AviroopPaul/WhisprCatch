//! Filler removal — "um", "uh", "you know", "like". The lowest of Wispr's four
//! cleanup grades, and the one users notice first.

use serde::{Deserialize, Serialize};

use crate::Transform;

/// Implemented in issue #44. This is a deliberate no-op stub so the seam can
/// land first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FillersConfig {
    /// Off until #44 lands. Nothing here changes a byte of output today.
    pub enabled: bool,
}

/// Deletes hesitation words and hedges.
///
/// Runs *after* [`crate::SelfCorrect`]: "I mean" is a hedge this transform
/// strips and a correction marker #48 needs to see first. See
/// `Polish::from_config`.
pub struct Fillers {
    #[allow(dead_code)] // read once #44 implements `apply`
    cfg: FillersConfig,
}

impl Fillers {
    pub fn new(cfg: FillersConfig) -> Self {
        Self { cfg }
    }
}

impl Transform for Fillers {
    fn name(&self) -> &'static str {
        "fillers"
    }

    /// Implemented in issue #44. This is a deliberate no-op stub so the seam
    /// can land first.
    fn apply(&self, text: &str) -> String {
        text.to_string()
    }

    /// Not prefix-stable: it deletes. A streaming pass that has heard
    /// `"so um"` types both words, and the finished `"so um yeah"` polishes to
    /// `"so yeah"` — shorter than what is already on screen. Deletion can
    /// never satisfy the prefix property.
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
        assert_noop(&Fillers::new(FillersConfig { enabled: true }));
        assert_noop(&Fillers::new(FillersConfig::default()));
    }

    #[test]
    fn disabled_by_default() {
        assert!(!FillersConfig::default().enabled);
    }

    /// #44: this one can never become true — deleting a word always shortens
    /// text a streaming pass may already have typed.
    #[test]
    fn is_not_prefix_stable() {
        assert!(!Fillers::new(FillersConfig::default()).prefix_stable());
    }

    #[test]
    fn nothing_to_validate_yet() {
        assert!(Fillers::new(FillersConfig::default()).validate().is_empty());
    }
}
