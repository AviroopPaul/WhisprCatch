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

use anyhow::{Context, Result};

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
/// have to fail on, and a backend that cannot release a held modifier never
/// receives a [`Action::Backspace`] it could not make safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    /// The backend can put text on the clipboard and paste it.
    pub paste: bool,
    /// [`KeyboardSink::lift_modifiers`] genuinely releases a modifier the user
    /// is holding, rather than reporting success having done nothing.
    ///
    /// This is the whole of #77. On Wayland there is no XTEST connection, so
    /// the lift is a silent no-op and the Backspace that follows it arrives
    /// with the user's push-to-talk Ctrl still physically down — where it is
    /// delete-*word*, not delete-character. Five presses take out a sentence.
    ///
    /// The flag is honoured by [`Plan::replace`], not by callers: a planner
    /// that refuses to emit an unsafe action cannot be forgotten, whereas a
    /// bool that three future callers each have to remember to check can.
    pub can_lift_modifiers: bool,
}

impl Capabilities {
    /// Keystrokes only, and a held modifier really can be released first. X11
    /// and macOS. See the crate docs for why there is no pasteboard path yet.
    pub const TYPING_ONLY: Self = Self {
        paste: false,
        can_lift_modifiers: true,
    };

    /// Keystrokes only, with no way to release a held modifier: Wayland. A
    /// [`Plan::replace`] for a sink like this declines rather than backspacing
    /// into the user's push-to-talk key.
    pub const NO_LIFT: Self = Self {
        paste: false,
        can_lift_modifiers: false,
    };
}

/// What a backend in *this* crate reports, given whether its lift works.
///
/// The one place the mapping lives, so that "the injector answers honestly
/// about Wayland" is a claim a test can make without a display server — the
/// method on `Injector` is then a single call with nothing left to get wrong.
/// `paste` is false here and everywhere until #68 lands.
pub const fn capabilities_for(can_lift_modifiers: bool) -> Capabilities {
    Capabilities {
        paste: false,
        can_lift_modifiers,
    }
}

/// Planner knobs, derived from a backend's [`Capabilities`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanOpts {
    /// Type runs longer than this are pasted instead. `None` always types.
    pub paste_threshold: Option<usize>,
    /// Whether [`Action::Backspace`] may be emitted at all. False when the sink
    /// cannot lift a held modifier — see [`Capabilities::can_lift_modifiers`].
    pub can_backspace: bool,
}

impl PlanOpts {
    /// Never paste; backspaces are allowed.
    pub fn typing_only() -> Self {
        Self {
            paste_threshold: None,
            can_backspace: true,
        }
    }

    /// A sink that can neither paste nor lift a held modifier, so no backspace
    /// run may be planned for it at all.
    pub fn no_backspace() -> Self {
        Self {
            paste_threshold: None,
            can_backspace: false,
        }
    }

