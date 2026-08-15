//! Custom dictionary — the user's names, jargon and acronyms, spelled the way
//! they spell them. The day-one papercut: the app gets your own name wrong.

use serde::{Deserialize, Serialize};

use crate::Transform;

/// Implemented in issue #43. This is a deliberate no-op stub so the seam can
/// land first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DictionaryConfig {
    /// Off until #43 lands. Nothing here changes a byte of output today.
    pub enabled: bool,
}

/// Replaces what the model heard with what the user actually writes.
pub struct Dictionary {
    #[allow(dead_code)] // read once #43 implements `apply`
    cfg: DictionaryConfig,
}

impl Dictionary {
    pub fn new(cfg: DictionaryConfig) -> Self {
        Self { cfg }
    }
}

impl Transform for Dictionary {
    fn name(&self) -> &'static str {
        "dictionary"
    }

    /// Implemented in issue #43. This is a deliberate no-op stub so the seam
    /// can land first.
    fn apply(&self, text: &str) -> String {
        text.to_string()
    }

    /// Append-safe: substitution happens inside a matched span, so text before
    /// it never moves and nothing already typed has to be taken back.
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
        assert_noop(&Dictionary::new(DictionaryConfig { enabled: true }));
        assert_noop(&Dictionary::new(DictionaryConfig::default()));
    }

    #[test]
    fn disabled_by_default() {
        assert!(!DictionaryConfig::default().enabled);
    }

    #[test]
    fn is_append_safe() {
        assert!(Dictionary::new(DictionaryConfig::default()).append_safe());
    }
}
