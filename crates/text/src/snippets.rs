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

    /// Append-safe: an expansion replaces its trigger in place and generally
    /// grows the text. Nothing before the trigger moves.
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
        assert_noop(&Snippets::new(SnippetsConfig { enabled: true }));
        assert_noop(&Snippets::new(SnippetsConfig::default()));
    }

    #[test]
    fn disabled_by_default() {
        assert!(!SnippetsConfig::default().enabled);
    }

    #[test]
    fn is_append_safe() {
        assert!(Snippets::new(SnippetsConfig::default()).append_safe());
    }
}
