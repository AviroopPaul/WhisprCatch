//! Deterministic text cleanup — the seam between transcription and injection.
//!
//! Everything in here is a pure function of a `&str` and some config. No I/O,
//! no platform code, no network, no async, no model. That is the whole point:
//! tier 0 of the cleanup design costs zero milliseconds of inference, zero
//! megabytes of download and zero bytes of the user's audio leaving the
//! machine. If a transform ever needs any of those, it does not belong here.
//!
//! ```text
//! engine.transcribe() -> Polish::apply(raw) -> history -> inject
//! ```
//!
//! A [`Polish`] is an ordered chain of [`Transform`]s built from a
//! [`PolishConfig`]. Every transform ships disabled, so a default chain is
//! empty and `apply` returns its input byte for byte.

pub mod dictionary;
pub mod filler;
pub mod numbers;
pub mod self_correct;
pub mod snippets;
pub mod spoken;

#[cfg(test)]
mod testing;

pub use dictionary::{Dictionary, DictionaryConfig};
pub use filler::{Filler, FillerConfig};
pub use numbers::{Numbers, NumbersConfig};
pub use self_correct::{SelfCorrect, SelfCorrectConfig};
pub use snippets::{Snippets, SnippetsConfig};
pub use spoken::{Spoken, SpokenConfig};

use serde::{Deserialize, Serialize};

/// A single text transform. Implementations live in their own module file.
pub trait Transform {
    /// Stable identifier, used in config and logs.
    fn name(&self) -> &'static str;
    /// Transform the text. Must be deterministic and side-effect free.
    fn apply(&self, text: &str) -> String;
    /// True if this transform can only ever append or extend text, never
    /// shorten or reorder it. Live streaming (issue #50) relies on this:
    /// transforms that are not append-safe cannot run on streaming passes.
    fn append_safe(&self) -> bool;
}

/// A boxed transform. `Send + Sync` because the dictation loop runs on a
/// worker thread on macOS while the menu bar owns the main one, and the
/// streaming passes of #50 will want the same chain from either.
pub type BoxedTransform = Box<dyn Transform + Send + Sync>;

/// Configuration for the whole chain: one field per transform, each defaulting
/// to disabled. `#[serde(default)]` at every level is what lets a `config.toml`
/// written by an older build — with no `[polish]` section at all — keep loading.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PolishConfig {
    pub dictionary: DictionaryConfig,
    pub snippets: SnippetsConfig,
    pub spoken: SpokenConfig,
    pub self_correct: SelfCorrectConfig,
    pub fillers: FillerConfig,
    pub numbers: NumbersConfig,
}

/// The ordered transform chain.
pub struct Polish {
    chain: Vec<BoxedTransform>,
}

impl Polish {
    /// Build the chain from config. Disabled transforms are omitted entirely.
    ///
    /// The order below is load-bearing, not alphabetical and not the order the
    /// issues happen to be numbered in. Two constraints fix it:
    ///
    /// 1. `dictionary` and `snippets` run first, on text that still reads the
    ///    way the user said it, before anything has been deleted out from
    ///    under their trigger phrases.
    /// 2. **`self_correct` MUST run before `fillers`.** "I mean" is both a
    ///    correction marker (#48) and a hedge that filler removal strips (#44).
    ///    Run fillers first and self-correction can never see its own markers —
    ///    it silently stops working, with no error and no failing test unless
    ///    someone wrote one. Someone did: `self_correct_runs_before_fillers`.
    ///
    /// `numbers` runs last so it normalizes the final wording rather than
    /// digits that a later pass would have rewritten anyway.
    pub fn from_config(cfg: &PolishConfig) -> Self {
        let mut chain: Vec<BoxedTransform> = Vec::new();
        if cfg.dictionary.enabled {
            chain.push(Box::new(Dictionary::new(cfg.dictionary.clone())));
        }
        if cfg.snippets.enabled {
            chain.push(Box::new(Snippets::new(cfg.snippets.clone())));
        }
        if cfg.spoken.enabled {
            chain.push(Box::new(Spoken::new(cfg.spoken.clone())));
        }
        // self_correct before fillers — see the doc comment above.
        if cfg.self_correct.enabled {
            chain.push(Box::new(SelfCorrect::new(cfg.self_correct.clone())));
        }
        if cfg.fillers.enabled {
            chain.push(Box::new(Filler::new(cfg.fillers.clone())));
        }
        if cfg.numbers.enabled {
            chain.push(Box::new(Numbers::new(cfg.numbers.clone())));
        }
        Self { chain }
    }

    /// Build a chain from transforms directly. Escape hatch for tests and for
    /// the Settings preview (#49), which wants to show one transform's effect
    /// in isolation. [`from_config`](Self::from_config) is the production path
    /// and the only one that guarantees the documented order.
    pub fn from_transforms(chain: Vec<BoxedTransform>) -> Self {
        Self { chain }
    }

