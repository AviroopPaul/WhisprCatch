//! Spoken formatting — "new line", "open paren", "comma" become the character
//! the user meant rather than the words they said.

use serde::{Deserialize, Serialize};

use crate::Transform;

/// Implemented in issue #45. This is a deliberate no-op stub so the seam can
/// land first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SpokenConfig {
    /// Off until #45 lands. Nothing here changes a byte of output today.
    pub enabled: bool,
}

/// Turns spoken punctuation and layout commands into characters.
pub struct Spoken {
    #[allow(dead_code)] // read once #45 implements `apply`
    cfg: SpokenConfig,
}

impl Spoken {
    pub fn new(cfg: SpokenConfig) -> Self {
        Self { cfg }
    }
}

impl Transform for Spoken {
    fn name(&self) -> &'static str {
        "spoken"
    }

    /// Implemented in issue #45. This is a deliberate no-op stub so the seam
    /// can land first.
    fn apply(&self, text: &str) -> String {
        text.to_string()
    }

    /// Not prefix-stable. "open paren" is two words: a streaming pass that has
    /// heard `"hello open"` types those 10 characters, and the finished
    /// `"hello open paren world"` polishes to `"hello ( world"`, which does
    /// not start with them. Any multi-word command can be cut in half by the
    /// prefix boundary.
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
        assert_noop(&Spoken::new(SpokenConfig { enabled: true }));
        assert_noop(&Spoken::new(SpokenConfig::default()));
    }

    #[test]
    fn disabled_by_default() {
        assert!(!SpokenConfig::default().enabled);
    }

    /// #45: keep this false unless you can prove the prefix property, and add
    /// a `prefix_violation` case here showing why the proof holds.
    #[test]
    fn is_not_prefix_stable() {
        assert!(!Spoken::new(SpokenConfig::default()).prefix_stable());
    }

    #[test]
    fn nothing_to_validate_yet() {
        assert!(Spoken::new(SpokenConfig::default()).validate().is_empty());
    }
}
