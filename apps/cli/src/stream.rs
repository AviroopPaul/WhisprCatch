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
    let max = c.len().min(d.len()).min(MAX_ANCHOR);
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
    /// True until the first word of the utterance has been typed, so the caller
    /// knows whether to prefix a space.
    nothing_typed: bool,
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
            nothing_typed: true,
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

    pub fn nothing_typed(&self) -> bool {
        self.nothing_typed
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
            // Re-locate ourselves in the new window. If we can't, treat the
            // whole window as already typed rather than risk repeating it —
            // the final pass re-transcribes everything and fills any gap.
            self.committed_in_window = anchor_in_window(&self.committed, &hyp).unwrap_or(hyp.len());
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

    pub fn mark_first_typed(&mut self) {
        self.nothing_typed = false;
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
    fn unanchorable_slide_emits_nothing_rather_than_repeating() {
        let mut s = Stream::new();
        s.advance(w("alpha beta gamma delta"));
        s.advance(w("alpha beta gamma delta epsilon"));
        let before = s.committed().to_vec();

        s.maybe_slide(secs(MAX_WINDOW_SECS + 1.0), RATE);
        // window shares no text with what we typed — must not replay it
        assert!(s.advance(w("totally unrelated words here")).is_empty());
        let out = s.advance(w("totally unrelated words here again"));
        for word in &out {
            assert!(!before.contains(word), "replayed {word:?}");
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
}