    /// Run every enabled transform in order.
    pub fn apply(&self, raw: &str) -> String {
        self.chain
            .iter()
            .fold(raw.to_string(), |acc, t| t.apply(&acc))
    }

    /// Run only the append-safe subset. Used by streaming passes (#50), which
    /// type as the user speaks and so cannot afford a transform that deletes
    /// or reorders text already on screen.
    pub fn apply_append_safe(&self, raw: &str) -> String {
        self.chain
            .iter()
            .filter(|t| t.append_safe())
            .fold(raw.to_string(), |acc, t| t.apply(&acc))
    }

    /// True when the chain contains at least one non-append-safe transform.
    pub fn has_rewriting_transforms(&self) -> bool {
        self.chain.iter().any(|t| !t.append_safe())
    }

    /// True when nothing is enabled, i.e. `apply` is the identity function.
    pub fn is_empty(&self) -> bool {
        self.chain.is_empty()
    }

    /// Names of the enabled transforms, in the order they run. For logs, for
    /// the Settings UI (#49) and for asserting the order in tests.
    pub fn names(&self) -> Vec<&'static str> {
        self.chain.iter().map(|t| t.name()).collect()
    }
}

impl std::fmt::Debug for Polish {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Polish({})", self.names().join(" -> "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{cfg_with, torture_inputs, ALL};

    /// Every transform, on.
    fn all_on() -> PolishConfig {
        cfg_with(&ALL)
    }

    /// A transform that appends its own marker, so chain order is observable
    /// even while every real transform is still a no-op stub.
    struct Marker(&'static str, bool);
    impl Transform for Marker {
        fn name(&self) -> &'static str {
            self.0
        }
        fn apply(&self, text: &str) -> String {
            format!("{text}{}", self.0)
        }
        fn append_safe(&self) -> bool {
            self.1
        }
    }

    // ---- default chain ----------------------------------------------------

    #[test]
    fn default_config_builds_an_empty_chain() {
        let p = Polish::from_config(&PolishConfig::default());
        assert!(p.is_empty());
        assert_eq!(p.names(), Vec::<&str>::new());
        assert!(!p.has_rewriting_transforms());
    }

    #[test]
    fn default_chain_is_byte_identical_on_everything() {
        let p = Polish::from_config(&PolishConfig::default());
        for input in torture_inputs() {
            assert_eq!(p.apply(&input), input, "apply changed {input:?}");
            assert_eq!(
                p.apply_append_safe(&input),
                input,
                "apply_append_safe changed {input:?}"
            );
        }
    }

    /// The promise the issue makes to existing users: turn nothing on and the
    /// v0.5 pipeline emits exactly what v0.4.0 did.
    #[test]
    fn all_stubs_on_is_still_byte_identical() {
        let p = Polish::from_config(&all_on());
        assert_eq!(p.names().len(), 6);
        for input in torture_inputs() {
            assert_eq!(p.apply(&input), input, "apply changed {input:?}");
            assert_eq!(
                p.apply_append_safe(&input),
                input,
                "apply_append_safe changed {input:?}"
            );
        }
    }

    // ---- order ------------------------------------------------------------

    #[test]
    fn chain_order_is_fixed() {
        assert_eq!(
            Polish::from_config(&all_on()).names(),
            [
                "dictionary",
                "snippets",
                "spoken",
                "self_correct",
                "fillers",
                "numbers"
            ]
        );
    }

    /// Enabling transforms in a different order must not change the order they
    /// run in — the chain order is the code's, not the config's.
    #[test]
    fn config_field_order_does_not_leak_into_the_chain() {
        let reversed: Vec<&str> = ALL.iter().rev().copied().collect();
        assert_eq!(
            Polish::from_config(&cfg_with(&reversed)).names(),
            Polish::from_config(&all_on()).names()
        );
    }

    /// The silent bug this whole ordering exists to prevent: "I mean" is a
    /// correction marker (#48) *and* a hedge filler removal strips (#44). If
    /// fillers run first, self-correction never sees its own markers.
    #[test]
    fn self_correct_runs_before_fillers() {
        let names = Polish::from_config(&all_on()).names();
        let sc = names.iter().position(|n| *n == "self_correct").unwrap();
        let fi = names.iter().position(|n| *n == "fillers").unwrap();
        assert!(
            sc < fi,
            "self_correct must run before fillers, got {names:?}"
        );
    }

    #[test]
    fn transforms_run_in_chain_order() {
        let p = Polish::from_transforms(vec![
            Box::new(Marker("a", true)),
            Box::new(Marker("b", true)),
            Box::new(Marker("c", true)),
        ]);
        assert_eq!(p.apply("x"), "xabc");
    }

    // ---- enable / disable -------------------------------------------------