    pub fn from_capabilities(caps: Capabilities) -> Self {
        Self {
            paste_threshold: caps.paste.then_some(PASTE_THRESHOLD),
            can_backspace: caps.can_lift_modifiers,
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
    /// What this backend can do.
    ///
    /// The default is deliberately the *most* restricted answer: no pasteboard
    /// and no working lift. A backend that forgets to override it loses
    /// `replace_last`, which surfaces as a loud error; the opposite default
    /// would lose the user's text silently.
    fn capabilities(&self) -> Capabilities {
        Capabilities::NO_LIFT
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
    /// Set when the replace was *refused* rather than found to be unnecessary.
    /// Both are empty plans, and the difference matters: nothing to do is a
    /// success, and "this sink cannot do it safely" is something the caller has
    /// to hear about. See [`Plan::is_declined`].
    declined: bool,
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
    /// Some cases are deliberately under-deleted, and one class is not caught
    /// at all — see [`joins_the_cluster_before`], which is where the limits of
    /// planning without the *preceding* text are written down. #76 threads that
    /// context in and removes both the special case and the gap.
    ///
    /// # Refusing
    ///
    /// When the plan would need a Backspace run and
    /// [`PlanOpts::can_backspace`] is false, this returns
    /// [`Plan::declined`] — no actions at all, not a half-plan that types the
    /// new text on top of the old. The sink cannot release the push-to-talk
    /// modifier the user is still holding, so every press would arrive as
    /// Ctrl/⌥+Backspace, i.e. delete-word (#77). Deciding that here rather than
    /// in each caller is the point: a planner that will not emit an unsafe
    /// action cannot be forgotten by a future caller.
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
            return Self {
                actions,
                declined: false,
            };
        }
        if backspaces > 0 && !opts.can_backspace {
            return Self::declined();
        }
        actions.push(Action::LiftModifiers);
        if backspaces > 0 {
            actions.push(Action::Backspace(backspaces));
        }
        push_insert(&mut actions, fresh, opts);
        Self {
            actions,
            declined: false,
        }
    }

    /// The refusal: a plan that sends nothing because it could not be made
    /// safe. Distinct from an empty plan, which means there was nothing to do.
    fn declined() -> Self {
        Self {
            actions: Vec::new(),
            declined: true,
        }
    }

    /// True when this plan sends nothing *because it refused to*. A caller that
    /// only checks [`Plan::is_empty`] would mistake a refusal for success and
    /// carry on as if the screen had been corrected.
    pub fn is_declined(&self) -> bool {
        self.declined
    }

    /// The events that put `text` at the cursor with nothing to take back.
    ///
    /// This exists so there is exactly **one** place that decides how text gets
    /// inserted. `type_text` is the path that carries essentially all traffic
    /// today, and #68 will add a rule to that decision (paste, never type, when
    /// the text contains a newline). With two decision sites the rule gets
    /// written twice, and the second one gets forgotten.
    ///
    /// It emits no [`Action::LiftModifiers`], which is a deliberate difference
    /// from [`Plan::replace`] rather than an oversight. A type run under a held
    /// modifier loses keystrokes to shortcuts; a *backspace* run under one
    /// deletes whole words. The typing path is also the hot one — a lift costs
    /// two synchronous X round-trips, on every streaming pass — and its callers
    /// already lift explicitly via `Injector::lift_key` at the point where they
    /// know the push-to-talk key is still down.
    ///
    /// It also never declines. #50 asked whether the append path carries the
    /// same Wayland exposure as the replace path, and the answer is: the same
    /// *hazard*, a different *cost*. A type run under a held modifier turns
    /// characters into shortcuts and loses them; a backspace run under one
    /// deletes words the user wrote. Refusing to type would take live
    /// streaming away from every Wayland user to avoid a failure they already
    /// live with, so the refusal is confined to the destructive path.
    pub fn type_text(text: &str, opts: PlanOpts) -> Self {
        let mut actions = Vec::new();
        push_insert(&mut actions, text, opts);
        Self {
            actions,
            declined: false,
        }
    }

    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Send the plan. Stops at the first failure: carrying on after the
    /// backspaces failed would be worse than stopping.
    ///
    /// # Putting text back
    ///
    /// Stopping is right for a failure *before* anything was deleted, and
    /// wrong for one after. If the Backspace run lands and the insert then
    /// fails, the user is looking at a hole where their sentence was, and the
    /// error alone does not tell the caller that — so this makes one attempt to
    /// type the text back before returning the original error.
    ///
    /// The retry is `send_text`, never `send_paste`, because the most likely
    /// way an insert fails is the clipboard being unavailable, and re-trying
    /// the thing that just failed is not a recovery. Its own result is
    /// deliberately discarded: it is a best effort at limiting damage, and the
    /// caller still has to hear about the failure that started it.
    ///
    /// Nothing is deleted on the pre-#50 paths, so this whole case is new with
    /// the streaming replace.
    pub fn run(&self, keyboard: &mut dyn KeyboardSink) -> Result<()> {
        let mut deleted = false;
        for action in &self.actions {
            let sent = match action {
                Action::LiftModifiers => keyboard.lift_modifiers(),
                Action::Backspace(n) => keyboard.send_backspaces(*n),
                Action::Type(text) => keyboard.send_text(text),
                Action::Paste(text) => keyboard.send_paste(text),
            };
            if let Err(e) = sent {
                if deleted {
                    if let Some(text) = self.inserted_text() {
                        log::warn!(
                            "an insert failed after {} chars had been deleted; \
                             typing them back",
                            text.chars().count()
                        );
                        let _ = keyboard.send_text(text);
                    }
                }
                return Err(e);
            }
            if matches!(action, Action::Backspace(_)) {
                deleted = true;
            }
        }
        Ok(())
    }

    /// The text this plan inserts, if any. One action at most ever inserts.
    fn inserted_text(&self) -> Option<&str> {
        self.actions.iter().find_map(|a| match a {
            Action::Type(t) | Action::Paste(t) => Some(t.as_str()),
            _ => None,
        })
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

/// Whether a finished send may be added to the record of what is on screen.
///
/// `Ok(())` is not evidence that anything arrived. macOS Secure Input drops
/// synthetic keystrokes and every call still reports success, so a record taken
/// on trust describes a screen that does not exist — and the next replace
/// backspaces through the difference, into text the user wrote.
///
/// Secure Input is sampled **twice**, before the send and after, because
/// another process can switch it on or off while we are mid-call. One sample
/// alone leaves a hole at each end:
///
/// | before | after | one sample (after) | both samples |
/// |---|---|---|---|
/// | off | on  | forget ✓ | forget ✓ |
/// | on  | off | record ✗ — the events were dropped | forget ✓ |
///
/// Either being true is enough to refuse. The costs are not symmetric:
/// forgetting wrongly means a later replace types instead of replacing, and
/// recording wrongly means it deletes the user's writing.
pub fn should_record(sent_ok: bool, secure_before: bool, secure_after: bool) -> bool {
    sent_ok && !secure_before && !secure_after
}

/// The whole of `Injector::replace_last`, for any sink and any record.
///
/// Lives here rather than in the platform layer because every branch of it is a
/// decision, and decisions in this crate are testable ones: `Injector` needs a
/// display server to construct, so a refusal written there could only ever be
/// checked by reading it. Three of the four branches below — the Secure Input
/// refusal, the decline, and dropping the record after a failed send — are
/// exactly the kind of guard that survives a mutation run when it is stranded
/// on an untestable type.
///
/// `secure` is [`crate::secure_input_active`] sampled by the caller.
pub fn replace_recorded(
    keyboard: &mut dyn KeyboardSink,
    typed: &mut Typed,
    n_chars: usize,
    new_text: &str,
    secure: bool,
) -> Result<()> {
    if secure {
        // Dropping the record here is not just caution about this call. Secure
        // Input drops synthetic keystrokes silently, so any earlier send may
        // have reported success while nothing reached the screen — the record
        // could already describe text that does not exist, and backspacing
        // against it would eat the user's.
        typed.forget();
        anyhow::bail!(
            "secure input is enabled (a password field has focus); \
             refusing to replace text"
        );
    }
    let opts = PlanOpts::from_capabilities(keyboard.capabilities());
    let plan = typed.plan_replace(n_chars, new_text, opts);
    if plan.is_declined() {
        // Not a failure to send: a refusal to plan. The record still describes
        // the screen, so leave it alone and let the caller pick a path that
        // does not delete.
        anyhow::bail!(
            "cannot take back typed text on this display server: a modifier you are \
             still holding cannot be released, so Backspace would delete whole words"
        );
    }
    if plan.is_empty() {
        return Ok(());
    }
    if let Err(e) = plan.run(keyboard) {
        typed.forget();
        return Err(e).context("replacing typed text");
    }
    Ok(())
}

/// Append the one action that inserts `text`, if there is any text. The single
/// place the type-or-paste threshold is applied, for both entry points.
fn push_insert(actions: &mut Vec<Action>, text: &str, opts: PlanOpts) {
    if text.is_empty() {
        return;
    }
    let paste = opts
        .paste_threshold
        .is_some_and(|limit| text.chars().count() > limit);
    actions.push(if paste {
        Action::Paste(text.to_string())
    } else {
        Action::Type(text.to_string())
    });
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
/// the cluster count goes up.
///
/// # What this does not catch
///
/// The probe prepends `'a'`, so it rules out only the **context-free** joins:
/// UAX #29 GB9 and GB9a, a combining mark or a ZWJ after anything at all. Any
/// rule whose left context is not a plain base character — Hangul jamo (GB6 /
/// GB7), emoji-ZWJ (GB11), regional indicators (GB12 / GB13), CR (GB3), Indic
/// conjuncts (GB9c), Prepend (GB9b) — fuses across the seam undetected, and the
/// plan then counts one cluster too many. That is the over-deletion direction:
/// the press takes text the user wrote.
///
/// **Treat that list as examples, not as the set.** GB9b in particular puts no
/// constraint on our side at all — a Prepend character fuses with whatever
/// follows it — so enumerating what our text may start with cannot make this
/// probe correct. Nor is any of it hypothetical: our half of GB3 is a newline,
/// which the snippets transform (#67) injects today.
///
/// This is not fixable from inside this function, which sees `s` alone while
/// the answer depends on text this crate never gets. The fix is to thread the
/// preceding characters into [`Typed::plan_replace`], which deletes this
/// special case rather than extending it — **#76**. Until then the guarantee in
/// the crate docs is a bound on the *count*, not a promise about whose
/// characters those presses take.
fn joins_the_cluster_before(s: &str) -> bool {
    !s.is_empty() && cluster_count(&format!("a{s}")) == cluster_count(s)
}

/// The tail of what the injector typed, so a `replace_last(n_chars, ..)` can be
/// resolved into actual text.
///
/// A char count on its own is not enough to plan a replace: cluster boundaries,
/// the common prefix and the paste decision all need the characters themselves.
/// The record is bounded ([`TYPED_MEMORY_CHARS`]) and never counts as consent —
/// [`Typed::plan_replace`] will not emit more backspaces than there are
/// clusters in the recorded text, so a caller asking for more than we typed
/// gets its request shortened rather than the user's text.
///
/// That bounds the count. It is *not* a promise that those presses only remove
/// our characters: where our text begins is only a real cluster boundary if the
/// character in front of it does not join ours, and three UAX #29 rules can
/// join across a seam this crate cannot see. See [`joins_the_cluster_before`]
/// for the exact list and #76 for the fix.
///
/// The record is also only as good as our knowledge that the text arrived.
/// [`Typed::forget`] exists for the cases where it did not — a failed
/// injection, or macOS Secure Input silently swallowing keystrokes that were
/// reported as sent.
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
    ///
    /// A plan the sink refused ([`Plan::is_declined`]) leaves the record
    /// untouched, which falls out of replaying it rather than being a special
    /// case: a plan with no actions changes no text, so the screen still reads
    /// what it read before.
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
        PlanOpts::from_capabilities(Capabilities {
            paste: true,
            can_lift_modifiers: true,
        })
    }

    /// A sink that cannot release a held modifier: Wayland.
    fn no_lift() -> PlanOpts {
        PlanOpts::from_capabilities(Capabilities::NO_LIFT)
    }

    /// Build a plan out of raw actions, for the few tests that need to assert
    /// what `run` does with events the planner would not have emitted.
    fn raw(actions: Vec<Action>) -> Plan {
        Plan {
            actions,
            declined: false,
        }
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
    fn typing_is_planned_through_the_same_seam() {
        // One decision site: whatever `replace` would do to insert text, this
        // does too. #68 adds a rule here and must only add it once.
        assert_eq!(
            Plan::type_text("hello", typing()).actions(),
            [typed("hello")]
        );
        assert!(Plan::type_text("", typing()).is_empty());
        assert_eq!(Plan::type_text("hi", typing()).simulate("> "), "> hi");
    }

    #[test]
    fn typing_obeys_the_same_paste_threshold_as_replacing() {
        for len in [199, 200, 201] {
            let text = "x".repeat(len);
            let inserting = Plan::type_text(&text, pasting()).actions().to_vec();
            let replacing = Plan::replace("", &text, pasting()).actions().to_vec();
            // `replace` prepends the lift; the insert action itself must match.
            assert_eq!(inserting.last(), replacing.last(), "{len} chars");
        }
    }

    #[test]
    fn typing_does_not_lift_modifiers() {
        // Deliberate, and documented on `Plan::type_text`: this is the hot path
        // and its callers lift explicitly. If that changes, change it here and
        // let this test fail loudly rather than drifting.
        let actions = Plan::type_text("hello", typing()).actions().to_vec();
        assert!(!actions.contains(&Action::LiftModifiers));
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
            PlanOpts::from_capabilities(Capabilities {
                paste: true,
                can_lift_modifiers: true
            })
            .paste_threshold,
            Some(PASTE_THRESHOLD)
        );
    }

    // --- refusing to backspace into a held modifier (#77) --------------------

    #[test]
    fn capabilities_decide_whether_a_backspace_run_is_reachable() {
        assert!(PlanOpts::from_capabilities(Capabilities::TYPING_ONLY).can_backspace);
        assert!(!PlanOpts::from_capabilities(Capabilities::NO_LIFT).can_backspace);
        // The trait default is the restricted one: a backend that never says
        // what it can do loses `replace_last`, rather than losing the user's
        // text. Asserted through a sink that overrides nothing.
        struct Silent;
        impl KeyboardSink for Silent {
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
        assert!(!Silent.capabilities().can_lift_modifiers);
    }

    #[test]
    fn a_sink_that_cannot_lift_gets_no_backspace_run() {
        // The #77 hazard in one line: on Wayland `lift_modifiers` reports
        // success having lifted nothing, so these five presses would arrive as
        // Ctrl+Backspace — delete-word — into the user's own sentence.
        let plan = Plan::replace("hello world", "hello there", no_lift());
        assert_eq!(plan.actions(), []);
        assert!(plan.is_declined());
        // And it declines outright rather than typing the new text on top of
        // the old, which would read "hello worldthere".
        assert_eq!(plan.simulate("hello world"), "hello world");
    }

    #[test]
    fn a_sink_that_cannot_lift_still_appends() {
        // Only the destructive half is refused. Pure appends are the streaming
        // hot path and carry a different, lesser hazard — see `type_text`.
        let plan = Plan::replace("hello", "hello world", no_lift());
        assert_eq!(plan.actions(), [Action::LiftModifiers, typed(" world")]);
        assert!(!plan.is_declined());
        assert_eq!(plan.simulate("hello"), "hello world");

        // Typing never declines, whatever the sink can do.
        assert!(!Plan::type_text("hello", no_lift()).is_declined());
        assert_eq!(
            Plan::type_text("hello", no_lift()).actions(),
            [typed("hello")]
        );
    }

    #[test]
    fn nothing_to_do_is_not_a_refusal() {
        // Two empty plans that mean opposite things. `replace_last` reports one
        // as success and the other as an error, so they must stay tellable
        // apart.
        let nothing = Plan::replace("same", "same", no_lift());
        assert!(nothing.is_empty() && !nothing.is_declined());
        let refused = Plan::replace("same", "different", no_lift());
        assert!(refused.is_empty() && refused.is_declined());
    }

    #[test]
    fn no_pair_at_all_can_produce_a_backspace_without_a_lift() {
        // The guarantee stated over the whole torture corpus rather than over
        // the handful of pairs above: there is no old/new that slips a
        // Backspace past a sink that cannot lift.
        let mut rng = Rng(0x5EED_1234_ABCD_0003);
        let mut refusals = 0;
        for i in 0..4_000 {
            let old = rng.text(8);
            let new = rng.text(8);
            let plan = Plan::replace(&old, &new, no_lift());
            assert!(
                !plan
                    .actions()
                    .iter()
                    .any(|a| matches!(a, Action::Backspace(_))),
                "iteration {i}: backspace planned for a sink that cannot lift. \
                 old={old:?} new={new:?} plan={:?}",
                plan.actions()
            );
            // A refusal must send nothing at all, not a partial replace.
            if plan.is_declined() {
                refusals += 1;
                assert_eq!(plan.actions(), [], "iteration {i}: half a replace");
            } else {
                // Anything it *did* plan has to be the honest answer, i.e. the
                // same one a lifting sink would have got.
                assert_eq!(
                    plan.simulate(&old),
                    Plan::replace(&old, &new, typing()).simulate(&old),
                    "iteration {i}: appended a different result. old={old:?} new={new:?}"
                );
            }
        }
        assert!(refusals > 0, "the generator stopped producing deletions");
    }

    #[test]
    fn a_backend_reports_exactly_what_its_lift_can_do() {
        // #77's Done-when is "`can_lift_modifiers` exists and is honest per
        // backend". `Injector` needs a display server, so the honesty lives in
        // this mapping and the method is one call with nothing left to get
        // wrong. Reverting it to a constant has to delete this test too.
        assert!(capabilities_for(true).can_lift_modifiers);
        assert!(!capabilities_for(false).can_lift_modifiers);
        assert_eq!(capabilities_for(true), Capabilities::TYPING_ONLY);
        assert_eq!(capabilities_for(false), Capabilities::NO_LIFT);
        // No backend in the tree can paste, whatever else it can do (#68).
        for can_lift in [true, false] {
            assert!(!capabilities_for(can_lift).paste);
        }
        // And the mapping is what drives the planner, end to end.
        assert!(!PlanOpts::from_capabilities(capabilities_for(false)).can_backspace);
        assert!(Plan::replace(
            "hello world",
            "hello there",
            PlanOpts::from_capabilities(capabilities_for(false))
        )
        .is_declined());
    }

    // --- the policy `replace_last` applies -----------------------------------

    #[test]
    fn a_replace_refuses_outright_under_secure_input() {
        let mut keyboard = FakeKeyboard::default();
        let mut record = Typed::new();
        record.record("hello world");
        let err = replace_recorded(&mut keyboard, &mut record, 5, "there", true).unwrap_err();
        assert!(err.to_string().contains("secure input"));
        assert_eq!(keyboard.log, Vec::<String>::new(), "sent something anyway");
        // The record is dropped, not kept: under Secure Input an *earlier* send
        // may have reported success while nothing reached the screen.
        assert_eq!(record.known(), "");
    }

    #[test]
    fn a_replace_reports_a_refusal_rather_than_silently_doing_nothing() {
        // Without this the caller cannot tell "corrected" from "declined", and
        // #50 would believe the screen had been fixed when it had not.
        let mut keyboard = FakeKeyboard {
            caps: Capabilities::NO_LIFT,
            ..FakeKeyboard::default()
        };
        let mut record = Typed::new();
        record.record("hello world");
        let err = replace_recorded(&mut keyboard, &mut record, 5, "there", false).unwrap_err();
        assert!(err.to_string().contains("cannot take back typed text"));
        assert_eq!(keyboard.log, Vec::<String>::new());
        assert_eq!(record.known(), "hello world", "the screen did not change");
    }

    #[test]
    fn a_replace_that_has_nothing_to_do_is_a_success() {
        let mut keyboard = FakeKeyboard {
            caps: Capabilities::NO_LIFT,
            ..FakeKeyboard::default()
        };
        let mut record = Typed::new();
        record.record("same");
        assert!(replace_recorded(&mut keyboard, &mut record, 4, "same", false).is_ok());
        assert_eq!(keyboard.log, Vec::<String>::new());
    }

    #[test]
    fn a_replace_sends_and_records_when_everything_works() {
        let mut keyboard = FakeKeyboard {
            caps: Capabilities::TYPING_ONLY,
            screen: "hello world".into(),
            ..FakeKeyboard::default()
        };
        let mut record = Typed::new();
        record.record("hello world");
        replace_recorded(&mut keyboard, &mut record, 5, "there", false).unwrap();
        assert_eq!(keyboard.screen, "hello there");
        assert_eq!(record.known(), "hello there");
    }

    #[test]
    fn a_failed_replace_drops_the_record() {
        // The screen is now unknown, so the next replace must type rather than
        // delete. Keeping the record would count presses against a fiction.
        let mut keyboard = FakeKeyboard {
            caps: Capabilities::TYPING_ONLY,
            fail_text: true,
            screen: "hello world".into(),
            ..FakeKeyboard::default()
        };
        let mut record = Typed::new();
        record.record("hello world");
        assert!(replace_recorded(&mut keyboard, &mut record, 5, "there", false).is_err());
        assert_eq!(record.known(), "");
    }

    #[test]
    fn a_refused_replace_leaves_the_record_describing_the_screen() {
        // Nothing was sent, so the record must still say what is on screen. If
        // it were updated to the text we wanted, the *next* replace would be
        // counted against a screen that never existed.
        let mut typed_record = Typed::new();
        typed_record.record("hello world");
        let plan = typed_record.plan_replace(5, "there", no_lift());
        assert!(plan.is_declined());
        assert_eq!(typed_record.known(), "hello world");

        // And an append through the same record still works and is recorded.
        let plan = typed_record.plan_replace(0, "!", no_lift());
        assert!(!plan.is_declined());
        assert_eq!(typed_record.known(), "hello world!");
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
    fn a_send_is_only_recorded_when_nothing_could_have_swallowed_it() {
        // The whole truth table, because three of its four Secure Input rows
        // were wrong at some point in this PR's history. Row `(true, true,
        // false)` is the original bug: the events were dropped while Secure
        // Input was on, it went off mid-call, and a check placed only after the
        // send reads false and records text that never arrived.
        assert!(should_record(true, false, false), "the ordinary case");
        assert!(!should_record(false, false, false), "the send failed");
        assert!(!should_record(true, true, true), "swallowed throughout");
        assert!(!should_record(true, false, true), "came on mid-send");
        assert!(!should_record(true, true, false), "went off mid-send");
        assert!(!should_record(false, true, false), "failed and unsure");

        // Stated as the rule, so a future edit cannot satisfy the rows above by
        // accident: recording needs a successful send and no Secure Input at
        // either end.
        for sent_ok in [true, false] {
            for before in [true, false] {
                for after in [true, false] {
                    assert_eq!(
                        should_record(sent_ok, before, after),
                        sent_ok && !before && !after,
                        "({sent_ok}, {before}, {after})"
                    );
                }
            }
        }
    }

    #[test]
    fn text_that_never_landed_must_not_be_recorded() {
        // NB: this is a *specification* of the hazard, not a regression guard
        // for the fix. It exercises `Typed`, which was never the broken part,
        // and it passes against the code before the fix landed. The guard is
        // `a_send_is_only_recorded_when_nothing_could_have_swallowed_it`
        // above; what neither can reach is the wiring in `Injector::type_text`,
        // which needs a Mac and a password prompt (manual step 11).
        //
        // Why `Injector::type_text` forgets rather than records when macOS
        // Secure Input is on. The OS swallows synthetic keystrokes and still
        // reports success, so a record taken on trust describes a screen that
        // does not exist — and the next replace backspaces through the
        // difference, into text the user wrote.
        let mut poisoned = Typed::new();
        poisoned.record("SECRET"); // swallowed: never reached the screen
        poisoned.record("hi"); // Secure Input off again; this one landed
        let plan = poisoned.plan_replace(8, "bye", typing());
        assert_eq!(
            plan.simulate("user's own words hi"),
            "user's own bye",
            "six characters of the user's text destroyed — this is the bug"
        );

        // What the injector does instead: no record without evidence.
        let mut sound = Typed::new();
        sound.forget(); // the swallowed type
        sound.record("hi");
        let plan = sound.plan_replace(8, "bye", typing());
        assert_eq!(plan.simulate("user's own words hi"), "user's own words bye");
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
        fail_text: bool,
        /// Counts `send_text` calls including the recovery one, so a test can
        /// tell "typed the text back" from "never tried".
        text_attempts: usize,
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
            self.text_attempts += 1;
            if self.fail_text {
                anyhow::bail!("keyboard gone");
            }
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
        let bare = raw(vec![Action::Backspace(1)]);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            bare.run(&mut keyboard).unwrap();
        }))
        .is_err());
    }

