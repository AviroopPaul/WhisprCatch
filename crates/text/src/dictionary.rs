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

    /// Not prefix-stable. With the entry "push to get" -> "push to GitHub":
    /// a streaming pass that has heard `"push to get"` types all 11
    /// characters, and the finished utterance `"push to get now"` polishes to
    /// `"push to GitHub now"`, which does not start with what is on screen —
    /// the last 3 characters have to be retracted. A trigger phrase that
    /// straddles the prefix boundary breaks the property even though the
    /// substitution is "in place".
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
        assert_noop(&Dictionary::new(DictionaryConfig { enabled: true }));
        assert_noop(&Dictionary::new(DictionaryConfig::default()));
    }

    #[test]
    fn disabled_by_default() {
        assert!(!DictionaryConfig::default().enabled);
    }

    /// #43: keep this false unless you can prove the prefix property, and add
    /// a `prefix_violation` case here showing why the proof holds.
    #[test]
    fn is_not_prefix_stable() {
        assert!(!Dictionary::new(DictionaryConfig::default()).prefix_stable());
    }

    #[test]
    fn nothing_to_validate_yet() {
        assert!(Dictionary::new(DictionaryConfig::default())
            .validate()
            .is_empty());
    }
}
