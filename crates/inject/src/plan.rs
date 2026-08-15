//! What to send, decided without a display server.
//!
//! Replacing text that is already on screen is the one injector operation that
//! can destroy the user's own writing: every Backspace we get wrong eats a
//! character nobody asked us to touch. The decision of *how many* to send is
//! therefore kept here — pure functions over strings — and the platform code in
//! [`crate::Injector`] is left as a translator with no arithmetic in it.
//!
//! The seam is an [`Action`] sequence plus the [`KeyboardSink`] trait. A test
//! can build the plan for any old/new pair and assert the exact events, and
//! [`Plan::simulate`] replays them against a model text field, which is what
//! makes the round-trip property test possible with no compositor in sight.
//!
//! ```
//! use wc_inject::plan::{Action, Plan, PlanOpts};
//! let plan = Plan::replace("hello world", "hello there", PlanOpts::typing_only());
//! assert_eq!(
//!     plan.actions(),
//!     [
//!         Action::LiftModifiers,
//!         Action::Backspace(5),
//!         Action::Type("there".into())
//!     ]
//! );
//! ```

use anyhow::Result;

use crate::unicode::{
    char_index_from_end, cluster_count, common_cluster_prefix, snap_to_cluster, trim_to_last_chars,
};

/// Type runs longer than this go through the pasteboard on backends that have
/// one. Synthesising a few hundred keystrokes is slow enough that apps start
/// dropping them; a paste is one event.
///
/// Strictly greater: 200 chars is typed, 201 is pasted.
pub const PASTE_THRESHOLD: usize = 200;

/// How much of what we typed the injector remembers, in chars. A replace can
/// only reach back this far — see [`Typed`].
pub const TYPED_MEMORY_CHARS: usize = 4096;

/// One step of synthetic input. Deliberately coarse: a backend decides how to
/// pace and chunk, never *what* to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Release any modifier the user is still holding, before anything else in
    /// the plan is sent.
    ///
    /// SCOPE.md calls this the critical push-to-talk bug class: our keystrokes
    /// combine with the held hotkey and the focused app runs shortcuts instead
    /// of editing text. A backspace run is more exposed than a type run, not
    /// less — ⌥/Ctrl+Backspace deletes a whole *word* in most editors, so a
    /// five-press correction can take out a sentence.
    LiftModifiers,
    /// Press Backspace this many times. The count is in grapheme clusters,
    /// because that is what one press removes.
    Backspace(usize),
    /// Type this text as characters.
    Type(String),
    /// Put this text on the clipboard and paste it. Only ever emitted when the
    /// backend reports [`Capabilities::paste`].
    Paste(String),
}

/// What a backend can actually do. The planner asks before it decides, so a
/// backend without a pasteboard never receives a [`Action::Paste`] it would
/// have to fail on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    /// The backend can put text on the clipboard and paste it.
    pub paste: bool,
}

impl Capabilities {
    /// Keystrokes only. This is what every backend in the tree reports today;
    /// see the crate docs for why there is no pasteboard path yet.
    pub const TYPING_ONLY: Self = Self { paste: false };
}

/// Planner knobs, derived from a backend's [`Capabilities`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanOpts {
    /// Type runs longer than this are pasted instead. `None` always types.
    pub paste_threshold: Option<usize>,
}

impl PlanOpts {
    /// Never paste.
    pub fn typing_only() -> Self {
        Self {
            paste_threshold: None,
        }
    }

    pub fn from_capabilities(caps: Capabilities) -> Self {
        Self {
            paste_threshold: caps.paste.then_some(PASTE_THRESHOLD),
        }
    }
}

impl Default for PlanOpts {
    fn default() -> Self {
        Self::typing_only()
    }
}

/// Everything a [`Plan`] needs from a keyboard. Implemented by
/// [`crate::Injector`] per platform, and by fakes in tests.
pub trait KeyboardSink {
    /// What this backend can do. The default is keystrokes only.
    fn capabilities(&self) -> Capabilities {
        Capabilities::TYPING_ONLY
    }

