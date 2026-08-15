//! Number normalization — Parakeet writes numbers as words ("twenty five"),
//! and nobody wants that in a Slack message. SCOPE.md flagged it as a known
//! model quirk; this is where it gets fixed.

use serde::{Deserialize, Serialize};

use crate::Transform;

/// Implemented in issue #46. This is a deliberate no-op stub so the seam can
/// land first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NumbersConfig {
    /// Off until #46 lands. Nothing here changes a byte of output today.
    pub enabled: bool,
}

/// Rewrites spelled-out numbers as digits.
///
/// Runs last, so it sees the final wording rather than digits some later pass
/// would have rewritten anyway.
pub struct Numbers {
    #[allow(dead_code)] // read once #46 implements `apply`
    cfg: NumbersConfig,
}

impl Numbers {
    pub fn new(cfg: NumbersConfig) -> Self {
        Self { cfg }
    }
}

impl Transform for Numbers {
    fn name(&self) -> &'static str {
        "numbers"
    }

    /// Implemented in issue #46. This is a deliberate no-op stub so the seam
    /// can land first.
    fn apply(&self, text: &str) -> String {
        text.to_string()
    }

    /// Not prefix-stable. A streaming pass that has heard `"twenty"` types
    /// `"20"`, and the finished `"twenty five percent"` polishes to
    /// `"25 percent"` — the `0` already on screen has to become a `5`.
    /// Compound numerals are exactly the case this transform exists for, and
    /// they always straddle the prefix boundary partway through.
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
        assert_noop(&Numbers::new(NumbersConfig { enabled: true }));
        assert_noop(&Numbers::new(NumbersConfig::default()));
    }

    #[test]
    fn disabled_by_default() {
        assert!(!NumbersConfig::default().enabled);
    }

    /// #46: keep this false unless you can prove the prefix property, and add
    /// a `prefix_violation` case here showing why the proof holds.
    #[test]
    fn is_not_prefix_stable() {
        assert!(!Numbers::new(NumbersConfig::default()).prefix_stable());
    }

    #[test]
    fn nothing_to_validate_yet() {
        assert!(Numbers::new(NumbersConfig::default()).validate().is_empty());
    }
}
