//! Rolling-transcription state for a single utterance.
//!
//! While the key is held the daemon re-transcribes recent audio every few
//! hundred milliseconds and types whatever has settled. Doing that over the
//! *whole* utterance does not scale: pass cost grows with what has been said,
//! so past roughly forty seconds a pass costs more than the interval between
//! passes — the CPU pegs, the live text lags, and Moonshine refuses audio over
//! 64s outright.
//!
//! So the window is bounded. It grows until it hits [`MAX_WINDOW_SECS`], then
//! slides forward keeping [`KEEP_TAIL_SECS`] of overlap. Sliding means a pass
//! no longer starts at the beginning of the utterance, so the words it returns
//! are only partly new — [`Stream::advance`] re-anchors on the text already
//! typed to work out which.
//!
//! # Reconciling with the finished transcript
//!
//! The streaming passes type what the *model* said. The release pass runs the
//! deterministic cleanup chain (`wc-text`), which can delete, substitute and
//! reorder words the streaming passes already put on screen — none of the six
//! transforms is prefix-stable. Appending a tail to text that is already wrong
//! cannot produce the right answer, which is the bug in #50.
//!
//! [`plan_release`] is the whole decision, as a pure function: append when the
//! finished text still begins with what is on screen, and otherwise take the
//! streamed run back and type the finished text in its place. It needs no
//! audio, no model, no display server and no clock, so the case that used to
//! need a microphone and a stopwatch is an ordinary unit test.
//!
//! This lives apart from the daemon loop so it can be tested without a mic.

/// Longest stretch of audio a single streaming pass will transcribe. Sized so a
/// pass stays near the streaming interval on modest hardware (~0.6s on an M1).
pub const MAX_WINDOW_SECS: f32 = 15.0;
/// Audio retained when the window slides. This is the overlap the re-anchor has
/// to work with, so it needs to comfortably exceed the anchor length in words.
pub const KEEP_TAIL_SECS: f32 = 6.0;
/// Words at the tail of a hypothesis we refuse to commit — the model routinely
/// revises the most recent words on the next pass (LocalAgreement guard).
const GUARD_WORDS: usize = 2;
/// Longest run of words used to re-locate ourselves after the window slides.
const MAX_ANCHOR: usize = 12;
/// Shortest anchor we will trust. A one- or two-word anchor ("the", "and so")
/// matches almost anywhere and would resume in the wrong place.
const MIN_ANCHOR: usize = 3;
/// How far back the duplicate-trim will look. This has to cover a whole
/// window's worth of words, not just an anchor: when the anchor fails we replay
/// the window from its start, and the trim is what stops that being visible.
const MAX_TRIM: usize = 64;

pub fn split_words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

/// Compare loosely — a pass often punctuates or capitalises a word differently
/// from the one before it, and that shouldn't break alignment.
fn norm(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn normalized(words: &[String]) -> Vec<String> {
    words.iter().map(|w| norm(w)).collect()
}

/// Words of `hyp` agreed with `prev` (common prefix), minus the guard tail.
fn stable_prefix_len(prev: &[String], hyp: &[String]) -> usize {
    let lcp = prev
        .iter()
        .zip(hyp.iter())
        .take_while(|(a, b)| norm(a) == norm(b))
        .count();
    lcp.min(hyp.len().saturating_sub(GUARD_WORDS))
}

/// Where the already-typed text ends inside a freshly slid window.
///
/// `hyp` covers only the last few seconds, so the tail of `committed` should
/// appear near its end. Returns the index just past the match, or `None` when
/// no anchor is trustworthy — the caller must then assume nothing new rather
/// than risk repeating the window.
fn anchor_in_window(committed: &[String], hyp: &[String]) -> Option<usize> {
    if committed.is_empty() || hyp.is_empty() {
        return None;
    }
    let c = normalized(committed);
    let h = normalized(hyp);
    let min_k = MIN_ANCHOR.min(c.len());
    let max_k = c.len().min(MAX_ANCHOR).min(h.len());

    for k in (min_k..=max_k).rev() {
        let tail = &c[c.len() - k..];
        // latest match: the overlap sits at the end of the window, and
        // consuming as much of it as possible is what avoids re-typing
        if let Some(j) = (0..=h.len() - k).rev().find(|&j| &h[j..j + k] == tail) {
            return Some(j + k);
        }
    }
    None
}

/// Index in `final_words` to resume typing from, given what's already on screen.
///
/// Resuming at `committed.len()` assumes the final transcript opens with exactly
/// the words already typed. It doesn't: the final pass sees the whole utterance
/// and revises what the streaming passes guessed, and every revision before that
/// point shifts the index — which duplicates or drops text. Align on the words
/// themselves instead.
pub fn resume_at(committed: &[String], final_words: &[String]) -> usize {
    if committed.is_empty() {
        return 0;
    }
    let c = normalized(committed);
    let f = normalized(final_words);
    let max_k = c.len().min(MAX_ANCHOR).min(f.len());

    for k in (1..=max_k).rev() {
        let tail = &c[c.len() - k..];
        // Of all the places this anchor occurs, take the one nearest where we
        // expected to be. Never "latest wins": with committed "for the" against
        // "for the gut comedy for the grin" that would anchor on the second
        // occurrence and silently swallow "gut comedy for the".
        let best = (0..=f.len() - k)
            .filter(|&j| &f[j..j + k] == tail)
            .min_by_key(|&j| (j + k).abs_diff(c.len()));
        if let Some(j) = best {
            return j + k;
        }
    }
    committed.len().min(final_words.len())
}

/// How many leading words of `delta` merely repeat the tail of `committed`.
///
/// The window anchor is good but not perfect — a pass can transcribe the
/// overlap slightly differently and resume a few words early, which puts
/// "jumps over the" + "over the lazy dog" on screen. This is the backstop:
/// whatever the anchor decided, never emit text that simply continues where we
/// already are.
///
/// The trade-off is that genuinely repeated speech ("very very good") can lose
/// a repeat when it straddles a delta boundary. Visible duplication is the far
/// worse failure for dictation, so trimming wins.
fn overlap_with_committed(committed: &[String], delta: &[String]) -> usize {
    let c = normalized(committed);
    let d = normalized(delta);
    let max = c.len().min(d.len()).min(MAX_TRIM);
    (1..=max)
        .rev()
        .find(|&k| c[c.len() - k..] == d[..k])
        .unwrap_or(0)
}

/// Text to type for a run of new words, spaced against what came before.
pub fn join_delta(words: &[String], first: bool) -> String {
    let mut s = words.join(" ");
    if !first {
        s.insert(0, ' ');
    }
    s
}

/// What the release pass must send, once the streaming passes have already put
/// text on screen. Produced by [`plan_release`], executed by the daemon loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Release {
    /// Send nothing at all.
    Nothing,
    /// Type this at the cursor. Only ever adds characters, so it is safe
    /// whatever else has happened to the screen since we last looked.
    Append(String),
    /// Take `take_back` chars of *our own* typing back and put `text` there,
    /// via `Injector::replace_last`. The count is in chars because that is what
    /// `replace_last` takes; it counts grapheme clusters at the keyboard.
    Replace { take_back: usize, text: String },
}