    /// Release modifiers the user is still holding. Best effort: on backends
    /// that cannot fake a release, the same protection has to come from the
    /// events themselves carrying no modifier flags.
    fn lift_modifiers(&mut self) -> Result<()>;

    fn send_backspaces(&mut self, count: usize) -> Result<()>;

    fn send_text(&mut self, text: &str) -> Result<()>;

    /// Paste via the clipboard. Unreachable unless [`Capabilities::paste`] is
    /// set, so the default refuses rather than silently typing instead — a
    /// silent fallback would make the threshold untestable from the outside.
    fn send_paste(&mut self, text: &str) -> Result<()> {
        let _ = text;
        anyhow::bail!("this backend has no pasteboard path")
    }
}

/// An ordered, already-decided list of [`Action`]s.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    actions: Vec<Action>,
}

impl Plan {
    /// The events that turn `old` — text already on screen, ending at the
    /// cursor — into `new`.
    ///
    /// Only the changed tail is touched. The shared leading clusters are left
    /// alone rather than deleted and retyped, which matters most for streaming
    /// (#50), where every pass re-types an utterance that mostly did not
    /// change: without the common-prefix trim, a one-word correction would
    /// flash the whole sentence.
    ///
    /// One case is deliberately under-deleted. If `old` begins with a character
    /// that joins the cluster before it — a combining mark, a ZWJ — then on
    /// screen it has merged with text we did not type, and no number of
    /// Backspace presses removes ours without removing theirs. We leave that
    /// first cluster alone: the result is a stray mark the user can see and
    /// delete, rather than one of their own characters silently gone.
    pub fn replace(old: &str, new: &str, opts: PlanOpts) -> Self {
        let shared = common_cluster_prefix(old, new);
        let doomed = &old[shared..];
        let fresh = &new[shared..];

        let mut backspaces = cluster_count(doomed);
        // Only the very start of `old` is uncertain. Any boundary further in
        // was segmented with our own text on both sides of it, so it is real.
        if shared == 0 && joins_the_cluster_before(doomed) {
            backspaces -= 1;
        }
        let mut actions = Vec::new();
        if backspaces == 0 && fresh.is_empty() {
            return Self { actions };
        }
        actions.push(Action::LiftModifiers);
        if backspaces > 0 {
            actions.push(Action::Backspace(backspaces));
        }
        if !fresh.is_empty() {
            let paste = opts
                .paste_threshold
                .is_some_and(|limit| fresh.chars().count() > limit);
            actions.push(if paste {
                Action::Paste(fresh.to_string())
            } else {
                Action::Type(fresh.to_string())
            });
        }
        Self { actions }
    }

    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Send the plan. Stops at the first failure: a half-applied replace is
    /// bad, but carrying on after the backspaces failed would be worse.
    pub fn run(&self, keyboard: &mut dyn KeyboardSink) -> Result<()> {
        for action in &self.actions {
            match action {
                Action::LiftModifiers => keyboard.lift_modifiers()?,
                Action::Backspace(n) => keyboard.send_backspaces(*n)?,
                Action::Type(text) => keyboard.send_text(text)?,
                Action::Paste(text) => keyboard.send_paste(text)?,
            }
        }
        Ok(())
    }

    /// Replay the plan against a model text field: one Backspace removes one
    /// grapheme cluster, typing and pasting append at the cursor.
    ///
    /// This is the executable definition of what the plan *means*. The
    /// round-trip property — apply `Plan::replace(old, new)` to anything ending
    /// in `old` and get the same thing ending in `new` — is the strongest
    /// guarantee available without a compositor, so it is checked here rather
    /// than described in a comment.
    pub fn simulate(&self, before: &str) -> String {
        let mut buf = before.to_string();
        for action in &self.actions {
            match action {
                Action::LiftModifiers => {}
                // Dropping the last `n` clusters in one go is the same result
                // as `n` separate presses: cluster boundaries never depend on
                // text further to the right, so removing a suffix cannot
                // re-segment what is left.
                Action::Backspace(n) => {
                    buf = take_clusters(&buf, cluster_count(&buf).saturating_sub(*n));
                }
                Action::Type(text) | Action::Paste(text) => buf.push_str(text),
            }
        }
        buf
    }
}