    #[test]
    fn every_plan_lifts_before_it_sends_anything() {
        let mut keyboard = FakeKeyboard::holding(&["shift"]);
        keyboard.caps = Capabilities {
            paste: true,
            can_lift_modifiers: true,
        };
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
        let plan = raw(vec![Action::Paste("x".into())]);
        let err = plan.run(&mut Bare).unwrap_err();
        assert!(err.to_string().contains("pasteboard"));
    }

    #[test]
    fn a_backend_failure_stops_the_plan() {
        let mut keyboard = FakeKeyboard {
            caps: Capabilities {
                paste: true,
                can_lift_modifiers: true,
            },
            fail_paste: true,
            ..FakeKeyboard::default()
        };
        keyboard.screen = "old".into();
        let new = "n".repeat(300);
        let err = Plan::replace("old", &new, pasting())
            .run(&mut keyboard)
            .unwrap_err();
        assert!(err.to_string().contains("clipboard unavailable"));
        // The backspaces landed and the paste did not. Leaving it there would
        // hand the user a hole where their sentence was, so the text is typed
        // back — the caller still gets the error and still drops its record.
        assert_eq!(keyboard.screen, new, "deleted text was not put back");
        assert_eq!(keyboard.log.first().map(String::as_str), Some("lift"));
        assert_eq!(keyboard.log[1], "bs 3");
        assert!(
            keyboard.log.last().unwrap().starts_with("type"),
            "the recovery should type, never re-try the paste that just failed: {:?}",
            keyboard.log
        );
    }