/// Everything [`plan_release`] needs to decide. A struct because the answer
/// depends on six independent facts and a positional call of six arguments is
/// a bug waiting for someone to swap two bools.
#[derive(Debug, Clone, Copy)]
pub struct ReleaseInput<'a> {
    /// Exactly the text the streaming passes put on screen this utterance: the
    /// concatenation of every delta injected *without an error*, so it is our
    /// best account of what is actually there.
    pub streamed: &'a str,
    /// The word list those deltas came from. Runs ahead of `streamed` when an
    /// injection failed, which is why the two are not interchangeable.
    pub committed: &'a [String],
    /// The finished, polished transcript.
    pub final_text: &'a str,
    /// Whether the cleanup chain changed the model's words at all.
    pub rewritten: bool,
    /// `Injector::replaceable_chars` — how far back a replace can still reach.
    pub replaceable_chars: usize,
    /// Whether a replace is available at all: the sink can lift a held modifier
    /// (#77) *and* the wipe is affordable (#68). See `replace_available` in
    /// `main.rs`.
    pub can_replace: bool,
    /// Whether any key other than the push-to-talk key went down while it was
    /// held. When it did, the cursor is not where we left it and a replace
    /// counted from it would delete the user's own characters.
    pub user_typed: bool,
}

/// Decide what to send when the key comes up.
///
/// The cases, in the order they are tried:
///
/// 1. **Nothing streamed.** The release pass owns the whole utterance, so type
///    it verbatim — whitespace included, because a snippet (#67) can put a
///    newline in it that re-joining the words would flatten.
/// 2. **Nothing on screen needs taking back.** Every word already typed still
///    matches the start of the finished text, so append the rest. This is the
///    ordinary case with no cleanup enabled, and also the case where cleanup
///    only touched words the streaming passes had not reached.
/// 3. **Cleanup rewrote words that are already typed.** No tail appended to
///    them can spell the finished text, so take those words back and type the
///    finished text from that point. This is the case #50 exists for.
/// 4. **Anything else.** The splice that shipped: align on the words
///    (`resume_at`) and append the tail.
///
/// # How much is taken back
///
/// Only from the first word that genuinely changed — found with the same loose
/// comparison `resume_at` uses, so a pass that capitalised or punctuated a word
/// differently does not count as a change.
///
/// Comparing the two strings byte for byte instead looks stricter and is far
/// worse: the streaming window and the whole-utterance pass routinely disagree
/// about a sentence-initial capital, which puts the common prefix at zero and
/// turns a three-character cleanup edit into a wipe of the entire utterance.
///
/// The price is that words *before* the first real change keep the model's own
/// capitalisation and punctuation, so the screen can differ from the finished
/// transcript by exactly that much. That is not a new bargain — it is the same
/// one the append path has always made, now applied uniformly instead of the
/// replace path silently making a different one at a cost of hundreds of
/// keystrokes.
///
/// # When it refuses
///
/// Case 3 gives way to case 4 under four conditions, and every one of them
/// fails in the direction that only adds characters:
///
/// * `user_typed` — somebody else's keystroke landed while the key was held, so
///   the cursor has moved and a count from it would eat their characters.
/// * `can_replace` is false — either the display server cannot release the key
///   the user is holding (#77), or the wipe is not affordable yet (#68).
/// * `replaceable_chars` cannot cover the run. The injector's record is the
///   only thing bounding the presses, and it is dropped whenever we stop being
///   sure the text landed (a failed injection, macOS Secure Input).
pub fn plan_release(input: ReleaseInput<'_>) -> Release {
    let ReleaseInput {
        streamed,
        committed,
        rewritten,
        replaceable_chars,
        can_replace,
        user_typed,
        ..
    } = input;
    // Whitespace-only is nothing: the pre-#50 path guarded on the word list
    // being empty, so a transcript of two spaces typed nothing at all.
    let final_text = match input.final_text.trim().is_empty() {
        true => "",
        false => input.final_text,
    };

    if streamed.is_empty() {
        return match final_text.is_empty() {
            true => Release::Nothing,
            false => Release::Append(final_text.to_string()),
        };
    }

    // Where the text on screen stops agreeing with the finished text.
    let (kept, matched) = common_word_prefix(streamed, final_text);
    if kept == streamed.len() {
        // Everything typed still stands; only the tail is missing.
        return match final_text[matched..].is_empty() {
            true => Release::Nothing,
            false => Release::Append(final_text[matched..].to_string()),
        };
    }

    if rewritten {
        let take_back = streamed[kept..].chars().count();
        let refusal = if user_typed {
            Some("a key of the user's own went down while the hotkey was held")
        } else if !can_replace {
            Some("this session cannot take typed text back")
        } else if replaceable_chars < take_back {
            Some("the injector's record no longer covers what was typed")
        } else {
            None
        };
        match refusal {
            None => {
                return Release::Replace {
                    take_back,
                    text: final_text[matched..].to_string(),
                }
            }
            Some(why) => log::warn!(
                "text cleanup rewrote {take_back} chars that are already on screen and \
                 they cannot be taken back ({why}) — appending the tail instead, so \
                 what stays on screen keeps the model's own words"
            ),
        }
    }

    let words = split_words(final_text);
    let start = resume_at(committed, &words);
    match start >= words.len() {
        true => Release::Nothing,
        false => Release::Append(join_delta(&words[start..], false)),
    }
}

/// Byte offsets, into `streamed` and into `final_text`, just past the longest
/// run of leading words that match loosely.
///
/// `(streamed.len(), _)` means everything on screen still stands. `(0, 0)`
/// means nothing does. Loose is [`norm`] — the same comparison the rest of this
/// module aligns on, so re-capitalisation is not mistaken for a rewrite.
fn common_word_prefix(streamed: &str, final_text: &str) -> (usize, usize) {
    let mut ours = word_spans(streamed);
    let mut theirs = word_spans(final_text);
    let (mut kept, mut matched) = (0, 0);
    loop {
        let (Some(a), Some(b)) = (ours.next(), theirs.next()) else {
            return (kept, matched);
        };
        if norm(&streamed[a.0..a.1]) != norm(&final_text[b.0..b.1]) {
            return (kept, matched);
        }
        (kept, matched) = (a.1, b.1);
    }
}

/// `(start, end)` byte offsets of each whitespace-separated word.
fn word_spans(s: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    s.split_whitespace()
        .map(move |w| {
            let start = w.as_ptr() as usize - s.as_ptr() as usize;
            (start, start + w.len())
        })
        .collect::<Vec<_>>()
        .into_iter()
}

// ------------------------------------------------- driving the injector

/// The injector, as the dictation loop uses it.
///
/// `wc_inject` already splits its decisions (`plan.rs`, pure) from its platform
/// code, and this is the same split one level up: the *sequencing* — forget the
/// record when an utterance starts, note text only after it lands, replace or
/// append when the key comes up — is decided here and tested against a fake,
/// instead of living in a loop that needs a microphone to enter.
///
/// Three of the guards this issue added sat in that loop at first and no test
/// could see any of them.
pub trait TextSink {
    fn type_text(&mut self, text: &str) -> anyhow::Result<()>;
    fn replace_last(&mut self, n_chars: usize, text: &str) -> anyhow::Result<()>;
    fn forget_typed(&mut self);
    fn replaceable_chars(&self) -> usize;
}

