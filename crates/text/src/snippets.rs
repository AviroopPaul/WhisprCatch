//! Snippets — a spoken trigger expands to canned text ("my address" → the
//! actual address). Dictation's text expander.

use serde::{Deserialize, Serialize};

use crate::Transform;

/// Implemented in issue #47. This is a deliberate no-op stub so the seam can
/// land first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SnippetsConfig {
    /// Off until #47 lands. Nothing here changes a byte of output today.
    pub enabled: bool,
}

/// Expands spoken triggers into their stored text.
pub struct Snippets {
    #[allow(dead_code)] // read once #47 implements `apply`
    cfg: SnippetsConfig,
}

impl Snippets {
    pub fn new(cfg: SnippetsConfig) -> Self {
        Self { cfg }
    }
}

impl Transform for Snippets {
    fn name(&self) -> &'static str {
        "snippets"
    }

    /// Implemented in issue #47. This is a deliberate no-op stub so the seam
    /// can land first.
    fn apply(&self, text: &str) -> String {
        text.to_string()
    }

    /// Not prefix-stable. With the snippet "my address" -> "1 Infinite Loop":
    /// a streaming pass that has heard `"send to my"` types those 10
    /// characters, and the finished `"send to my address please"` polishes to
    /// `"send to 1 Infinite Loop please"` — the trigger began before the
    /// prefix boundary, so the text already on screen has to be taken back.
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
        assert_noop(&Snippets::new(SnippetsConfig { enabled: true }));
        assert_noop(&Snippets::new(SnippetsConfig::default()));
    }

    #[test]
    fn disabled_by_default() {
        assert!(!SnippetsConfig::default().enabled);
    }

    /// #47: keep this false unless you can prove the prefix property, and add
    /// a `prefix_violation` case here showing why the proof holds.
    #[test]
    fn is_not_prefix_stable() {
        assert!(!Snippets::new(SnippetsConfig::default()).prefix_stable());
    }

    #[test]
    fn nothing_to_validate_yet() {
        assert!(Snippets::new(SnippetsConfig::default())
            .validate()
            .is_empty());
    }
}
