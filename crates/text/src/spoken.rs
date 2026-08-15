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

    /// Append-safe: each command is rewritten where it sits. The result is
    /// shorter in characters but never reorders, and never reaches back past
    /// the phrase it matched.
    fn append_safe(&self) -> bool {
        true
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

    #[test]
    fn is_append_safe() {
        assert!(Spoken::new(SpokenConfig::default()).append_safe());
    }
}