/// Start an utterance.
///
/// The record has to go. Between utterances the cursor can be anywhere — a
/// click, another window, the user's own typing — and the injector can see none
/// of it, so nothing it typed last time is ours to take back now. Without this
/// a replace would happily count presses against a previous utterance's text in
/// a different application.
pub fn begin_utterance(sink: &mut dyn TextSink, stream: &mut Stream) {
    stream.reset();
    sink.forget_typed();
}

/// Type one streaming delta, and note it **only if it landed**.
///
/// The order is the whole point. `Stream::advance` has already committed these
/// words, but committing is a claim about the transcript and `typed` is a claim
/// about the screen; marking before the send would make the release pass count
/// backspaces against text that a failed injection never put there.
pub fn stream_delta(sink: &mut dyn TextSink, stream: &mut Stream, delta: &[String]) -> String {
    let text = join_delta(delta, stream.nothing_typed());
    match sink.type_text(&text) {
        Ok(()) => {
            stream.mark_typed(&text);
            text
        }
        Err(e) => {
            log::error!("streaming injection failed: {e:#}");
            String::new()
        }
    }
}

/// Reconcile the finished transcript with what streaming typed, and send it.
///
/// Returns the plan it carried out, so the caller can log it, and the error if
/// the send failed.
pub fn finish_utterance(
    sink: &mut dyn TextSink,
    stream: &Stream,
    final_text: &str,
    rewritten: bool,
    can_replace: bool,
    user_typed: bool,
) -> (Release, anyhow::Result<()>) {
    let release = plan_release(ReleaseInput {
        streamed: stream.typed(),
        committed: stream.committed(),
        final_text,
        rewritten,
        replaceable_chars: sink.replaceable_chars(),
        can_replace,
        user_typed,
    });
    let sent = match &release {
        Release::Nothing => Ok(()),
        Release::Append(t) => sink.type_text(t),
        Release::Replace { take_back, text } => sink.replace_last(*take_back, text),
    };
    (release, sent)
}

#[derive(Default)]
pub struct Stream {
    committed: Vec<String>,
    prev_hyp: Vec<String>,
    /// Start of the current window, as an index into captured samples.
    window_start: usize,
    /// How many words of the current window have already been typed.
    committed_in_window: usize,
    /// Set when the window has just moved and we don't yet know where we are.
    needs_anchor: bool,
    /// Exactly the text the streaming passes have put on screen this utterance.
    ///
    /// Not derivable from `committed`: a delta is committed before it is
    /// injected, so a failed injection leaves a word committed that never
    /// reached the screen. The release pass has to know what is *there*, not
    /// what we meant to put there, because it may have to take it back.
    typed: String,
    /// Window bounds in seconds. Configurable so the streaming harness can
    /// measure the unbounded behaviour this replaced; production uses the
    /// module constants.
    max_window_secs: f32,
    keep_tail_secs: f32,
}

impl Stream {
    pub fn new() -> Self {
        Self::with_window(MAX_WINDOW_SECS, KEEP_TAIL_SECS)
    }

    pub fn with_window(max_window_secs: f32, keep_tail_secs: f32) -> Self {
        Self {
            max_window_secs,
            keep_tail_secs,
            ..Default::default()
        }
    }

    pub fn reset(&mut self) {
        *self = Self::with_window(self.max_window_secs, self.keep_tail_secs);
    }

    pub fn committed(&self) -> &[String] {
        &self.committed
    }

    pub fn window_start(&self) -> usize {
        self.window_start
    }

    /// Whether the utterance has put anything on screen yet, so the caller
    /// knows whether to prefix a space. Derived from [`Stream::typed`] rather
    /// than tracked alongside it: two fields that must agree eventually do not.
    pub fn nothing_typed(&self) -> bool {
        self.typed.is_empty()
    }

    /// Exactly what the streaming passes have typed this utterance.
    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// Move the window forward if it has outgrown the cap. `armed` and the
    /// return value are in captured samples, i.e. at the device rate.
    pub fn maybe_slide(&mut self, armed: usize, rate: u32) -> bool {
        let max = (self.max_window_secs * rate as f32) as usize;
        if armed.saturating_sub(self.window_start) <= max {
            return false;
        }
        let keep = (self.keep_tail_secs * rate as f32) as usize;
        self.window_start = armed.saturating_sub(keep);
        self.prev_hyp.clear();
        self.committed_in_window = 0;
        self.needs_anchor = true;
        true
    }

    /// Feed the hypothesis for the current window; returns the words to type.
    pub fn advance(&mut self, hyp: Vec<String>) -> Vec<String> {
        if self.needs_anchor {
            // Re-locate ourselves in the new window.
            //
            // When that fails, start from the beginning of the window and let
            // `overlap_with_committed` strip whatever turns out to be a repeat.
            // The opposite fallback — assuming the window is already typed —
            // loses everything in it, and nothing downstream can recover it:
            // the final pass only ever *appends* its tail, so a hole in the
            // middle is permanent. Duplicated words are recoverable by the
            // reader; deleted words are not.
            self.committed_in_window = match anchor_in_window(&self.committed, &hyp) {
                Some(i) => i,
                None => {
                    log::debug!(
                        "no anchor in a {}-word window after sliding; \
                         replaying it and trimming any repeat",
                        hyp.len()
                    );
                    0
                }
            };
            self.needs_anchor = false;
            self.prev_hyp = hyp;
            // no previous hypothesis in this window yet, so nothing has settled
            return Vec::new();
        }

        let stable = stable_prefix_len(&self.prev_hyp, &hyp);
        let mut out = Vec::new();
        if stable > self.committed_in_window {
            out = hyp[self.committed_in_window..stable].to_vec();
            // Backstop against an anchor that resumed a little early.
            let dup = overlap_with_committed(&self.committed, &out);
            if dup > 0 {
                log::debug!("trimmed {dup} duplicated word(s) at a window seam");
                out.drain(..dup);
            }
            self.committed.extend(out.iter().cloned());
            self.committed_in_window = stable;
        }
        self.prev_hyp = hyp;
        out
    }

