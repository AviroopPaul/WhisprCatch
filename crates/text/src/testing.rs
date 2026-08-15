//! Shared test fixtures. `#[cfg(test)]` only — none of this ships.

use crate::{PolishConfig, Transform};

/// Every transform name, in the order [`crate::Polish::from_config`] runs them.
pub const ALL: [&str; 6] = [
    "dictionary",
    "snippets",
    "spoken",
    "self_correct",
    "fillers",
    "numbers",
];

/// A `PolishConfig` with exactly the named transforms enabled.
///
/// Deliberately not a struct literal: the six issues that implement these
/// transforms each add options to their own config, and a literal here would
/// make every one of them a breaking change to a file they should not have to
/// touch.
pub fn cfg_with(enabled: &[&str]) -> PolishConfig {
    let mut cfg = PolishConfig::default();
    for name in enabled {
        match *name {
            "dictionary" => cfg.dictionary.enabled = true,
            "snippets" => cfg.snippets.enabled = true,
            "spoken" => cfg.spoken.enabled = true,
            "self_correct" => cfg.self_correct.enabled = true,
            "fillers" => cfg.fillers.enabled = true,
            "numbers" => cfg.numbers.enabled = true,
            other => panic!("unknown transform {other:?}"),
        }
    }
    cfg
}

/// Inputs that break naive string code: empty, whitespace-only, multibyte,
/// emoji (including a ZWJ sequence and a skin-tone modifier), combining marks,
/// zero-width characters, and one input long enough to catch an accidentally
/// quadratic implementation.
pub fn torture_inputs() -> Vec<String> {
    let mut v: Vec<String> = [
        "",
        " ",
        "\t\n  \r\n",
        "\n",
        "hello world",
        "Hello, world. This is a test.",
        "naïve café résumé",
        "日本語のテキストです。",
        "Правда — это не то, что кажется",
        "👩‍💻 shipped it 🚀 👍🏽",
        "e\u{0301}gal", // combining acute, not precomposed
        "\u{200b}zero\u{200b}width\u{200b}",
        "um, I mean, like, the thing",
        "I said Tuesday, I mean Wednesday",
        "twenty five percent of one thousand",
        "   leading and trailing   ",
        "no-trailing-newline",
        "trailing newline\n",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    // ~2 MB of text. A user who dictates for ten minutes gets nowhere near
    // this; anything that chokes here is quadratic and would show up as a
    // freeze on release.
    v.push("the quick brown fox jumps over the lazy dog. ".repeat(45_000));
    // A single 100k-character token with no whitespace anywhere. Different
    // code path from the input above: anything that splits on whitespace and
    // works per word sees one enormous word here instead of many small ones.
    v.push("a".repeat(100_000));
    // Same, multibyte, so a naive byte-index slice lands mid-character.
    v.push("é".repeat(50_000));
    v
}

/// Executable definition of the property [`Transform::prefix_stable`] claims:
/// for every prefix `p` of `w`, `apply(p)` must be a prefix of `apply(w)`.
///
/// Returns the first `(p, apply(p), apply(w))` that violates it. `None` means
/// the transform held on every pair tried — which is evidence, not proof.
///
/// This is the check #50 needs and the one each of #43-#48 should run against
/// its real implementation before ever returning `true`.
pub fn prefix_violation(t: &dyn Transform, whole: &str) -> Option<(String, String, String)> {
    let polished_whole = t.apply(whole);
    // every character boundary, so multi-word triggers get cut in every place
    // they can be cut
    for (i, _) in whole
        .char_indices()
        .chain(std::iter::once((whole.len(), ' ')))
    {
        let prefix = &whole[..i];
        let polished_prefix = t.apply(prefix);
        if !polished_whole.starts_with(&polished_prefix) {
            return Some((prefix.to_string(), polished_prefix, polished_whole));
        }
    }
    None
}

/// The contract for a no-op stub: byte-identical output for every input in the
/// torture corpus. The issue that implements a transform replaces this call
/// with real cases — it is not meant to survive the implementation.
pub fn assert_noop(t: &dyn Transform) {
    for input in torture_inputs() {
        let out = t.apply(&input);
        assert_eq!(
            out,
            input,
            "{} is a stub but changed input {:?}",
            t.name(),
            truncate(&input)
        );
    }
}

pub fn truncate(s: &str) -> String {
    if s.chars().count() <= 60 {
        return s.to_string();
    }
    let head: String = s.chars().take(60).collect();
    format!("{head}… ({} chars)", s.chars().count())
}