    #[test]
    fn disabled_transforms_are_omitted_entirely() {
        let p = Polish::from_config(&cfg_with(&["spoken", "numbers"]));
        assert_eq!(p.names(), ["spoken", "numbers"]);
        assert!(!p.is_empty());
    }

    #[test]
    fn enabling_one_transform_builds_a_chain_of_one() {
        for name in ALL {
            assert_eq!(Polish::from_config(&cfg_with(&[name])).names(), [name]);
        }
    }

    // ---- append-safety ----------------------------------------------------

    /// Locks the per-transform flags #50 will branch on. Changing one of these
    /// changes what live streaming is allowed to run, so it is not a detail.
    #[test]
    fn append_safe_flags_match_the_contract() {
        let cases: [(BoxedTransform, bool); 6] = [
            (Box::new(Dictionary::new(Default::default())), true),
            (Box::new(Snippets::new(Default::default())), true),
            (Box::new(Spoken::new(Default::default())), true),
            (Box::new(SelfCorrect::new(Default::default())), false),
            (Box::new(Filler::new(Default::default())), false),
            (Box::new(Numbers::new(Default::default())), true),
        ];
        for (t, want) in cases {
            assert_eq!(t.append_safe(), want, "{} append_safe", t.name());
        }
    }

    #[test]
    fn apply_append_safe_skips_rewriting_transforms() {
        let p = Polish::from_transforms(vec![
            Box::new(Marker("safe1", true)),
            Box::new(Marker("rewrite", false)),
            Box::new(Marker("safe2", true)),
        ]);
        assert_eq!(p.apply("x"), "xsafe1rewritesafe2");
        assert_eq!(p.apply_append_safe("x"), "xsafe1safe2");
    }

    #[test]
    fn apply_append_safe_equals_apply_when_nothing_rewrites() {
        let p = Polish::from_transforms(vec![
            Box::new(Marker("a", true)),
            Box::new(Marker("b", true)),
        ]);
        assert!(!p.has_rewriting_transforms());
        assert_eq!(p.apply("x"), p.apply_append_safe("x"));
    }

    #[test]
    fn has_rewriting_transforms_tracks_the_deleting_ones() {
        let only_safe = cfg_with(&["dictionary", "snippets", "spoken", "numbers"]);
        assert!(!Polish::from_config(&only_safe).has_rewriting_transforms());

        for cfg in [
            cfg_with(&["fillers"]),
            cfg_with(&["self_correct"]),
            all_on(),
        ] {
            assert!(Polish::from_config(&cfg).has_rewriting_transforms());
        }
    }

    // ---- serde ------------------------------------------------------------

    #[test]
    fn config_round_trips_through_toml() {
        let cfg = all_on();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: PolishConfig = toml::from_str(&text).unwrap();
        assert_eq!(
            Polish::from_config(&back).names(),
            Polish::from_config(&cfg).names()
        );
    }

    #[test]
    fn config_round_trips_through_json() {
        let cfg = cfg_with(&["fillers"]);
        let json = serde_json::to_string(&cfg).unwrap();
        let back: PolishConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(Polish::from_config(&back).names(), ["fillers"]);
    }

    /// An empty `[polish]` table — what a v0.4.0 config gets once the field is
    /// added — must deserialize to "everything off".
    #[test]
    fn empty_table_deserializes_to_all_disabled() {
        let cfg: PolishConfig = toml::from_str("").unwrap();
        assert!(Polish::from_config(&cfg).is_empty());
    }

    #[test]
    fn partial_config_fills_the_rest_with_defaults() {
        let cfg: PolishConfig = toml::from_str("[fillers]\nenabled = true\n").unwrap();
        assert_eq!(Polish::from_config(&cfg).names(), ["fillers"]);
    }

    /// Forward compatibility: a newer build writes keys this one has never
    /// heard of. Ignoring them beats refusing to start.
    #[test]
    fn unknown_keys_are_ignored() {
        let cfg: PolishConfig =
            toml::from_str("[fillers]\nenabled = true\naggression = 3\n\n[tone]\nenabled = true\n")
                .unwrap();
        assert_eq!(Polish::from_config(&cfg).names(), ["fillers"]);
    }

    // ---- misc -------------------------------------------------------------

    #[test]
    fn debug_lists_the_chain() {
        assert_eq!(
            format!("{:?}", Polish::from_config(&all_on())),
            "Polish(dictionary -> snippets -> spoken -> self_correct -> fillers -> numbers)"
        );
        assert_eq!(
            format!("{:?}", Polish::from_config(&PolishConfig::default())),
            "Polish()"
        );
    }

    #[test]
    fn names_are_unique_and_snake_case() {
        let names = Polish::from_config(&all_on()).names();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            names.len(),
            "duplicate transform name in {names:?}"
        );
        for n in names {
            assert!(
                !n.is_empty() && n.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{n:?} is not a stable snake_case identifier"
            );
        }
    }
}