/// The first `n` grapheme clusters of `s`.
fn take_clusters(s: &str, n: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    s.graphemes(true).take(n).collect()
}

/// True when `s` starts with a character that joins whatever precedes it, so
/// that on screen its first cluster is partly made of text we did not type.
///
/// Probed rather than looked up: prepend a plain base letter and see whether
/// the cluster count goes up. That covers combining marks, ZWJ and spacing
/// marks — everything reachable from transcribed text. It does not detect a
/// regional-indicator pairing (a flag half typed against another flag half),
/// which needs a preceding regional indicator to occur at all.
fn joins_the_cluster_before(s: &str) -> bool {
    !s.is_empty() && cluster_count(&format!("a{s}")) == cluster_count(s)
}

/// The tail of what the injector typed, so a `replace_last(n_chars, ..)` can be
/// resolved into actual text.
///
/// A char count on its own is not enough to plan a replace: cluster boundaries,
/// the common prefix and the paste decision all need the characters themselves.
/// The record is bounded ([`TYPED_MEMORY_CHARS`]) and never counts as consent —
/// [`Typed::plan_replace`] will not emit more backspaces than there is recorded
/// text, so a caller asking for more than we typed cannot reach text the user
/// wrote themselves.
#[derive(Debug, Clone)]
pub struct Typed {
    buf: String,
    cap: usize,
}

impl Default for Typed {
    fn default() -> Self {
        Self::new()
    }
}

impl Typed {
    pub fn new() -> Self {
        Self::with_memory(TYPED_MEMORY_CHARS)
    }

    pub fn with_memory(cap: usize) -> Self {
        Self {
            buf: String::new(),
            cap,
        }
    }

    /// Note text that reached the screen.
    pub fn record(&mut self, text: &str) {
        self.buf.push_str(text);
        trim_to_last_chars(&mut self.buf, self.cap);
    }

    /// Forget everything. Called whenever we stop being sure what is on screen:
    /// after a failed injection, and by callers when focus may have moved.
    pub fn forget(&mut self) {
        self.buf.clear();
    }

    /// The recorded tail.
    pub fn known(&self) -> &str {
        &self.buf
    }

    /// How many chars a `replace_last` can reach back over.
    pub fn known_chars(&self) -> usize {
        self.buf.chars().count()
    }

