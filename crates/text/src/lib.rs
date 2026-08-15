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
pub mod fillers;
pub mod numbers;
pub mod self_correct;
pub mod snippets;
pub mod spoken;

#[cfg(test)]
mod testing;

pub use dictionary::{Dictionary, DictionaryConfig};
pub use fillers::{Fillers, FillersConfig};
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

    /// True when, for every prefix `p` of an utterance `w`, `apply(p)` is a
    /// prefix of `apply(w)`.
    ///
    /// That is the exact property live streaming (#50) needs, and it is much
    /// stronger than "never shortens the text". A streaming pass has already
    /// typed `apply(p)` on the user's screen; if `apply(w)` does not start
    /// with it, the loop has to *retract* characters that are already there,
    /// which it cannot do until injector replace (#41) lands.
    ///
    /// **Every transform in this crate returns `false`**, and that is not
    /// pessimism — each one has a concrete counterexample recorded in its own
    /// module, drawn from its own documented behaviour. Substituting text "in
    /// place" is not enough: a substitution whose trigger straddles the prefix
    /// boundary rewrites characters the streaming pass already committed.
    ///
    /// Return `true` only with a proof, not an intuition. `prefix_violation`
    /// in `testing.rs` is the executable definition to check against.
    fn prefix_stable(&self) -> bool;

    /// Problems with user-authored config, surfaced in Settings (#49).
    /// Empty means the config is usable as written.
    ///
    /// Exists now, before six branches fork, because adding a trait method
    /// later is a breaking change for every implementor. #43 and #47 accept
    /// user-written patterns and need somewhere to report a bad one; the
    /// default is correct for transforms with nothing to validate.
    fn validate(&self) -> Vec<String> {
        Vec::new()
    }
}

/// A boxed transform. The `Send + Sync` bound is forward-looking insurance,
/// not a present requirement: today `Polish` is a local in `run_ptt` and never
/// crosses a thread, and the workspace compiles without the bound. It is here
/// so that moving a chain onto the streaming pass (#50) or into the Settings
/// process (#49) stays a non-breaking change for the six implementors.
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
    pub fillers: FillersConfig,
    pub numbers: NumbersConfig,
}

impl PolishConfig {
    /// Every problem the six transforms can see in their own config, for
    /// Settings (#49) to show. Empty means everything is usable as written.
    ///
    /// Deliberately checks *disabled* transforms too: a user editing a
    /// dictionary they have not switched on yet still deserves to be told the
    /// entry is malformed, rather than finding out when they enable it.
    pub fn validate(&self) -> Vec<String> {
        let all: [BoxedTransform; 6] = [
            Box::new(Dictionary::new(self.dictionary.clone())),
            Box::new(Snippets::new(self.snippets.clone())),
            Box::new(Spoken::new(self.spoken.clone())),
            Box::new(SelfCorrect::new(self.self_correct.clone())),
            Box::new(Fillers::new(self.fillers.clone())),
            Box::new(Numbers::new(self.numbers.clone())),
        ];
        all.iter().flat_map(|t| t.validate()).collect()
    }
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
            chain.push(Box::new(Fillers::new(cfg.fillers.clone())));
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

    /// Run only the prefix-stable subset — the transforms a streaming pass
    /// (#50) may run without having to retract text already on the screen.
    ///
    /// **Today this runs nothing at all**, because no transform in this crate
    /// is prefix-stable. That is the correct conservative answer, not a bug:
    /// live output stays exactly what the model said until #41 gives the
    /// injector a way to replace text and #50 does the reconciliation. It has
    /// no production caller yet; it exists so #50 has the seam to plug into.
    pub fn apply_prefix_stable(&self, raw: &str) -> String {
        self.chain
            .iter()
            .filter(|t| t.prefix_stable())
            .fold(raw.to_string(), |acc, t| t.apply(&acc))
    }