    /// Note a delta that reached the screen. Call it only after the injection
    /// succeeded — the release pass may have to take this text back, and it can
    /// only do that safely for text that is really there.
    pub fn mark_typed(&mut self, text: &str) {
        self.typed.push_str(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }
    const RATE: u32 = 16_000;
    fn secs(n: f32) -> usize {
        (n * RATE as f32) as usize
    }

    // ---- resume_at (final splice) ----------------------------------------

    #[test]
    fn resumes_after_the_committed_tail() {
        let committed = w("the whole emotional spectrum");
        let final_words = w("he covers the whole emotional spectrum drama for the gut");
        assert_eq!(resume_at(&committed, &final_words), 6);
    }

    #[test]
    fn survives_the_final_pass_rewording_earlier_text() {
        // streaming heard "in a fellow", the final pass corrected it to
        // "in othello" — same word count, different words, which is exactly
        // what broke the old index splice
        let committed = w("jealousy in a fellow is invisible");
        let final_words = w("jealousy in othello is invisible but utterly destructive");
        assert_eq!(resume_at(&committed, &final_words), 5);
    }

    #[test]
    fn ignores_case_and_punctuation_when_aligning() {
        let committed = w("covers the whole emotional spectrum");
        let final_words = w("Covers the whole emotional spectrum. Drama for the gut");
        assert_eq!(resume_at(&committed, &final_words), 5);
    }

    #[test]
    fn anchors_a_repeated_phrase_nearest_the_expected_position() {
        let committed = w("for the");
        let final_words = w("for the gut comedy for the grin");
        assert_eq!(resume_at(&committed, &final_words), 2);
    }

    #[test]
    fn anchors_late_when_that_is_where_we_actually_are() {
        let committed = w("for the gut comedy for the");
        let final_words = w("for the gut comedy for the grin");
        assert_eq!(resume_at(&committed, &final_words), 6);
    }

    #[test]
    fn nothing_committed_types_everything() {
        assert_eq!(resume_at(&[], &w("a b c")), 0);
    }

    #[test]
    fn final_shorter_than_committed_does_not_panic() {
        assert_eq!(resume_at(&w("one two three four five"), &w("five")), 1);
    }

    // ---- windowing --------------------------------------------------------

    #[test]
    fn window_does_not_slide_before_the_cap() {
        let mut s = Stream::new();
        assert!(!s.maybe_slide(secs(MAX_WINDOW_SECS - 1.0), RATE));
        assert_eq!(s.window_start(), 0);
    }

    #[test]
    fn window_slides_once_past_the_cap_and_keeps_the_tail() {
        let mut s = Stream::new();
        let armed = secs(MAX_WINDOW_SECS + 5.0);
        assert!(s.maybe_slide(armed, RATE));
        assert_eq!(s.window_start(), armed - secs(KEEP_TAIL_SECS));
        // and not again until it has grown past the cap from the new start
        assert!(!s.maybe_slide(armed + secs(1.0), RATE));
    }

    #[test]
    fn window_cost_stays_bounded_however_long_the_utterance() {
        let mut s = Stream::new();
        let max = secs(MAX_WINDOW_SECS);
        for i in 1..=600 {
            let armed = secs(0.5) * i; // half a second per pass, five minutes
            s.maybe_slide(armed, RATE);
            assert!(
                armed - s.window_start() <= max,
                "window grew to {} samples at {}s",
                armed - s.window_start(),
                armed as f32 / RATE as f32
            );
        }
    }

    // ---- committing -------------------------------------------------------

    #[test]
    fn commits_only_words_two_consecutive_passes_agree_on() {
        let mut s = Stream::new();
        assert!(s.advance(w("the quick brown")).is_empty()); // nothing to compare
        // both passes agree on "the quick brown", but the last GUARD_WORDS are
        // held back because the model routinely revises them
        assert_eq!(s.advance(w("the quick brown fox")), w("the quick"));
    }

    #[test]
    fn holds_back_the_guard_words_until_they_settle() {
        let mut s = Stream::new();
        s.advance(w("alpha beta"));
        // a two-word hypothesis is entirely guard, so nothing can be committed
        assert!(s.advance(w("alpha beta")).is_empty());
        assert!(s.committed().is_empty());
    }

    #[test]
    fn never_re_emits_a_word_it_already_committed() {
        let mut s = Stream::new();
        s.advance(w("alpha beta gamma"));
        let mut all = Vec::new();
        for hyp in [
            "alpha beta gamma delta",
            "alpha beta gamma delta epsilon",
            "alpha beta gamma delta epsilon zeta",
        ] {
            all.extend(s.advance(w(hyp)));
        }
        // each word appears at most once across everything emitted
        let mut seen = all.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), all.len(), "duplicate emitted: {all:?}");
        assert_eq!(s.committed(), all.as_slice());
    }

    #[test]
    fn re_anchors_after_a_slide_without_repeating_text() {
        let mut s = Stream::new();
        // build up some committed text the slow way
        s.advance(w("one two three four five"));
        s.advance(w("one two three four five six seven"));
        let before = s.committed().to_vec();
        assert!(!before.is_empty());

        s.maybe_slide(secs(MAX_WINDOW_SECS + 1.0), RATE);
        // the new window re-transcribes the tail plus new speech
        let mut tail = before[before.len().saturating_sub(3)..].to_vec();
        tail.extend(w("eight nine ten"));
        assert!(s.advance(tail.clone()).is_empty(), "anchor pass emits nothing");

        let mut hyp = tail.clone();
        hyp.extend(w("eleven twelve"));
        let out = s.advance(hyp);
        for word in &out {
            assert!(!before.contains(word), "re-typed {word:?} after slide");
        }
    }

    #[test]
    fn unanchorable_slide_still_emits_the_window_rather_than_losing_it() {
        // Regression: the anchor failing used to mark the whole window as
        // already typed, which silently deleted every word in it. Nothing
        // downstream can recover that — the final pass only appends its tail,
        // so a hole in the middle is permanent. Observed live as ~15 words lost
        // at each of seven slides in a 75s dictation.
        let mut s = Stream::new();
        s.advance(w("alpha beta gamma delta"));
        s.advance(w("alpha beta gamma delta epsilon"));
        assert!(!s.committed().is_empty());

        s.maybe_slide(secs(MAX_WINDOW_SECS + 1.0), RATE);
        // window shares no text with what we typed: we must still say it
        assert!(s.advance(w("totally unrelated words here")).is_empty());
        let out = s.advance(w("totally unrelated words here now"));
        assert!(
            !out.is_empty(),
            "an unanchorable window must be replayed, not dropped"
        );
        assert!(out.contains(&"totally".to_string()), "lost the window: {out:?}");
    }

    #[test]
    fn unanchorable_slide_does_not_duplicate_what_is_already_typed() {
        // the other half of the trade: replaying the window must not put text
        // back on screen that is already there
        let mut s = Stream::new();
        s.advance(w("one two three four five six"));
        s.advance(w("one two three four five six seven"));
        let before = s.committed().to_vec();
        assert!(before.len() >= 4);

        s.maybe_slide(secs(MAX_WINDOW_SECS + 1.0), RATE);
        // the new window re-transcribes what we already typed, then continues,
        // but reworded early enough that the anchor cannot match
        let mut hyp = w("XX YY");
        hyp.extend(before.iter().cloned());
        s.advance(hyp.clone());
        hyp.extend(w("eight nine"));
        let out = s.advance(hyp);

        let joined: Vec<String> = before.iter().chain(out.iter()).cloned().collect();
        for i in 0..joined.len().saturating_sub(5) {
            assert_ne!(
                joined[i..i + 3],
                joined[i + 3..i + 6],
                "duplicated run in {joined:?}"
            );
        }
    }

    #[test]
    fn reset_clears_everything_between_utterances() {
        let mut s = Stream::new();
        s.advance(w("a b c"));
        s.advance(w("a b c d"));
        s.maybe_slide(secs(MAX_WINDOW_SECS + 1.0), RATE);
        s.reset();
        assert!(s.committed().is_empty());
        assert_eq!(s.window_start(), 0);
        assert!(s.nothing_typed());
    }

    #[test]
    fn trims_a_seam_that_repeats_the_committed_tail() {
        // the three shapes observed in the 113s streaming run
        assert_eq!(overlap_with_committed(&w("for the"), &w("the entire utterance")), 1);
        assert_eq!(
            overlap_with_committed(&w("jumps over the"), &w("over the lazy dog")),
            2
        );
        assert_eq!(
            overlap_with_committed(
                &w("lazy dog while the engineer reviews the transcription"),
                &w("dog while the engineer reviews the transcription pipeline")
            ),
            7
        );
    }

    #[test]
    fn does_not_trim_when_there_is_no_overlap() {
        assert_eq!(overlap_with_committed(&w("alpha beta"), &w("gamma delta")), 0);
    }

    #[test]
    fn advance_never_emits_text_continuing_where_we_already_are() {
        let mut s = Stream::new();
        s.advance(w("jumps over the lazy"));
        s.advance(w("jumps over the lazy dog"));
        let committed = s.committed().to_vec();
        assert!(!committed.is_empty());

        // a slide whose window re-transcribes the overlap slightly early
        s.maybe_slide(secs(MAX_WINDOW_SECS + 1.0), RATE);
        let mut hyp = w("over the lazy dog while the engineer");
        s.advance(hyp.clone());
        hyp.extend(w("reviews it"));
        let out = s.advance(hyp);

        let joined = format!("{} {}", committed.join(" "), out.join(" "));
        let ws: Vec<&str> = joined.split_whitespace().collect();
        for i in 0..ws.len().saturating_sub(3) {
            assert_ne!(ws[i..i + 2], ws[i + 2..i + 4], "duplicated run in {joined:?}");
        }
    }

    #[test]
    fn join_delta_spaces_against_preceding_text() {
        assert_eq!(join_delta(&w("hello world"), true), "hello world");
        assert_eq!(join_delta(&w("hello world"), false), " hello world");
    }

    // ---- reconciling the release pass with what streaming typed (#50) ------

    /// What the user ends up looking at, **through the real injector**.
    ///
    /// `Release` is only interesting for the text it leaves on screen, so every
    /// test below asserts that rather than the shape of the enum — an assertion
    /// on the shape would still pass with the take-back count off by one.
    ///
    /// This composes the actual `Typed::plan_replace` → `Plan::replace` →
    /// `Plan::simulate` chain rather than modelling it. A hand-rolled model
    /// here was wrong in precisely the case that matters: it deleted
    /// `take_back` characters from the cursor, while the planner deletes the
    /// smaller common-prefix-trimmed count from the same cursor, so the two
    /// agree only while the cursor is where we left it. The interference case
    /// is exactly the one where it is not.
    ///
    /// `recorded` is what the injector believes it typed; `screen` is what is
    /// actually there. They differ only when something else wrote to the screen.
    fn screen_after_with_record(recorded: &str, screen: &str, release: &Release) -> String {
        use wc_inject::plan::{PlanOpts, Typed};
        match release {
            Release::Nothing => screen.to_string(),
            Release::Append(t) => format!("{screen}{t}"),
            Release::Replace { take_back, text } => {
                let mut record = Typed::new();
                record.record(recorded);
                record
                    .plan_replace(*take_back, text, PlanOpts::typing_only())
                    .simulate(screen)
            }
        }
    }

    /// The undisturbed case: the injector's record and the screen agree.
    fn screen_after(before: &str, release: &Release) -> String {
        screen_after_with_record(before, before, release)
    }

    /// The behaviour this issue replaces: append only, spliced on the words.
    /// Kept so each test can show what the old path would have produced, which
    /// is what makes these guards fail against the bug rather than merely
    /// describe the fix.
    fn append_only(streamed: &str, committed: &[String], final_text: &str) -> String {
        let words = split_words(final_text);
        let start = resume_at(committed, &words);
        if start >= words.len() {
            return streamed.to_string();
        }
        format!(
            "{streamed}{}",
            join_delta(&words[start..], streamed.is_empty())
        )
    }

    /// A release plan for text that streamed cleanly: the record covers all of
    /// it, the display server can take it back, and the user kept their hands
    /// off the keyboard.
    fn release(streamed: &str, final_text: &str, rewritten: bool) -> Release {
        plan_release(ReleaseInput {
            streamed,
            committed: &split_words(streamed),
            final_text,
            rewritten,
            replaceable_chars: streamed.chars().count(),
            can_replace: true,
            user_typed: false,
        })
    }

    /// The same, with one fact varied.
    fn release_with(
        streamed: &str,
        final_text: &str,
        f: impl FnOnce(&mut ReleaseInput<'_>),
    ) -> Release {
        let committed = split_words(streamed);
        let mut input = ReleaseInput {
            streamed,
            committed: &committed,
            final_text,
            rewritten: true,
            replaceable_chars: streamed.chars().count(),
            can_replace: true,
            user_typed: false,
        };
        f(&mut input);
        plan_release(input)
    }

    #[test]
    fn nothing_streamed_types_the_whole_finished_text() {
        assert_eq!(
            release("", "Hello there.", false),
            Release::Append("Hello there.".into())
        );
        // Verbatim, not re-joined from words: #67 snippets put newlines in the
        // finished text and splitting on whitespace would flatten them.
        assert_eq!(
            release("", "Best,\nAviroop", true),
            Release::Append("Best,\nAviroop".into())
        );
    }

    #[test]
    fn nothing_streamed_and_nothing_transcribed_sends_nothing() {
        assert_eq!(release("", "", false), Release::Nothing);
    }

    #[test]
    fn an_untouched_transcript_only_appends_the_tail() {
        // The shipped path, and it must stay byte-for-byte what it was: no
        // cleanup, no deletion, only the words that are missing.
        let r = release("the quick brown", "the quick brown fox", false);
        assert_eq!(r, Release::Append(" fox".into()));
        assert_eq!(screen_after("the quick brown", &r), "the quick brown fox");
    }

    #[test]
    fn a_finished_transcript_matching_the_screen_sends_nothing() {
        assert_eq!(release("all of it", "all of it", false), Release::Nothing);
    }

    /// **The acceptance criterion of #50.** Filler removal shortens the
    /// transcript, so the words already on screen are wrong and no tail can fix
    /// them. The old append path is exercised alongside, so this fails against
    /// the behaviour it replaces rather than merely restating the new one.
    #[test]
    fn a_shortening_transform_replaces_what_streaming_typed() {
        let streamed = "So um I think um we should ship it";
        let polished = "So I think we should ship it";
        let committed = split_words(streamed);

        let r = release(streamed, polished, true);
        // "So" is unchanged, so it stays; everything from the first filler on
        // is taken back and retyped.
        assert_eq!(
            r,
            Release::Replace {
                take_back: " um I think um we should ship it".chars().count(),
                text: " I think we should ship it".into()
            }
        );
        assert_eq!(screen_after(streamed, &r), polished);

        // What shipped before this change, on the same input: the fillers stay
        // on screen and the tail is spliced onto them.
        let broken = append_only(streamed, &committed, polished);
        assert_ne!(broken, polished, "the old path was not broken on this input");
        assert!(
            broken.contains("um"),
            "the old path should have left the fillers on screen: {broken:?}"
        );
    }

    #[test]
    fn a_lengthening_transform_replaces_what_streaming_typed() {
        // A dictionary substitution that grows the text, and one that grows it
        // past the streamed run entirely.
        let streamed = "we ship whisper catch on friday";
        let polished = "we ship WhisprCatch on Friday";
        let r = release(streamed, polished, true);
        assert_eq!(screen_after(streamed, &r), polished);
        assert_ne!(append_only(streamed, &split_words(streamed), polished), polished);
    }

    #[test]
    fn a_transform_that_empties_the_transcript_takes_it_all_back() {
        // Every word was a filler. Nothing should be left on screen.
        let streamed = "um uh er";
        let r = release(streamed, "", true);
        assert_eq!(
            r,
            Release::Replace {
                take_back: 8,
                text: String::new()
            }
        );
        assert_eq!(screen_after(streamed, &r), "");
        // Appending cannot express this at all: it would leave every word.
        assert_eq!(append_only(streamed, &split_words(streamed), ""), streamed);
    }

    #[test]
    fn an_entirely_different_transcript_replaces_the_lot() {
        // Marked self-correction (#48): "meet Tuesday, I mean Wednesday" keeps
        // only the correction, and reorders what is left.
        let streamed = "let us meet Tuesday I mean Wednesday";
        let polished = "let us meet Wednesday";
        let r = release(streamed, polished, true);
        assert_eq!(screen_after(streamed, &r), polished);

        // And the pathological case: nothing in common at all.
        let r = release("alpha beta gamma", "totally different words", true);
        assert_eq!(screen_after("alpha beta gamma", &r), "totally different words");
    }

    #[test]
    fn cleanup_that_only_touched_the_unstreamed_tail_still_only_appends() {
        // The finished text still opens with exactly what is on screen, so
        // there is nothing to take back however much cleanup did further on.
        let streamed = "the meeting is at";
        let polished = "the meeting is at 3pm";
        let r = release(streamed, polished, true);
        assert_eq!(r, Release::Append(" 3pm".into()));
        assert_eq!(screen_after(streamed, &r), polished);
    }

    // ---- refusing to delete when the deletion cannot be trusted -------------

    #[test]
    fn a_rewrite_falls_back_to_appending_when_the_record_is_short() {
        // The injector drops its record whenever it stops being sure the text
        // landed — a failed injection, macOS Secure Input. A replace counted
        // against a record shorter than what we streamed would delete whatever
        // sits in front of it, so it must not happen.
        let streamed = "So um I think um we should ship it";
        let polished = "So I think we should ship it";
        // One char short of what the replace needs to take back.
        let needed = release(streamed, polished, true);
        let Release::Replace { take_back, .. } = needed else {
            panic!("expected a replace to be possible at all: {needed:?}");
        };
        let r = release_with(streamed, polished, |i| i.replaceable_chars = take_back - 1);
        assert!(
            !matches!(r, Release::Replace { .. }),
            "replaced against a record that cannot cover the run: {r:?}"
        );
        // Exactly at the boundary it is allowed again.
        let r = release_with(streamed, polished, |i| i.replaceable_chars = take_back);
        assert!(matches!(r, Release::Replace { .. }), "{r:?}");
    }

    #[test]
    fn a_rewrite_falls_back_to_appending_where_modifiers_cannot_be_lifted() {
        // Wayland (#77), and equally the v0.5 default where the wipe is not
        // affordable yet (#68). The daemon does not type live at all in either
        // situation, so `streamed` should be empty in practice; this is the
        // backstop for a session where that changed underneath us.
        let r = release_with(
            "So um I think um we should ship it",
            "So I think we should ship it",
            |i| i.can_replace = false,
        );
        assert!(
            !matches!(r, Release::Replace { .. }),
            "planned a backspace run where a held modifier cannot be lifted: {r:?}"
        );
    }

    /// The reviewer's case, and the reason the replace is refused rather than
    /// bounded: once the cursor has moved, a count taken from it lands in the
    /// wrong place and eats the user's characters.
    #[test]
    fn a_rewrite_falls_back_to_appending_when_the_user_typed_too() {
        let streamed = "So um I think";
        let polished = "So I think";
        let r = release_with(streamed, polished, |i| i.user_typed = true);
        assert!(
            !matches!(r, Release::Replace { .. }),
            "deleted with the cursor in an unknown place: {r:?}"
        );
        // The user's own characters survive, whatever else does.
        let screen = screen_after_with_record(streamed, &format!("{streamed}XYZ"), &r);
        assert!(
            screen.starts_with(&format!("{streamed}XYZ")),
            "the user's keystrokes were destroyed: {screen:?}"
        );
    }

    #[test]
    fn a_fallback_never_shortens_what_is_on_screen() {
        // Whatever it decides, a fallback may only add characters. Stated over
        // every combination that can reach one, because "it appends" is the
        // property the user's text depends on, not "it takes this branch".
        let streamed = "So um I think um we should ship it";
        for recorded in [0usize, 5, 999] {
            for can_replace in [true, false] {
                for user_typed in [true, false] {
                    if can_replace && !user_typed && recorded == 999 {
                        continue; // the replace is legitimately available here
                    }
                    let r = release_with(streamed, "So I think we should ship it", |i| {
                        i.replaceable_chars = recorded;
                        i.can_replace = can_replace;
                        i.user_typed = user_typed;
                    });
                    let screen = screen_after(streamed, &r);
                    assert!(
                        screen.starts_with(streamed),
                        "recorded={recorded} can_replace={can_replace} \
                         user_typed={user_typed} shortened the screen: {screen:?}"
                    );
                }
            }
        }
    }

    /// With no cleanup enabled — what every user ships with — the release pass
    /// must never plan a deletion, whatever the model did between passes.
    #[test]
    fn an_unrewritten_utterance_never_deletes() {
        let mut rng = Rng(0x5EED_0050_ABCD_0001);
        let mut appends = 0;
        for i in 0..3_000 {
            let streamed = rng.words(6);
            let finished = rng.words(8);
            let r = release(&streamed, &finished, false);
            assert!(
                !matches!(r, Release::Replace { .. }),
                "iteration {i}: deleted without a rewrite. streamed={streamed:?} \
                 finished={finished:?}"
            );
            // And it puts the same words on screen as the path that shipped.
            // Words, not bytes: where the shipped path re-joined the tail with
            // single spaces this one slices the transcript, so a double space
            // or a newline in the finished text now survives (#67 snippets).
            // That is the only difference, and this pins it to be the only one.
            assert_eq!(
                split_words(&screen_after(&streamed, &r)),
                split_words(&append_only(&streamed, &split_words(&streamed), &finished)),
                "iteration {i}: streamed={streamed:?} finished={finished:?}"
            );
            if matches!(r, Release::Append(_)) {
                appends += 1;
            }
        }
        assert!(appends > 0, "the generator stopped producing appends");
    }

    #[test]
    fn an_ordinary_transcript_is_spliced_byte_for_byte_as_it_always_was() {
        // The whitespace caveat above matters only for transcripts with odd
        // whitespace in them. For the ones a model actually emits, the release
        // pass is unchanged to the byte.
        for (streamed, finished) in [
            ("", "hello world"),
            ("hello", "hello world"),
            ("the quick brown", "the quick brown fox jumps"),
            ("all of it", "all of it"),
            ("for the", "for the gut comedy for the grin"),
        ] {
            assert_eq!(
                screen_after(streamed, &release(streamed, finished, false)),
                append_only(streamed, &split_words(streamed), finished),
                "streamed {streamed:?}"
            );
        }
    }

    /// The property the whole issue asks for: when cleanup rewrote the words,
    /// the user is left looking at the finished transcript.
    ///
    /// "Reading the finished text" is stated as word equality under [`norm`],
    /// not byte equality, and that is the honest form of the promise. Words
    /// before the first genuine change are left as the model typed them, so the
    /// screen can differ from the transcript by a capital or a comma — the same
    /// bargain the append path has always made, and the thing that keeps a
    /// three-character edit from wiping the whole utterance. The tail, where
    /// the change actually is, is asserted byte for byte below.
    #[test]
    fn a_rewritten_utterance_always_ends_up_reading_the_finished_text() {
        let mut rng = Rng(0x5EED_0050_ABCD_0002);
        let mut replaced = 0;
        for i in 0..3_000 {
            let streamed = rng.words(6);
            let finished = rng.words(8);
            if streamed.is_empty() {
                continue;
            }
            let r = release(&streamed, &finished, true);
            let screen = screen_after(&streamed, &r);
            assert_eq!(
                normalized(&split_words(&screen)),
                normalized(&split_words(&finished)),
                "iteration {i}: streamed={streamed:?} finished={finished:?} plan={r:?} \
                 screen={screen:?}"
            );
            if let Release::Replace { text, .. } = &r {
                replaced += 1;
                assert!(
                    screen.ends_with(text),
                    "iteration {i}: the replaced tail is not exact. screen={screen:?} \
                     text={text:?}"
                );
            }
        }
        assert!(replaced > 0, "the generator stopped producing rewrites");
    }

    /// The blast radius is bounded by the first word that genuinely changed,
    /// not by the first byte that differs.
    #[test]
    fn only_the_words_that_changed_are_taken_back() {
        // A leading capital is routine — the streaming window and the
        // whole-utterance pass disagree about sentence starts all the time. A
        // byte-wise common prefix would be 0 here and wipe all 41 characters
        // for a three-character edit.
        let streamed = "the meeting is at twenty five past um four";
        let polished = "The meeting is at 25 past four";
        let r = release(streamed, polished, true);
        let Release::Replace { take_back, text } = &r else {
            panic!("expected a replace: {r:?}");
        };
        assert_eq!(*take_back, " twenty five past um four".chars().count());
        assert_eq!(text, " 25 past four");
        // What a byte-wise common prefix would have cost: everything, because
        // the very first character differs.
        let bytewise = streamed
            .chars()
            .zip(polished.chars())
            .take_while(|(a, b)| a == b)
            .count();
        assert_eq!(bytewise, 0, "the leading capital is the whole point");
        assert!(
            *take_back < streamed.chars().count() - bytewise,
            "took back {take_back} of {} chars — no better than a byte-wise prefix",
            streamed.chars().count()
        );
        assert_eq!(
            screen_after(streamed, &r),
            "the meeting is at 25 past four",
            "the words before the change keep the model's own capitalisation"
        );
    }

    /// xorshift64 with a fixed seed, so a failure reproduces exactly. Same
    /// trick as `wc-inject`'s planner tests; a property test is not worth a new
    /// dependency.
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

        /// A short utterance drawn from a vocabulary with repeats, fillers and
        /// punctuation, so the word aligner is given the ambiguity it has to
        /// cope with rather than a stream of unique tokens.
        fn words(&mut self, max: usize) -> String {
            const VOCAB: &[&str] = &[
                "um", "so", "the", "the", "quick", "brown", "fox", "I", "mean", "we", "should",
                "ship", "it", "café", "你好", "twenty", "five", "percent.", "and",
            ];
            let n = self.below(max + 1);
            (0..n)
                .map(|_| VOCAB[self.below(VOCAB.len())])
                .collect::<Vec<_>>()
                .join(" ")
        }
    }

    // ---- what streaming actually put on screen ------------------------------

    #[test]
    fn typed_is_exactly_what_was_sent_to_the_injector() {
        let mut s = Stream::new();
        let mut sent = String::new();
        for hyp in [
            "alpha beta gamma",
            "alpha beta gamma delta",
            "alpha beta gamma delta epsilon",
            "alpha beta gamma delta epsilon zeta",
        ] {
            let delta = s.advance(w(hyp));
            if !delta.is_empty() {
                let text = join_delta(&delta, s.nothing_typed());
                sent.push_str(&text);
                s.mark_typed(&text);
            }
        }
        assert!(!sent.is_empty());
        assert_eq!(s.typed(), sent);
        // With every delta injected, that is the committed words with single
        // spaces — which is what makes `plan_release`'s prefix test meaningful.
        assert_eq!(s.typed(), s.committed().join(" "));
        assert!(!s.nothing_typed());
    }

    #[test]
    fn a_delta_that_failed_to_inject_is_not_counted_as_on_screen() {
        // The daemon calls `mark_typed` only after a successful send, so a
        // failed injection leaves a word committed that never reached the
        // screen. `typed` must follow the screen and not the word list: a
        // release that took back `committed.join(" ")` worth of characters
        // would count presses against text that is not there and delete the
        // user's own writing in front of it.
        let mut s = Stream::new();
        s.advance(w("alpha beta gamma"));
        let first = s.advance(w("alpha beta gamma delta"));
        assert!(!first.is_empty());
        // ... the injection failed, so no mark_typed. Nothing is on screen.
        assert!(s.nothing_typed());
        assert_eq!(s.typed(), "");

        // The next pass lands. Because nothing had been typed, it carries no
        // leading space — the screen starts at this delta.
        let second = s.advance(w("alpha beta gamma delta epsilon zeta"));
        assert!(!second.is_empty());
        let text = join_delta(&second, s.nothing_typed());
        s.mark_typed(&text);

        assert_eq!(s.typed(), text);
        assert!(!s.nothing_typed());
        assert_ne!(
            s.typed(),
            s.committed().join(" "),
            "`typed` must describe the screen, not the words we meant to type"
        );
        assert!(
            s.typed().chars().count() < s.committed().join(" ").chars().count(),
            "the lost delta should make `typed` the shorter of the two"
        );

        // And the release plan takes back only what is really there.
        let finished = "alpha beta gamma delta epsilon zeta";
        let r = plan_release(ReleaseInput {
            streamed: s.typed(),
            committed: s.committed(),
            final_text: finished,
            rewritten: true,
            replaceable_chars: s.typed().chars().count(),
            can_replace: true,
            user_typed: false,
        });
        assert_eq!(
            r,
            Release::Replace {
                take_back: s.typed().chars().count(),
                text: finished.into()
            }
        );
        assert_eq!(screen_after(s.typed(), &r), finished);
    }

    /// What a replace does to the user's own characters when it is issued
    /// anyway — the damage the `user_typed` refusal exists to prevent.
    ///
    /// The first version of this test asserted a *model* of the replace rather
    /// than the replace, and specified damage the code does not produce: it
    /// deleted `take_back` chars from the cursor, while the planner deletes the
    /// smaller common-prefix-trimmed count from the same cursor. The two agree
    /// only while the cursor is where we left it, which is exactly not this
    /// case. Everything below goes through the real `Typed::plan_replace`.
    ///
    /// The damage is *not* "bounded and roughly right". Once the cursor moves,
    /// the deletion window slides off the end of our run and takes the user's
    /// characters first — and with a long enough interjection it takes nothing
    /// else, leaving their text destroyed and the fillers still on screen.
    ///
    /// This is why the daemon refuses rather than bounds. On Linux it knows,
    /// because the evdev listener already reads every key and counting the ones
    /// that are not ours costs a `!=`. On macOS with a modifier hotkey it does
    /// not know, because that tap only receives `FlagsChanged` — and widening
    /// it is not a trade this project makes.
    #[test]
    fn issuing_a_replace_after_the_user_typed_destroys_their_characters() {
        let streamed = "So um I think";
        let polished = "So I think";
        let r = release(streamed, polished, true);
        let Release::Replace { take_back, .. } = &r else {
            panic!("expected a replace, got {r:?}");
        };
        // The count is still bounded by our own run — that part always held.
        assert!(*take_back <= streamed.chars().count());

        // Undisturbed, it is exactly right.
        assert_eq!(screen_after(streamed, &r), polished);

        // Disturbed, it is not. Each row is the real planner against a screen
        // the record no longer describes.
        for (interjection, expected) in [
            ("X", "So I thinkX"),
            ("XYZ", "So I thinkXYZ"),
            ("XYZABCDEFGHIJ", "So I thinkXYZABCDEFGHIJ"),
        ] {
            let screen = screen_after_with_record(
                streamed,
                &format!("{streamed}{interjection}"),
                &r,
            );
            assert_ne!(
                screen, expected,
                "if this ever holds, the replace became safe under interference \
                 and the `user_typed` refusal can be reconsidered"
            );
            assert!(
                !screen.contains(interjection),
                "the user's {interjection:?} survived: {screen:?}"
            );
        }

        // And the refusal: told that the user typed, it never plans a deletion.
        let refused = release_with(streamed, polished, |i| i.user_typed = true);
        assert!(!matches!(refused, Release::Replace { .. }), "{refused:?}");
        let screen =
            screen_after_with_record(streamed, &format!("{streamed}XYZ"), &refused);
        assert!(
            screen.starts_with(&format!("{streamed}XYZ")),
            "the refusal must leave every character alone: {screen:?}"
        );
    }

    // ---- driving the injector ----------------------------------------------

    /// A `TextSink` that remembers what it was asked to do, and can be told to
    /// fail. The screen is modelled through the real `Typed`/`Plan`, so a
    /// replace lands here exactly as it would on a keyboard.
    #[derive(Default)]
    struct FakeSink {
        screen: String,
        record: wc_inject::plan::Typed,
        forgets: usize,
        fail_type: bool,
        log: Vec<String>,
    }

    impl TextSink for FakeSink {
        fn type_text(&mut self, text: &str) -> anyhow::Result<()> {
            self.log.push(format!("type {text:?}"));
            if self.fail_type {
                self.record.forget();
                anyhow::bail!("no keyboard");
            }
            self.screen.push_str(text);
            self.record.record(text);
            Ok(())
        }
        fn replace_last(&mut self, n_chars: usize, text: &str) -> anyhow::Result<()> {
            self.log.push(format!("replace {n_chars} {text:?}"));
            let plan = self.record.plan_replace(
                n_chars,
                text,
                wc_inject::plan::PlanOpts::typing_only(),
            );
            self.screen = plan.simulate(&self.screen);
            Ok(())
        }
        fn forget_typed(&mut self) {
            self.forgets += 1;
            self.record.forget();
            self.log.push("forget".into());
        }
        fn replaceable_chars(&self) -> usize {
            self.record.known_chars()
        }
    }

    #[test]
    fn starting_an_utterance_forgets_what_was_typed_before_it() {
        // Between utterances the cursor can go anywhere. Without this, a
        // replace at the end of utterance two counts presses against utterance
        // one's text — in whatever window it happened to land in.
        let mut sink = FakeSink::default();
        let mut s = Stream::new();
        sink.type_text("text from a previous utterance").unwrap();
        assert!(sink.replaceable_chars() > 0);

        begin_utterance(&mut sink, &mut s);

        assert_eq!(sink.forgets, 1, "the record was not dropped");
        assert_eq!(sink.replaceable_chars(), 0);
        assert!(s.typed().is_empty() && s.committed().is_empty());

        // So a replace now can only ever type.
        let (release, sent) =
            finish_utterance(&mut sink, &s, "a new sentence", true, true, false);
        assert!(sent.is_ok());
        assert_eq!(release, Release::Append("a new sentence".into()));
        assert_eq!(
            sink.screen, "text from a previous utterancea new sentence",
            "the earlier utterance was disturbed"
        );
    }

    #[test]
    fn a_delta_is_noted_only_after_it_lands() {
        let mut s = Stream::new();
        let mut sink = FakeSink::default();
        assert_eq!(stream_delta(&mut sink, &mut s, &w("hello world")), "hello world");
        assert_eq!(s.typed(), "hello world");

        // Now the keyboard goes away. The words are still committed by
        // `advance`, but nothing reached the screen, so `typed` must not grow.
        sink.fail_type = true;
        assert_eq!(stream_delta(&mut sink, &mut s, &w("and more")), "");
        assert_eq!(
            s.typed(),
            "hello world",
            "counted text the injector could not send"
        );
    }

    #[test]
    fn a_delta_that_failed_leaves_the_release_unable_to_delete() {
        // The two halves together: the failed send drops the injector's record
        // *and* is kept out of `typed`, so the release has nothing to take back
        // and types instead. This is the path that protects a user whose
        // injection failed halfway through an utterance.
        let mut s = Stream::new();
        let mut sink = FakeSink::default();
        stream_delta(&mut sink, &mut s, &w("hello world"));
        sink.fail_type = true;
        stream_delta(&mut sink, &mut s, &w("and more"));
        sink.fail_type = false;

        assert_eq!(sink.replaceable_chars(), 0, "the record survived a failure");
        let (release, _) =
            finish_utterance(&mut sink, &s, "Hello, world and more.", true, true, false);
        assert!(
            !matches!(release, Release::Replace { .. }),
            "deleted against a record that was dropped: {release:?}"
        );
    }

    #[test]
    fn the_release_carries_out_exactly_what_was_planned() {
        // `finish_utterance` is wiring, so what it must not do is decide
        // anything: whatever `plan_release` says, that is what reaches the sink.
        let mut s = Stream::new();
        let mut sink = FakeSink::default();
        stream_delta(&mut sink, &mut s, &w("So um I think"));
        assert_eq!(sink.screen, "So um I think");

        let (release, sent) =
            finish_utterance(&mut sink, &s, "So I think we should", true, true, false);
        assert!(sent.is_ok());
        assert!(matches!(release, Release::Replace { .. }), "{release:?}");
        assert_eq!(sink.screen, "So I think we should");
        assert!(
            sink.log.last().unwrap().starts_with("replace"),
            "{:?}",
            sink.log
        );
    }

    #[test]
    fn a_release_that_may_not_delete_appends_instead() {
        let mut s = Stream::new();
        let mut sink = FakeSink::default();
        stream_delta(&mut sink, &mut s, &w("So um I think"));

        // The one bit says the user typed, so no deletion may be planned.
        let (release, sent) =
            finish_utterance(&mut sink, &s, "So I think we should", true, true, true);
        assert!(sent.is_ok());
        assert!(matches!(release, Release::Append(_)), "{release:?}");
        assert!(
            sink.screen.starts_with("So um I think"),
            "took characters back anyway: {:?}",
            sink.screen
        );
    }

    #[test]
    fn reset_clears_what_streaming_typed() {
        let mut s = Stream::new();
        s.mark_typed("hello");
        assert!(!s.nothing_typed());
        s.reset();
        assert_eq!(s.typed(), "");
        assert!(s.nothing_typed());
    }
}