    /// Plan a `replace_last(n_chars, new_text)` and update the record to what
    /// the screen will read afterwards.
    ///
    /// `n_chars` counts chars and is clamped to what we typed. If it lands
    /// inside a grapheme cluster the deletion widens to the whole cluster and
    /// the swept-in characters are retyped ahead of `new_text` — the keyboard
    /// has no way to delete half a cluster, so the only honest options are to
    /// widen or to refuse, and widening keeps the visible result exact.
    pub fn plan_replace(&mut self, n_chars: usize, new_text: &str, opts: PlanOpts) -> Plan {
        // One clamp, not two: `char_index_from_end` saturates at the start of
        // the record, which is the only thing standing between an over-large
        // count and the user's own text. A second `min` here would look like
        // belt and braces and would in fact hide a broken belt from the tests.
        let asked = char_index_from_end(&self.buf, n_chars);
        let cut = snap_to_cluster(&self.buf, asked);

        let old = self.buf[cut..].to_string();
        let salvaged = &self.buf[cut..asked];
        let new = format!("{salvaged}{new_text}");

        let plan = Plan::replace(&old, &new, opts);
        // Replay the plan over the record rather than assuming it did what was
        // asked: when the plan spares a cluster that had merged with text in
        // front of ours, the record has to keep it too, or the next replace
        // will be counted against a screen that does not exist.
        self.buf = plan.simulate(&self.buf);
        trim_to_last_chars(&mut self.buf, self.cap);
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    const E_ACUTE: &str = "e\u{301}";

    fn typing() -> PlanOpts {
        PlanOpts::typing_only()
    }

    fn pasting() -> PlanOpts {
        PlanOpts::from_capabilities(Capabilities { paste: true })
    }

    fn plan(old: &str, new: &str) -> Vec<Action> {
        Plan::replace(old, new, typing()).actions().to_vec()
    }

    fn typed(text: &str) -> Action {
        Action::Type(text.to_string())
    }

    // --- the shape of a plan -------------------------------------------------

    #[test]
    fn replaces_only_the_changed_tail() {
        assert_eq!(
            plan("hello world", "hello there"),
            [Action::LiftModifiers, Action::Backspace(5), typed("there")]
        );
    }

    #[test]
    fn identical_text_is_no_work_at_all() {
        assert!(Plan::replace("same", "same", typing()).is_empty());
        assert!(Plan::replace("", "", typing()).is_empty());
    }

    #[test]
    fn appending_sends_no_backspaces() {
        assert_eq!(
            plan("hello", "hello world"),
            [Action::LiftModifiers, typed(" world")]
        );
    }

    #[test]
    fn deleting_sends_no_typing() {
        assert_eq!(
            plan("hello world", "hello"),
            [Action::LiftModifiers, Action::Backspace(6)]
        );
    }

    #[test]
    fn a_completely_different_string_deletes_everything() {
        assert_eq!(
            plan("abc", "xyz"),
            [Action::LiftModifiers, Action::Backspace(3), typed("xyz")]
        );
    }

    #[test]
    fn every_emitting_plan_lifts_modifiers_first() {
        for (old, new) in [
            ("hello world", "hello there"),
            ("", "typed from nothing"),
            ("all of it", ""),
            (FAMILY, "gone"),
        ] {
            let actions = plan(old, new);
            assert_eq!(
                actions.first(),
                Some(&Action::LiftModifiers),
                "{old:?} -> {new:?}"
            );
        }
    }

    // --- counting: clusters, not chars, not bytes ---------------------------

    #[test]
    fn an_emoji_zwj_sequence_is_one_backspace() {
        // Seven chars, 25 UTF-8 bytes, one press. Counting chars here would
        // delete six characters of the user's own text.
        assert_eq!(FAMILY.chars().count(), 7);
        assert_eq!(
            plan(&format!("hi {FAMILY}"), "hi "),
            [Action::LiftModifiers, Action::Backspace(1)]
        );
    }

    #[test]
    fn a_combining_mark_goes_with_its_base() {
        // `café` -> `cafe`: the shared prefix is `caf`, and the whole `é`
        // cluster is deleted and the bare `e` retyped. Deleting "one char"
        // would take the acute off in some apps and the whole `é` in others.
        assert_eq!(
            plan(&format!("caf{E_ACUTE}"), "cafe"),
            [Action::LiftModifiers, Action::Backspace(1), typed("e")]
        );
    }

    #[test]
    fn flags_and_skin_tones_are_single_presses() {
        assert_eq!(
            plan("go 🇯🇵", "go "),
            [Action::LiftModifiers, Action::Backspace(1)]
        );
        assert_eq!(
            plan("nice 👍🏽", "nice "),
            [Action::LiftModifiers, Action::Backspace(1)]
        );
    }

    #[test]
    fn cjk_counts_by_character_not_byte() {
        assert_eq!(
            plan("你好世界", "你好朋友"),
            [Action::LiftModifiers, Action::Backspace(2), typed("朋友")]
        );
    }

    #[test]
    fn rtl_text_counts_in_logical_order() {
        // Hebrew: the shared logical prefix is "שלום ", the last word changes.
        assert_eq!(
            plan("שלום עולם", "שלום חבר"),
            [Action::LiftModifiers, Action::Backspace(4), typed("חבר")]
        );
        // Arabic, same idea.
        assert_eq!(
            Plan::replace("مرحبا بالعالم", "مرحبا يا صديق", typing()).simulate("مرحبا بالعالم"),
            "مرحبا يا صديق"
        );
    }

    // --- the paste threshold ------------------------------------------------

    #[test]
    fn the_threshold_is_strictly_greater_than_200() {
        for (len, expect_paste) in [(199, false), (200, false), (201, true)] {
            let new = "x".repeat(len);
            let actions = Plan::replace("", &new, pasting()).actions().to_vec();
            let last = actions.last().unwrap();
            assert_eq!(
                matches!(last, Action::Paste(_)),
                expect_paste,
                "{len} chars: {last:?}"
            );
        }
    }

    #[test]
    fn without_a_pasteboard_everything_is_typed() {
        let new = "x".repeat(5_000);
        let actions = Plan::replace("", &new, typing()).actions().to_vec();
        assert_eq!(actions, [Action::LiftModifiers, typed(&new)]);
    }

    #[test]
    fn the_threshold_measures_the_new_text_in_both_directions() {
        // Long -> short: 500 clusters deleted, a short type. Never a paste,
        // however much is being removed.
        let long = "y".repeat(500);
        let actions = Plan::replace(&long, "short", pasting()).actions().to_vec();
        assert_eq!(
            actions,
            [
                Action::LiftModifiers,
                Action::Backspace(500),
                typed("short")
            ]
        );

        // Short -> long: three presses, then a paste.
        let long = "z".repeat(500);
        let actions = Plan::replace("abc", &long, pasting()).actions().to_vec();
        assert_eq!(
            actions,
            [
                Action::LiftModifiers,
                Action::Backspace(3),
                Action::Paste(long)
            ]
        );
    }

    #[test]
    fn the_threshold_counts_what_is_left_after_the_common_prefix() {
        // 400 chars shared, 201 new: the prefix trim is what decides this, not
        // the length of either whole string.
        let shared = "s".repeat(400);
        let old = format!("{shared}old");
        let new = format!("{shared}{}", "n".repeat(201));
        let actions = Plan::replace(&old, &new, pasting()).actions().to_vec();
        assert!(matches!(actions.last(), Some(Action::Paste(_))));

        let new = format!("{shared}{}", "n".repeat(200));
        let actions = Plan::replace(&old, &new, pasting()).actions().to_vec();
        assert!(matches!(actions.last(), Some(Action::Type(_))));
    }

    #[test]
    fn capabilities_decide_whether_paste_is_reachable() {
        assert_eq!(
            PlanOpts::from_capabilities(Capabilities::TYPING_ONLY).paste_threshold,
            None
        );
        assert_eq!(
            PlanOpts::from_capabilities(Capabilities { paste: true }).paste_threshold,
            Some(PASTE_THRESHOLD)
        );
    }

    // --- resolving a char count against what we typed ------------------------

    #[test]
    fn replace_last_resolves_against_the_recorded_text() {
        let mut typed_record = Typed::new();
        typed_record.record("hello world");
        let plan = typed_record.plan_replace(5, "there", typing());
        assert_eq!(
            plan.actions(),
            [Action::LiftModifiers, Action::Backspace(5), typed("there")]
        );
        assert_eq!(typed_record.known(), "hello there");
    }

    #[test]
    fn zero_chars_is_a_plain_type() {
        let mut typed_record = Typed::new();
        typed_record.record("hello");
        let plan = typed_record.plan_replace(0, " world", typing());
        assert_eq!(plan.actions(), [Action::LiftModifiers, typed(" world")]);
        assert_eq!(typed_record.known(), "hello world");
    }

    #[test]
    fn zero_chars_and_no_text_does_nothing() {
        let mut typed_record = Typed::new();
        typed_record.record("hello");
        assert!(typed_record.plan_replace(0, "", typing()).is_empty());
        assert_eq!(typed_record.known(), "hello");
    }

    #[test]
    fn a_replace_never_reaches_past_what_we_typed() {
        let mut typed_record = Typed::new();
        typed_record.record("ours");
        // The caller asks for 50 chars; only 4 of them are ours. The user's own
        // text sits in front of it and must survive.
        let plan = typed_record.plan_replace(50, "new", typing());
        assert_eq!(
            plan.actions(),
            [Action::LiftModifiers, Action::Backspace(4), typed("new")]
        );
        assert_eq!(plan.simulate("USER TEXT ours"), "USER TEXT new");
    }

    #[test]
    fn a_replace_with_nothing_recorded_only_types() {
        let mut typed_record = Typed::new();
        let plan = typed_record.plan_replace(10, "new", typing());
        assert_eq!(plan.actions(), [Action::LiftModifiers, typed("new")]);
        assert_eq!(plan.simulate("USER TEXT"), "USER TEXTnew");
    }

    #[test]
    fn a_count_landing_inside_a_cluster_widens_and_retypes() {
        // Recorded "café" decomposed; the caller asks to take back one char,
        // which is the combining acute. Backspace cannot do that, so the whole
        // `é` goes and the `e` is retyped with the new text after it.
        let mut typed_record = Typed::new();
        typed_record.record(&format!("caf{E_ACUTE}"));
        let plan = typed_record.plan_replace(1, "X", typing());
        assert_eq!(
            plan.actions(),
            [Action::LiftModifiers, Action::Backspace(1), typed("eX")]
        );
        assert_eq!(plan.simulate(&format!("caf{E_ACUTE}")), "cafeX");
        assert_eq!(typed_record.known(), "cafeX");
    }

    #[test]
    fn the_record_survives_repeated_replaces() {
        let mut typed_record = Typed::new();
        let mut screen = String::from("> ");
        for (n, text) in [(0, "one two"), (3, "three"), (5, "four"), (0, " five")] {
            let plan = typed_record.plan_replace(n, text, typing());
            screen = plan.simulate(&screen);
        }
        assert_eq!(screen, "> one four five");
        assert_eq!(typed_record.known(), "one four five");
    }

    #[test]
    fn the_record_is_bounded() {
        let mut typed_record = Typed::with_memory(8);
        typed_record.record("abcdefghijkl");
        assert_eq!(typed_record.known(), "efghijkl");
        assert_eq!(typed_record.known_chars(), 8);
        // And a replace can only reach back over what is left.
        let plan = typed_record.plan_replace(100, "", typing());
        assert_eq!(
            plan.actions(),
            [Action::LiftModifiers, Action::Backspace(8)]
        );
    }

    #[test]
    fn forgetting_the_record_makes_the_next_replace_type_only() {
        let mut typed_record = Typed::new();
        typed_record.record("hello");
        typed_record.forget();
        let plan = typed_record.plan_replace(5, "bye", typing());
        assert_eq!(plan.actions(), [Action::LiftModifiers, typed("bye")]);
    }

    // --- running a plan against a backend ------------------------------------

    /// A keyboard that remembers what the user is still holding down. Starting
    /// it with modifiers latched is the whole point: it is how the "a replace
    /// never leaves a latched modifier behind" requirement is checked without a
    /// compositor.
    #[derive(Default)]
    struct FakeKeyboard {
        latched: Vec<&'static str>,
        screen: String,
        caps: Capabilities,
        fail_paste: bool,
        log: Vec<String>,
    }

    impl FakeKeyboard {
        fn holding(mods: &[&'static str]) -> Self {
            Self {
                latched: mods.to_vec(),
                ..Self::default()
            }
        }
    }

    impl KeyboardSink for FakeKeyboard {
        fn capabilities(&self) -> Capabilities {
            self.caps
        }

        fn lift_modifiers(&mut self) -> Result<()> {
            self.latched.clear();
            self.log.push("lift".into());
            Ok(())
        }

        fn send_backspaces(&mut self, count: usize) -> Result<()> {
            // A held modifier turns Backspace into delete-word (or worse, an
            // app shortcut). Model it as damage rather than as a no-op, so the
            // assertion below has something to catch.
            assert!(
                self.latched.is_empty(),
                "backspace sent with {:?} still held",
                self.latched
            );
            self.screen = take_clusters(
                &self.screen,
                cluster_count(&self.screen).saturating_sub(count),
            );
            self.log.push(format!("bs {count}"));
            Ok(())
        }

        fn send_text(&mut self, text: &str) -> Result<()> {
            assert!(
                self.latched.is_empty(),
                "text sent with {:?} still held",
                self.latched
            );
            self.screen.push_str(text);
            self.log.push(format!("type {text}"));
            Ok(())
        }

        fn send_paste(&mut self, text: &str) -> Result<()> {
            if self.fail_paste {
                anyhow::bail!("clipboard unavailable");
            }
            assert!(self.caps.paste, "paste reached a backend without one");
            self.screen.push_str(text);
            self.log.push(format!("paste {text}"));
            Ok(())
        }
    }

    #[test]
    fn a_replace_never_leaves_a_latched_modifier_behind() {
        let mut keyboard = FakeKeyboard::holding(&["ctrl", "alt"]);
        keyboard.screen = "hello world".into();
        Plan::replace("hello world", "hello there", typing())
            .run(&mut keyboard)
            .unwrap();
        assert!(
            keyboard.latched.is_empty(),
            "modifiers still held after a replace: {:?}",
            keyboard.latched
        );
        assert_eq!(keyboard.screen, "hello there");
        assert_eq!(keyboard.log, ["lift", "bs 5", "type there"]);
    }

    #[test]
    fn the_latched_modifier_check_has_teeth() {
        // Same events with the lift removed: the fake keyboard must object, or
        // the test above proves nothing.
        let mut keyboard = FakeKeyboard::holding(&["ctrl"]);
        let bare = Plan {
            actions: vec![Action::Backspace(1)],
        };
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            bare.run(&mut keyboard).unwrap();
        }))
        .is_err());
    }

    #[test]
    fn every_plan_lifts_before_it_sends_anything() {
        let mut keyboard = FakeKeyboard::holding(&["shift"]);
        keyboard.caps = Capabilities { paste: true };
        keyboard.screen = "a".repeat(300);
        Plan::replace(&"a".repeat(300), &"b".repeat(300), pasting())
            .run(&mut keyboard)
            .unwrap();
        assert!(keyboard.latched.is_empty());
        assert_eq!(keyboard.screen, "b".repeat(300));
        assert_eq!(keyboard.log[0], "lift");
        assert!(keyboard.log.last().unwrap().starts_with("paste"));
    }

    #[test]
    fn a_backend_without_a_pasteboard_refuses_a_paste_action() {
        // Unreachable through PlanOpts::from_capabilities; asserted anyway so
        // the refusal cannot rot into a silent type-instead fallback.
        struct Bare;
        impl KeyboardSink for Bare {
            fn lift_modifiers(&mut self) -> Result<()> {
                Ok(())
            }
            fn send_backspaces(&mut self, _: usize) -> Result<()> {
                Ok(())
            }
            fn send_text(&mut self, _: &str) -> Result<()> {
                Ok(())
            }
        }
        let plan = Plan {
            actions: vec![Action::Paste("x".into())],
        };
        let err = plan.run(&mut Bare).unwrap_err();
        assert!(err.to_string().contains("pasteboard"));
    }

    #[test]
    fn a_backend_failure_stops_the_plan() {
        let mut keyboard = FakeKeyboard {
            caps: Capabilities { paste: true },
            fail_paste: true,
            ..FakeKeyboard::default()
        };
        keyboard.screen = "old".into();
        let err = Plan::replace("old", &"n".repeat(300), pasting())
            .run(&mut keyboard)
            .unwrap_err();
        assert!(err.to_string().contains("clipboard unavailable"));
        // The backspaces landed and the paste did not: the caller has to treat
        // the screen as unknown, which is why `Injector::replace_last` clears
        // its record on any error.
        assert_eq!(keyboard.screen, "");
    }

    // --- the round trip ------------------------------------------------------

    #[test]
    fn simulate_matches_the_fake_keyboard() {
        let mut keyboard = FakeKeyboard {
            screen: "hello world".into(),
            ..FakeKeyboard::default()
        };
        let plan = Plan::replace("hello world", "hello there", typing());
        plan.run(&mut keyboard).unwrap();
        assert_eq!(keyboard.screen, plan.simulate("hello world"));
    }

    /// xorshift64: a fixed-seed generator, so a failure reproduces exactly
    /// rather than haunting CI once a month. The assertions print the iteration
    /// number to find it by. No new dependency; a property test is not worth
    /// growing the build for.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }

        /// A string built from pieces that stress every counting mistake:
        /// multibyte, combining marks, ZWJ sequences, flags, CJK and RTL.
        fn text(&mut self, max_pieces: usize) -> String {
            const PIECES: &[&str] = &[
                "a",
                "b",
                " ",
                "z",
                "?",
                "\n",
                "é",
                "e\u{301}",
                "n\u{303}",
                "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}",
                "👍🏽",
                "🇯🇵",
                "😀",
                "你",
                "好",
                "世",
                "שלום",
                "ع",
                "\u{300}",
                "\u{200D}",
            ];
            let n = self.below(max_pieces + 1);
            (0..n).map(|_| PIECES[self.below(PIECES.len())]).collect()
        }
    }

    #[test]
    fn applying_a_plan_to_old_always_produces_new() {
        // The property the whole crate rests on: whatever the pair, replaying
        // the emitted events over text ending in `old` leaves text ending in
        // `new`, and never touches what came before it.
        const PREFIX: &str = "USER'S OWN TEXT 你好 ";
        let mut rng = Rng(0x5EED_1234_ABCD_0001);
        let mut merging = 0;
        for i in 0..4_000 {
            let old = rng.text(8);
            let new = rng.text(8);
            for opts in [typing(), pasting()] {
                let plan = Plan::replace(&old, &new, opts);
                let got = plan.simulate(&format!("{PREFIX}{old}"));
                assert!(
                    got.starts_with(PREFIX),
                    "iteration {i}: ate the user's own text. old={old:?} new={new:?} \
                     plan={:?} got={got:?}",
                    plan.actions()
                );
                if joins_the_cluster_before(&old) {
                    // Documented in `a_head_that_merges_backwards_is_left_alone`:
                    // exactness is unreachable here, safety is not.
                    merging += 1;
                    continue;
                }
                assert_eq!(
                    got,
                    format!("{PREFIX}{new}"),
                    "iteration {i}: old={old:?} new={new:?} plan={:?}",
                    plan.actions()
                );
            }
        }
        assert!(merging > 0, "the generator stopped producing merging heads");
    }

    #[test]
    fn a_head_that_merges_backwards_is_left_alone() {
        // `old` is a lone combining grave. On screen it has already fused with
        // the space before it, which we did not type. One press would delete
        // both; zero presses leaves a visible mark. We choose zero.
        let spared = Plan::replace("\u{300}", "x", typing());
        assert_eq!(spared.actions(), [Action::LiftModifiers, typed("x")]);
        assert_eq!(spared.simulate("PRE \u{300}"), "PRE \u{300}x");

        // The rest of the run is still deleted: only the fused head is spared.
        let partial = Plan::replace("\u{300}abc", "", typing());
        assert_eq!(
            partial.actions(),
            [Action::LiftModifiers, Action::Backspace(3)]
        );
        assert_eq!(partial.simulate("PRE \u{300}abc"), "PRE \u{300}");

        // A mark in the middle of our own text has a real boundary in front of
        // it and is deleted normally.
        assert_eq!(
            plan("a\u{300}", ""),
            [Action::LiftModifiers, Action::Backspace(1)]
        );
    }

    #[test]
    fn a_planned_replace_round_trips_through_the_record() {
        // Same property, entered the way callers will: by char count.
        let mut rng = Rng(0x5EED_1234_ABCD_0002);
        for i in 0..2_000 {
            let old = rng.text(6);
            let new = rng.text(6);
            let n = rng.below(old.chars().count() + 3);

            let mut record = Typed::new();
            record.record(&old);
            let plan = record.plan_replace(n, &new, typing());
            let screen = plan.simulate(&format!("PRE {old}"));

            assert!(
                screen.starts_with("PRE "),
                "iteration {i}: ate the user's text"
            );
            // Unconditional: the record is updated by replaying the plan, so it
            // tracks the screen even where the plan could not be exact.
            assert_eq!(
                screen,
                format!("PRE {}", record.known()),
                "iteration {i}: old={old:?} n={n} new={new:?}"
            );
        }
    }
}