    #[test]
    fn a_failed_insert_after_a_deletion_types_the_text_back() {
        // The new failure class this PR introduces: before #50 nothing was ever
        // deleted, so an injection failure could only ever lose text that was
        // never on screen. Now it can leave a hole.
        let mut keyboard = FakeKeyboard {
            fail_text: true,
            screen: "hello world".into(),
            ..FakeKeyboard::default()
        };
        let err = Plan::replace("hello world", "hello there", typing())
            .run(&mut keyboard)
            .unwrap_err();
        assert!(err.to_string().contains("keyboard gone"));
        // Two send_text attempts: the plan's, and the recovery.
        assert_eq!(keyboard.text_attempts, 2);
    }

    #[test]
    fn nothing_is_typed_back_when_nothing_was_deleted() {
        // A pure append that fails must not retry — there is no hole, and a
        // blind retry would risk typing the text twice.
        let mut keyboard = FakeKeyboard {
            fail_text: true,
            screen: "hello".into(),
            ..FakeKeyboard::default()
        };
        let err = Plan::replace("hello", "hello world", typing())
            .run(&mut keyboard)
            .unwrap_err();
        assert!(err.to_string().contains("keyboard gone"));
        assert_eq!(
            keyboard.text_attempts, 1,
            "retried a plan that deleted nothing"
        );
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