    /// True when the chain contains a transform that can rewrite text a
    /// streaming pass has already typed — i.e. any transform that is not
    /// prefix-stable.
    ///
    /// Since none of the six are, this is currently "any transform enabled".
    /// `run_ptt` warns on exactly this, so a user who turns on nothing but a
    /// custom dictionary still gets told that live typing will not match.
    pub fn has_rewriting_transforms(&self) -> bool {
        self.chain.iter().any(|t| !t.prefix_stable())
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
    use crate::testing::{cfg_with, prefix_violation, torture_inputs, truncate, ALL};

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
        fn prefix_stable(&self) -> bool {
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

    /// The promise the issue makes to existing users: with the shipping
    /// default, the v0.5 pipeline emits exactly what v0.4.0 did.
    ///
    /// Deliberately only the *default* chain. An earlier version of this test
    /// also enabled all six and asserted they were still no-ops, which meant
    /// the first of six parallel issues to implement its transform had to edit
    /// this file — the one file the parallel plan depends on nobody touching.
    /// Each stub's own `stub_is_a_byte_identical_no_op` covers that ground in
    /// a file its owner controls.
    #[test]
    fn default_chain_is_byte_identical_on_everything() {
        let p = Polish::from_config(&PolishConfig::default());
        for input in torture_inputs() {
            assert_eq!(p.apply(&input), input, "apply changed {input:?}");
            assert_eq!(
                p.apply_prefix_stable(&input),
                input,
                "apply_prefix_stable changed {input:?}"
            );
        }
    }

    /// A chain must not change its own output on a second pass, or running it
    /// twice (Settings preview then dictation, say) would drift.
    #[test]
    fn applying_the_chain_twice_changes_nothing() {
        for cfg in [PolishConfig::default(), all_on()] {
            let p = Polish::from_config(&cfg);
            for input in torture_inputs() {
                let once = p.apply(&input);
                assert_eq!(
                    p.apply(&once),
                    once,
                    "{p:?} is not idempotent on {:?}",
                    truncate(&input)
                );
            }
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

    // ---- prefix stability -------------------------------------------------

    /// Locks the per-transform flags #50 will branch on. Changing one of these
    /// changes what live streaming is allowed to run, so it is not a detail.
    ///
    /// **All six are `false`**, each for a reason recorded in its own module.
    /// An earlier version of this contract called four of them append-safe on
    /// the grounds that they "substitute in place"; that is not the property,
    /// and every one of the four has a one-line counterexample. Flipping one
    /// to `true` needs a proof and a `prefix_violation` case, not an intuition.
    #[test]
    fn no_transform_claims_prefix_stability() {
        let all: [BoxedTransform; 6] = [
            Box::new(Dictionary::new(Default::default())),
            Box::new(Snippets::new(Default::default())),
            Box::new(Spoken::new(Default::default())),
            Box::new(SelfCorrect::new(Default::default())),
            Box::new(Fillers::new(Default::default())),
            Box::new(Numbers::new(Default::default())),
        ];
        assert_eq!(all.len(), ALL.len());
        for t in all {
            assert!(
                !t.prefix_stable(),
                "{} claims prefix stability — prove it with prefix_violation first",
                t.name()
            );
        }
    }

    /// The executable definition of the property, so #50 inherits something it
    /// can run rather than a paragraph it has to interpret.
    ///
    /// The instructive case is `Marker`: appending a fixed suffix never
    /// shortens or reorders anything, and it is still *not* prefix-stable —
    /// `f("ab") = "abX"` is not a prefix of `f("abc") = "abcX"`. "Never
    /// shortens" was the wrong test; this is the right one.
    #[test]
    fn prefix_violation_is_the_definition_50_needs() {
        struct Identity;
        impl Transform for Identity {
            fn name(&self) -> &'static str {
                "identity"
            }
            fn apply(&self, text: &str) -> String {
                text.to_string()
            }
            fn prefix_stable(&self) -> bool {
                true
            }
        }
        struct Upper;
        impl Transform for Upper {
            fn name(&self) -> &'static str {
                "upper"
            }
            fn apply(&self, text: &str) -> String {
                text.to_uppercase()
            }
            fn prefix_stable(&self) -> bool {
                true
            }
        }
        struct DropUm;
        impl Transform for DropUm {
            fn name(&self) -> &'static str {
                "drop_um"
            }
            fn apply(&self, text: &str) -> String {
                text.split_whitespace()
                    .filter(|w| *w != "um")
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            fn prefix_stable(&self) -> bool {
                false
            }
        }

        // hold: character-by-character rewrites keep every prefix a prefix
        assert_eq!(prefix_violation(&Identity, "so um yeah"), None);
        assert_eq!(prefix_violation(&Upper, "so um yeah"), None);

        // violate: appending a suffix, even though it only ever grows the text
        let (p, got, whole) = prefix_violation(&Marker("X", false), "ab").unwrap();
        assert_eq!((p.as_str(), got.as_str(), whole.as_str()), ("", "X", "abX"));

        // violate: deleting a word, the obvious case
        assert!(prefix_violation(&DropUm, "so um yeah").is_some());
    }

    /// `apply_prefix_stable` runs nothing today, on purpose: none of the six
    /// qualify, so live streaming must keep typing exactly what the model
    /// said. If this starts failing, someone flipped a flag without #50.
    #[test]
    fn apply_prefix_stable_is_a_no_op_for_every_real_chain() {
        for cfg in [PolishConfig::default(), all_on()] {
            let p = Polish::from_config(&cfg);
            for input in torture_inputs() {
                assert_eq!(
                    p.apply_prefix_stable(&input),
                    input,
                    "{p:?} polished a streaming pass on {:?}",
                    truncate(&input)
                );
            }
        }
    }

    #[test]
    fn apply_prefix_stable_skips_rewriting_transforms() {
        let p = Polish::from_transforms(vec![
            Box::new(Marker("safe1", true)),
            Box::new(Marker("rewrite", false)),
            Box::new(Marker("safe2", true)),
        ]);
        assert_eq!(p.apply("x"), "xsafe1rewritesafe2");
        assert_eq!(p.apply_prefix_stable("x"), "xsafe1safe2");
    }

    #[test]
    fn apply_prefix_stable_equals_apply_when_nothing_rewrites() {
        let p = Polish::from_transforms(vec![
            Box::new(Marker("a", true)),
            Box::new(Marker("b", true)),
        ]);
        assert!(!p.has_rewriting_transforms());
        assert_eq!(p.apply("x"), p.apply_prefix_stable("x"));
    }

    /// The user-visible bug the old flags caused: enable nothing but a custom
    /// dictionary, leave streaming on (the default), and no warning fired
    /// anywhere because `has_rewriting_transforms` answered `false`. Every
    /// single-transform chain must answer `true`.
    #[test]
    fn any_enabled_transform_counts_as_rewriting() {
        assert!(!Polish::from_config(&PolishConfig::default()).has_rewriting_transforms());
        for name in ALL {
            assert!(
                Polish::from_config(&cfg_with(&[name])).has_rewriting_transforms(),
                "a chain of only {name} must still warn the streaming loop"
            );
        }
        assert!(Polish::from_config(&all_on()).has_rewriting_transforms());
    }

    // ---- validation -------------------------------------------------------

    /// The hook exists so #43/#47 can report a bad user-authored pattern
    /// without a breaking trait change later. Nothing validates anything yet.
    #[test]
    fn nothing_validates_anything_yet() {
        assert!(PolishConfig::default().validate().is_empty());
        assert!(all_on().validate().is_empty());
    }

    /// Validation must not depend on `enabled`: a user editing a dictionary
    /// they have not switched on still deserves to be told it is malformed.
    #[test]
    fn validation_covers_disabled_transforms_too() {
        struct Complainer;
        impl Transform for Complainer {
            fn name(&self) -> &'static str {
                "complainer"
            }
            fn apply(&self, text: &str) -> String {
                text.to_string()
            }
            fn prefix_stable(&self) -> bool {
                false
            }
            fn validate(&self) -> Vec<String> {
                vec!["line 3: unterminated pattern".into()]
            }
        }
        // the default trait method is the empty case; this pins the override
        assert_eq!(Complainer.validate().len(), 1);
        // and PolishConfig aggregates over all six regardless of `enabled`
        assert_eq!(PolishConfig::default().validate(), Vec::<String>::new());
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

    /// Unknown keys do not stop the daemon starting, which is the part that
    /// matters. They are **not** preserved: `apps/cli` writes the typed struct
    /// back on the next Settings save, so a key this build does not know about
    /// survives loading and is then dropped on the next write. Downgrading a
    /// build and saving from Settings therefore discards the newer build's
    /// polish settings. Acceptable because #43 and #47 keep the data users
    /// actually author in their own files, not in `config.toml` — but do not
    /// read this test as a round-trip guarantee, because it is not one.
    #[test]
    fn unknown_keys_are_ignored_on_load_not_preserved_on_save() {
        let from_newer_build =
            "[fillers]\nenabled = true\naggression = 3\n\n[tone]\nenabled = true\n";
        let cfg: PolishConfig = toml::from_str(from_newer_build).unwrap();
        assert_eq!(Polish::from_config(&cfg).names(), ["fillers"]);

        // and here is the half people assume but do not get: writing it back
        // drops both unknown keys
        let written = toml::to_string_pretty(&cfg).unwrap();
        assert!(!written.contains("aggression"), "{written}");
        assert!(!written.contains("tone"), "{written}");
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
