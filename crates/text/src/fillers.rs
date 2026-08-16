//! Filler removal — "um", "uh", "you know", "like". The lowest of Wispr's four
//! cleanup grades, and the one users notice first.
//!
//! Three graded levels, mirroring the grading users already expect. **Two of
//! them ship**:
//!
//! | level | removes | selectable |
//! |---|---|---|
//! | `off` | nothing — output is byte-identical to the input | yes |
//! | `light` | hesitation sounds (um, uh, er…) and stutters ("the the the") | yes |
//! | `medium` | light, plus hedges (like, you know, I mean, sort of…) | **no — see [`FillerLevel::parse`] and #74** |
//!
//! # The one rule that matters
//!
//! A missed filler is a papercut. A deleted meaningful word is a bug report and
//! a trust problem, because the user cannot see what was taken — they said it,
//! and it is simply not there. So every judgement call below resolves the same
//! way: **when in doubt, leave the text alone.**
//!
//! `light` earns that. Its word lists are closed classes checked one entry at a
//! time against a legitimate use, and a hesitation sound is not a word.
//! `medium` does not, yet: it rests on a comma meaning "this hedge is a
//! filler", and a comma means nothing of the sort — which is why it is gated
//! rather than shipped.
//!
//! # Repair
//!
//! Deleting a word damages what is around it, so removal is a splice, not a
//! `replace`. Everything outside the spliced region is copied byte for byte.
//! Inside it:
//!
//! - **Punctuation**: a filler sits in a pause, the pause is written as
//!   punctuation, and afterwards exactly one mark stands for it — the strongest
//!   found beside *or inside* the removed run, leftmost on a tie. Inside
//!   matters: "I think so, um. Uh, what's next?" keeps the full stop that ends
//!   the sentence rather than letting the two commas outvote it. This transform
//!   does not delete punctuation the model wrote: "However, um, we shipped"
//!   becomes "However, we shipped", never "However we shipped". The accepted
//!   cost of that is a comma the user can see and remove — "it was, uh,
//!   complicated" becomes "it was, complicated".
//! - **Whitespace**: merged to a single space, or to the line break if either
//!   side had one, because a paragraph is structure and not spacing.
//! - **Capitalization**: a word left standing at the start of a sentence is
//!   capitalized. Where nothing proves a sentence began — the start of the
//!   text, of a line, or of a list item — the removed word's own case is the
//!   evidence: "Um, the build broke" was a sentence, "um and then I left" is
//!   someone dictating into the middle of one.
//!
//! # What runs before this
//!
//! Two transforms, and both change what this one sees.
//!
//! [`crate::Spoken`] (#45) synthesizes `\n\n`, `- ` and `1. ` from dictated
//! "new paragraph" and "bullet", and a `.` from the spoken word "period". None
//! of them is distinguishable from model output by the time it gets here, so
//! the repair treats all three as structure: the ASCII hyphen is deliberately
//! not droppable punctuation, a line break is never crossed by a mark migrating
//! left, and a list marker is a fresh start rather than a sentence end.
//!
//! [`crate::SelfCorrect`] (#48) owns "I mean" as a correction marker, which is
//! also a `medium` hedge. With `medium` gated, this transform cannot touch it
//! at all — one more reason the gate is the right call while #48 is still a
//! stub on `main`.

use std::fmt;

use serde::{de, Deserialize, Deserializer, Serialize};

use crate::Transform;

// ---------------------------------------------------------------- config

/// How much to remove. Defaults to [`FillerLevel::Off`] — the most conservative
/// setting — because the shipping default is an open product decision, and a
/// transform that deletes words should never turn itself up on an upgrade.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FillerLevel {
    /// Byte-identical to the input.
    #[default]
    Off,
    /// Hesitation sounds and stutters.
    Light,
    /// `Light`, plus hedges in parenthetical position.
    Medium,
}

impl FillerLevel {
    /// The levels a user can actually select. `Medium` is deliberately absent —
    /// see [`FillerLevel::parse`].
    pub const SELECTABLE: [Self; 2] = [Self::Off, Self::Light];

    /// The name used in `config.toml` and in Settings (#49).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Light => "light",
            Self::Medium => "medium",
        }
    }

    /// Case- and whitespace-insensitive; `None` for anything a user may not
    /// select — which today includes `"medium"`.
    ///
    /// **`medium` is gated, not finished.** Its whole safety argument rests on
    /// a comma being evidence that a hedge is a filler, and a comma is not: it
    /// marks a prosodic break, which tag questions ("You know, don't you?"),
    /// hedged answers ("Well, sort of, yes."), vocatives, appositives and
    /// correction markers ("Tuesday, I mean, Wednesday") produce just as
    /// readily. Every one of those loses a load-bearing word, and every one of
    /// them needs the commas to be there — so the level is live *exclusively*
    /// in the case whose safety nobody has checked against real model output.
    /// Issue #74 answers that question first. The code stays because `light`
    /// shares all of it; only the door is locked.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "light" => Some(Self::Light),
            _ => None,
        }
    }
}

impl fmt::Display for FillerLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Deliberately lenient: anything this build cannot use deserializes to `Off`
/// instead of failing.
///
/// `apps/cli` propagates a config parse error out of `main`, so a strict
/// `Deserialize` would turn one typo in a hand-edited `config.toml` — or a
/// level a newer build wrote and this one has never heard of — into a daemon
/// that refuses to start. Falling back to the most conservative level keeps
/// dictation working and costs the user a setting, not their tool.
///
/// `deserialize_any`, not `deserialize_str`, because `level = 3` and
/// `level = true` are the same class of mistake as `level = "meduim"` and
/// deserve the same landing.
impl<'de> Deserialize<'de> for FillerLevel {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct LevelVisitor;
        impl<'de> de::Visitor<'de> for LevelVisitor {
            type Value = FillerLevel;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(r#""off" or "light""#)
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<FillerLevel, E> {
                Ok(FillerLevel::parse(s).unwrap_or(FillerLevel::Off))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<FillerLevel, E> {
                Ok(FillerLevel::Off)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<FillerLevel, E> {
                Ok(FillerLevel::Off)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<FillerLevel, E> {
                Ok(FillerLevel::Off)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<FillerLevel, E> {
                Ok(FillerLevel::Off)
            }
            fn visit_unit<E: de::Error>(self) -> Result<FillerLevel, E> {
                Ok(FillerLevel::Off)
            }
            fn visit_none<E: de::Error>(self) -> Result<FillerLevel, E> {
                Ok(FillerLevel::Off)
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<FillerLevel, D::Error> {
                FillerLevel::deserialize(d)
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut a: A) -> Result<FillerLevel, A::Error> {
                while a.next_element::<de::IgnoredAny>()?.is_some() {}
                Ok(FillerLevel::Off)
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut a: A) -> Result<FillerLevel, A::Error> {
                while a.next_entry::<de::IgnoredAny, de::IgnoredAny>()?.is_some() {}
                Ok(FillerLevel::Off)
            }
        }
        d.deserialize_any(LevelVisitor)
    }
}

/// Config for [`Fillers`]. Ships disabled, at the most conservative level.
///
/// Deliberately not `Copy`, even though it currently could be: `lib.rs` clones
/// it in two places, and this file is the only one issue #44 is allowed to
/// touch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FillersConfig {
    /// Off by default: with this false the transform is not even in the chain.
    pub enabled: bool,
    /// How much to remove. `off` even when `enabled`, so turning the feature on
    /// is one decision and choosing how aggressive it is is another.
    pub level: FillerLevel,
}

// ---------------------------------------------------------------- word lists

/// Hesitation sounds, deleted wherever they stand as a whole word.
///
/// Spelling variants are listed rather than derived — "collapse a run of the
/// same letter" would also eat "err" (a real verb) and "mmm-hmm" (agreement).
///
/// Deliberately absent:
/// - **`mm`** — the issue names it, but it is also the SI symbol for
///   millimetre, and "a 5 mm gap" is a thing people dictate. `mmm` (three or
///   more) is never a unit, so that one stays.
/// - **`hmm`, `ah`, `oh`, `eh`, `huh`** — these carry attitude ("Hmm." is not
///   the same message as ""), so they are not hesitation noise.
const HESITATIONS: &[&str] = &[
    "er", "erm", "mmm", "mmmm", "uh", "uhh", "uhhh", "uhm", "um", "umm", "ummm",
];

/// A hesitation directly followed by one of these is left alone: "uh huh" and
/// "uh oh" are words, and deleting the first half changes what the user said.
/// (The hyphenated spellings, "uh-huh" and "uh-oh", are single tokens and never
/// match a hesitation in the first place.)
const PAIR_FOLLOWERS: &[&str] = &["hm", "hmm", "huh", "oh"];

/// Words a run of which is a stutter rather than something the user meant.
///
/// Closed-class function words only, and every entry was checked against a
/// legitimate adjacent repetition before it went in. The list is short on
/// purpose; missing a stutter costs a papercut, collapsing a real repetition
/// costs trust. Notable exclusions and why:
///
/// - `had` — "I had had enough"
/// - `that` — "the fact that that happened"
/// - `is`, `was`, `are`, `were` — pseudo-clefts: "what it is is a problem"
/// - `can` — "a can can hold two litres"
/// - `do` — emphatic do-support: "I do do my own taxes"
/// - `in`, `on`, `up`, `out`, `off`, `by`, `down`, `back` — particle meets
///   preposition: "she came in in a hurry", "put it on on Monday"
/// - `no`, `so`, `very`, `really`, `well`, `now`, `there`, `here`, `why` —
///   repetition is the emphasis: "very very good", "there there"
/// - `he` — "he he" is laughter as often as it is a stutter
/// - `will`, `may` — also names and months
/// - `my`, `your`, `our` — "oh my my"
const STUTTER_WORDS: &[&str] = &[
    "a", "an", "and", "at", "because", "but", "could", "for", "from", "how", "i", "into", "it",
    "just", "of", "or", "should", "the", "then", "these", "they", "this", "those", "to", "we",
    "when", "where", "which", "with", "would", "you",
];

/// A hedge, and the two things that decide whether stripping it is safe.
struct Hedge {
    /// The words, lowercase, in order. Matched only when nothing but
    /// whitespace separates them.
    words: &'static [&'static str],
    /// May a sentence boundary — the start of the utterance, or a preceding
    /// full stop — stand in for the opening comma?
    ///
    /// True for the discourse markers, which routinely open an utterance
    /// ("You know, I was thinking"). False for `sort of`, `basically` and
    /// `actually`, which are load-bearing exactly there: "Basically, the answer
    /// is no" is the user making a point, not clearing their throat.
    start_ok: bool,
    /// Also a common imperative verb, so a following word gets inspected. Only
    /// `like` — "subscribe, like, and comment" is a list of instructions, not a
    /// hedged sentence.
    imperative_risk: bool,
}

const HEDGES: &[Hedge] = &[
    Hedge {
        words: &["like"],
        start_ok: true,
        imperative_risk: true,
    },
    Hedge {
        words: &["you", "know"],
        start_ok: true,
        imperative_risk: false,
    },
    Hedge {
        words: &["i", "mean"],
        start_ok: true,
        imperative_risk: false,
    },
    Hedge {
        words: &["sort", "of"],
        start_ok: false,
        imperative_risk: false,
    },
    Hedge {
        words: &["basically"],
        start_ok: false,
        imperative_risk: false,
    },
    Hedge {
        words: &["actually"],
        start_ok: false,
        imperative_risk: false,
    },
];

/// Words that can open a clause. A sentence-initial "like" is only treated as a
/// filler when one of these follows it, because the alternative reading —
/// "Like, comment, and subscribe" — is an imperative, and deleting the verb
/// there is exactly the failure this whole module is arranged to avoid.
const CLAUSE_STARTERS: &[&str] = &[
    "a",
    "an",
    "everybody",
    "everyone",
    "he",
    "he's",
    "her",
    "here",
    "his",
    "i",
    "i'd",
    "i'll",
    "i'm",
    "i've",
    "if",
    "it",
    "it's",
    "its",
    "my",
    "nobody",
    "our",
    "she",
    "she's",
    "someone",
    "that",
    "that's",
    "the",
    "their",
    "them",
    "there",
    "there's",
    "these",
    "they",
    "they'll",
    "they're",
    "they've",
    "this",
    "those",
    "us",
    "we",
    "we'll",
    "we're",
    "we've",
    "what",
    "what's",
    "when",
    "where",
    "who",
    "why",
    "you",
    "you'll",
    "you're",
    "you've",
    "your",
];

/// A hedge followed by one of these is part of a list rather than an aside:
/// "subscribe, like, and comment".
const CONJUNCTIONS: &[&str] = &["and", "nor", "or", "plus"];

/// Longest entry in any list above, as a byte length. Words longer than this
/// cannot match anything, which is what keeps a 100 000-character token cheap.
const MAX_LIST_WORD: usize = 9;

// ---------------------------------------------------------------- transform

/// Deletes hesitation words and hedges.
///
/// Runs *after* [`crate::SelfCorrect`]: "I mean" is a hedge this transform
/// strips and a correction marker #48 needs to see first. See
/// `Polish::from_config`.
pub struct Fillers {
    cfg: FillersConfig,
}

impl Fillers {
    pub fn new(cfg: FillersConfig) -> Self {
        Self { cfg }
    }
}

impl Transform for Fillers {
    fn name(&self) -> &'static str {
        "fillers"
    }

    fn apply(&self, text: &str) -> String {
        if self.cfg.level == FillerLevel::Off {
            return text.to_string();
        }
        let words = words(text);
        if words.is_empty() {
            return text.to_string();
        }
        let del = mark(text, &words, self.cfg.level);
        if !del.iter().any(|d| *d) {
            return text.to_string();
        }
        // An utterance that is nothing but fillers still has to type something:
        // an empty result reads as "the app is broken", and the user cannot
        // tell an eaten sentence from a missed hotkey. Hand back the raw text.
        if del.iter().all(|d| *d) {
            return text.to_string();
        }
        rebuild(text, &words, &del)
    }

    /// Not prefix-stable: it deletes. Confirmed against `prefix_violation` on
    /// the real implementation, which finds a smaller counterexample than the
    /// obvious one — `apply("so u") == "so u"` is not a prefix of
    /// `apply("so um yeah") == "so yeah"`, so the streaming pass is already
    /// wrong one character into a word it has not finished hearing. Deletion
    /// can never satisfy the prefix property.
    fn prefix_stable(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------- tokens

/// A byte range of the original text — a word while tokenizing, a run of
/// punctuation while repairing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    start: usize,
    end: usize,
}

/// A word is a run of alphanumerics (plus combining marks, so "e" + U+0301 does
/// not split), and may contain an apostrophe or hyphen *between* two of them.
/// That is what keeps "don't" and "like-minded" single tokens — splitting the
/// second one would leave a stripped "like" behind as "-minded".
fn is_word_char(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric();
    }
    c.is_alphanumeric() || is_combining(c)
}

fn is_combining(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}'
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20F0}'
        | '\u{FE20}'..='\u{FE2F}')
}

/// Characters that hold a word together when a word character sits on each
/// side. The underscore is in here because this app's users dictate
/// `snake_case` identifiers, and without it "um_var" tokenizes as "um" plus
/// "var" and filler removal eats half of a name.
fn is_connector(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}' | '-' | '_')
}

fn words(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut idx = 0;
    while idx < text.len() {
        let c = char_at(text, idx);
        if is_word_char(c) {
            let end = scan_word(text, idx);
            out.push(Span { start: idx, end });
            idx = end;
        } else {
            idx += c.len_utf8();
        }
    }
    out
}

/// End of the word starting at `start`. Shared with the capitalization pass so
/// the two can never disagree about where a word ends.
fn scan_word(text: &str, start: usize) -> usize {
    let mut end = start;
    for (off, c) in text[start..].char_indices() {
        if is_word_char(c) {
            end = start + off + c.len_utf8();
        } else if !is_connector(c) {
            break;
        }
    }
    end
}

fn char_at(text: &str, idx: usize) -> char {
    text[idx..].chars().next().expect("idx is a char boundary")
}

fn word(text: &str, s: Span) -> &str {
    &text[s.start..s.end]
}

/// ASCII-case-insensitive comparison against a lowercase ASCII literal, with
/// U+2019 treated as an apostrophe so "it’s" matches "it's".
fn eq_word(w: &str, lower: &str) -> bool {
    let mut a = w.chars();
    let mut b = lower.chars();
    loop {
        match (a.next(), b.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) => {
                let x = if x == '\u{2019}' {
                    '\''
                } else {
                    x.to_ascii_lowercase()
                };
                if x != y {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

fn in_list(text: &str, s: Span, list: &[&str]) -> bool {
    let w = word(text, s);
    if w.len() > MAX_LIST_WORD || w.is_empty() {
        return false;
    }
    let first = w.as_bytes()[0].to_ascii_lowercase();
    list.iter()
        .any(|c| c.as_bytes()[0] == first && eq_word(w, c))
}

// ---------------------------------------------------------------- marking

fn mark(text: &str, words: &[Span], level: FillerLevel) -> Vec<bool> {
    let mut del = vec![false; words.len()];
    if level == FillerLevel::Off {
        return del;
    }

    for (i, w) in words.iter().enumerate() {
        if in_list(text, *w, HESITATIONS) && !starts_a_pair(text, words, i) {
            del[i] = true;
        }
    }

    collapse_stutters(text, words, &mut del);

    if level == FillerLevel::Medium {
        strip_hedges(text, words, &mut del);
    }
    del
}

/// "uh huh", "uh oh", "um hmm" — the hesitation is half of a word here.
fn starts_a_pair(text: &str, words: &[Span], i: usize) -> bool {
    match words.get(i + 1) {
        Some(next) => {
            is_whitespace_only(&text[words[i].end..next.start])
                && in_list(text, *next, PAIR_FOLLOWERS)
        }
        None => false,
    }
}

/// "the the the" keeps its first "the". Only runs separated by whitespace
/// count: "No, no, I disagree" and "That, that is the question" have
/// punctuation between them, which is the user (or the model) marking them as
/// deliberate.
fn collapse_stutters(text: &str, words: &[Span], del: &mut [bool]) {
    let mut i = 0;
    while i < words.len() {
        if in_list(text, words[i], STUTTER_WORDS) {
            let head = word(text, words[i]);
            let mut j = i + 1;
            while j < words.len()
                && is_whitespace_only(&text[words[j - 1].end..words[j].start])
                && word(text, words[j]).eq_ignore_ascii_case(head)
            {
                j += 1;
            }
            if j > i + 1 {
                for d in del.iter_mut().take(j).skip(i + 1) {
                    *d = true;
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
}

fn strip_hedges(text: &str, words: &[Span], del: &mut [bool]) {
    let mut i = 0;
    while i < words.len() {
        if let Some(h) = HEDGES.iter().find(|h| matches_at(text, words, i, h)) {
            let last = i + h.words.len() - 1;
            if is_parenthetical(text, words, del, i, last, h) {
                for d in del.iter_mut().take(last + 1).skip(i) {
                    *d = true;
                }
                i = last + 1;
                continue;
            }
        }
        i += 1;
    }
}

fn matches_at(text: &str, words: &[Span], i: usize, h: &Hedge) -> bool {
    if i + h.words.len() > words.len() {
        return false;
    }
    for (k, expected) in h.words.iter().enumerate() {
        if !eq_word(word(text, words[i + k]), expected) {
            return false;
        }
        if k > 0 && !is_whitespace_only(&text[words[i + k - 1].end..words[i + k].start]) {
            return false;
        }
    }
    true
}

/// What sits next to a word, once whitespace is ignored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctx {
    /// The start or end of the text, or an opening/closing quote or bracket.
    Edge,
    /// Punctuation, carrying the strength of the strongest character in the run
    /// (comma 1, semicolon or colon 2, sentence-ender 3).
    Punct(u8),
    /// Another word, or anything else. A hedge here is doing work.
    Other,
}

/// The whole false-positive defence, in one function: a hedge is only a filler
/// when it is set off from the sentence on both sides.
fn is_parenthetical(
    text: &str,
    words: &[Span],
    del: &[bool],
    first: usize,
    last: usize,
    h: &Hedge,
) -> bool {
    let left = left_ctx_after_deletions(text, words, del, first);
    let right = ctx_right(text, words[last].end);

    let opens = match left {
        Ctx::Edge => h.start_ok,
        // A comma or semicolon opens an aside for any hedge. A full stop only
        // counts for the discourse markers, which may legitimately open the
        // next sentence.
        Ctx::Punct(s) => s < TERMINAL || h.start_ok,
        Ctx::Other => false,
    };
    let closes = !matches!(right, Ctx::Other);
    if !opens || !closes {
        return false;
    }

    if h.imperative_risk {
        let next = next_surviving(words, del, last);
        // "subscribe, like, and comment" — a list, not an aside.
        if let (Ctx::Punct(s), Some(n)) = (right, next) {
            if s < TERMINAL && in_list(text, n, CONJUNCTIONS) {
                return false;
            }
        }
        // Sentence-initial: only a following clause opener rules out the
        // imperative reading ("Like, I don't know" yes, "Like, comment" no).
        let sentence_initial = matches!(left, Ctx::Edge | Ctx::Punct(TERMINAL));
        if sentence_initial && !next.is_some_and(|n| in_list(text, n, CLAUSE_STARTERS)) {
            return false;
        }
    }
    true
}

/// [`ctx_left`], but looking through words that are already going to be
/// deleted, because the hedge's real neighbour is whatever survives.
///
/// Without this, "um, like, comment and subscribe" reads its left context as a
/// comma, decides "like" is mid-sentence, and deletes the imperative — the one
/// mistake this module exists to avoid. With it, the context is the start of
/// the utterance, which is where "like" is ambiguous and therefore left alone.
fn left_ctx_after_deletions(text: &str, words: &[Span], del: &[bool], first: usize) -> Ctx {
    /// Nobody says thirty-two fillers in a row. The cap keeps a pathological
    /// input from making this walk quadratic, and `Other` — "leave it alone" —
    /// is the safe answer to give up with.
    const MAX_LOOKBACK: usize = 32;

    let mut k = first;
    let mut punct = 0u8;
    let mut reached_a_survivor = false;
    for _ in 0..MAX_LOOKBACK {
        match ctx_left(text, words[k].start) {
            Ctx::Edge => return Ctx::Edge,
            Ctx::Punct(s) => punct = punct.max(s),
            Ctx::Other => {}
        }
        match k.checked_sub(1) {
            Some(p) if del[p] && is_separator(&text[words[p].end..words[k].start]) => k = p,
            _ => {
                reached_a_survivor = true;
                break;
            }
        }
    }
    match (reached_a_survivor, punct) {
        (true, 1..) => Ctx::Punct(punct),
        _ => Ctx::Other,
    }
}

/// The first word after `last` that is not already being deleted.
fn next_surviving(words: &[Span], del: &[bool], last: usize) -> Option<Span> {
    (last + 1..words.len()).find(|k| !del[*k]).map(|k| words[k])
}

fn ctx_left(text: &str, start: usize) -> Ctx {
    let ws = skip_ws_left(text, start);
    let p = punct_start_left(text, ws);
    if p < ws {
        return Ctx::Punct(strength(&text[p..ws]));
    }
    if ws == 0 || is_open(prev_char(text, ws)) {
        return Ctx::Edge;
    }
    Ctx::Other
}

fn ctx_right(text: &str, end: usize) -> Ctx {
    let ws = skip_ws_right(text, end);
    let p = punct_end_right(text, ws);
    if p > ws {
        return Ctx::Punct(strength(&text[ws..p]));
    }
    if ws == text.len() || is_close(char_at(text, ws)) {
        return Ctx::Edge;
    }
    Ctx::Other
}

// ---------------------------------------------------------------- separators

fn is_whitespace_only(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

/// Punctuation strength. A filler sits in a pause, the pause is written as
/// punctuation, and after the filler goes exactly one mark survives to stand
/// for it: the strongest one adjacent to or inside the removed run.
const DASH: u8 = 1;
const COMMA: u8 = 2;
const CLAUSE: u8 = 3;
const TERMINAL: u8 = 4;

/// Punctuation this transform is allowed to move or drop when it splices.
/// Everything else — quotes, emoji, and **the ASCII hyphen** — is left exactly
/// where the user put it.
///
/// The hyphen is the interesting exclusion. An em or en dash around a filler is
/// the filler's own bracketing ("it was — um — complicated"), but `-` at the
/// start of a line is a *list marker* that `spoken` (#45) synthesized from the
/// user saying "bullet", and swallowing it destroys the list they just
/// dictated.
fn strength_of(c: char) -> u8 {
    match c {
        '\u{2013}' | '\u{2014}' => DASH,
        ',' => COMMA,
        ';' | ':' => CLAUSE,
        '.' | '!' | '?' | '\u{2026}' => TERMINAL,
        _ => 0,
    }
}

fn strength(run: &str) -> u8 {
    run.chars().map(strength_of).max().unwrap_or(0)
}

fn is_open(c: Option<char>) -> bool {
    matches!(
        c,
        Some('"' | '\'' | '\u{201C}' | '\u{2018}' | '(' | '[' | '{' | '\u{00AB}')
    )
}

fn is_close(c: char) -> bool {
    matches!(
        c,
        '"' | '\'' | '\u{201D}' | '\u{2019}' | ')' | ']' | '}' | '\u{00BB}'
    )
}

fn prev_char(text: &str, at: usize) -> Option<char> {
    text[..at].chars().next_back()
}

fn skip_ws_left(text: &str, mut at: usize) -> usize {
    while let Some(c) = prev_char(text, at) {
        if !c.is_whitespace() {
            break;
        }
        at -= c.len_utf8();
    }
    at
}

fn skip_ws_right(text: &str, mut at: usize) -> usize {
    while at < text.len() {
        let c = char_at(text, at);
        if !c.is_whitespace() {
            break;
        }
        at += c.len_utf8();
    }
    at
}

fn punct_start_left(text: &str, mut at: usize) -> usize {
    while let Some(c) = prev_char(text, at) {
        if strength_of(c) == 0 {
            break;
        }
        at -= c.len_utf8();
    }
    at
}

fn punct_end_right(text: &str, mut at: usize) -> usize {
    while at < text.len() {
        let c = char_at(text, at);
        if strength_of(c) == 0 {
            break;
        }
        at += c.len_utf8();
    }
    at
}

// ---------------------------------------------------------------- rebuild

fn rebuild(text: &str, words: &[Span], del: &[bool]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let mut capitalize_next = false;
    let mut i = 0;

    while i < words.len() {
        if !del[i] {
            i += 1;
            continue;
        }
        // One splice per run of deleted words, so "um, uh, hello" does not go
        // through two rounds of punctuation repair.
        let (first, mut last) = (i, i);
        while last + 1 < words.len()
            && del[last + 1]
            && is_separator(&text[words[last].end..words[last + 1].start])
        {
            last += 1;
        }
        i = last + 1;

        let (from, to, replacement) = splice(text, words[first].start, words[last].end);
        debug_assert!(cursor <= from, "splices overlap");
        push_chunk(&mut out, &text[cursor..from], &mut capitalize_next);
        out.push_str(&replacement);
        cursor = to;
        // Restore the capitalization the deletion destroyed — and only that.
        // Where no full stop proves a sentence began (start of the text, of a
        // line, or of a list item), the removed word's own case is the
        // evidence: "Um, the build broke" was a sentence, "um and then I left"
        // is a user dictating into the middle of one, and "- um, buy milk" is
        // an item in a list that was already lowercase.
        capitalize_next = match position(&out) {
            Position::Fresh => starts_uppercase(word(text, words[first])),
            Position::AfterTerminal => true,
            Position::Mid => false,
        };
    }

    push_chunk(&mut out, &text[cursor..], &mut capitalize_next);
    out
}

/// Whitespace and droppable punctuation only — the gap between two deleted
/// words that this transform is willing to swallow whole.
fn is_separator(s: &str) -> bool {
    s.chars().all(|c| c.is_whitespace() || strength_of(c) > 0)
}

/// The region to cut, and what replaces it: `(from, to, replacement)`.
///
/// The region reaches out over whitespace, then over one run of punctuation,
/// then over whitespace again — no further, so a quote or a word always stops
/// it — and over a bracket pair the removed run fills entirely. What comes back
/// is at most one punctuation run plus one stretch of whitespace.
fn splice(text: &str, del_start: usize, del_end: usize) -> (usize, usize, String) {
    // "It was (um) complicated." — the brackets held nothing but the filler, so
    // they go with it. Left in place they read as debris: "It was () ...".
    let (del_start, del_end) = widen_over_brackets(text, del_start, del_end);

    let ws_l = skip_ws_left(text, del_start);
    let punct_l = punct_start_left(text, ws_l);
    let from = if punct_l < ws_l {
        skip_ws_left(text, punct_l)
    } else {
        ws_l
    };

    let ws_r = skip_ws_right(text, del_end);
    let punct_r = punct_end_right(text, ws_r);
    let to = if punct_r > ws_r {
        skip_ws_right(text, punct_r)
    } else {
        ws_r
    };

    let left_punct = Span {
        start: punct_l,
        end: ws_l,
    };
    let right_punct = Span {
        start: ws_r,
        end: punct_r,
    };
    // Punctuation attaches to a word. Not to the start of the text, not to an
    // opening quote, not to an emoji, and not to the "- " a list marker leaves
    // in front of an item — hanging a comma off any of those is debris.
    let no_anchor = !prev_char(text, from).is_some_and(is_word_char);
    let at_end = to == text.len() || is_close(char_at(text, to));

    let region = &text[from..to];
    let mut space = if no_anchor {
        leading_ws(region)
    } else if at_end {
        trailing_ws(region)
    } else {
        merged_ws(region)
    };

    // The strongest mark in or beside the removed run survives, and ties go to
    // the leftmost. Deleting a comma the model wrote is not this transform's
    // business: "However, um, we shipped" is "However, we shipped", never
    // "However we shipped". The *inner* candidate is what keeps a sentence
    // break inside the run — "I think so, um. Uh, what's next?" — from being
    // outvoted by the commas on the outside and silently dropped.
    let inner_punct = strongest_run(text, del_start, del_end);
    // Punctuation never migrates across a line break: the comma in
    // "first line\num, second line" belonged to the filler, and hanging it off
    // the end of the previous line is worse than dropping it.
    let right_punct = if space.contains('\n') {
        Span {
            start: ws_r,
            end: ws_r,
        }
    } else {
        right_punct
    };
    let nothing = Span {
        start: from,
        end: from,
    };

    let mut kept = if no_anchor {
        // Nothing to attach to: "Um. Hello." is one sentence, not an empty one
        // followed by a sentence.
        nothing
    } else {
        [left_punct, inner_punct, right_punct]
            .into_iter()
            .fold(nothing, |best, next| {
                if strength(word(text, next)) > strength(word(text, best)) {
                    next
                } else {
                    best
                }
            })
    };
    // A mark at the very end of the text has nothing left to separate, unless
    // it is the full stop that ends the sentence.
    if at_end && strength(word(text, kept)) < TERMINAL {
        kept = nothing;
    }
    let keep = word(text, kept);
    // A mark keeps the spacing it had: "it was — um — complicated" must not
    // come back as "it was— complicated". Commas are written tight against the
    // word before them and stay that way.
    let pad = if !keep.is_empty() && prev_char(text, kept.start).is_some_and(char::is_whitespace) {
        " "
    } else {
        ""
    };
    // Never weld two words together ("hi,um,there" must not become "hithere").
    // Only when both survivors really are words: zero-width characters are not
    // whitespace, and inserting a space between two of them invents one the
    // user can see.
    if keep.is_empty()
        && space.is_empty()
        && prev_char(text, from).is_some_and(is_word_char)
        && to < text.len()
        && is_word_char(char_at(text, to))
    {
        space = " ";
    }

    (from, to, format!("{pad}{keep}{space}"))
}

/// Widens a deleted run over a bracket pair it fills entirely, so "(um)" leaves
/// as one thing rather than leaving "()" behind. Repeats, so "((um))" goes too.
///
/// Quotes are not brackets here: `'` is an apostrophe more often than it is a
/// quotation mark, and a quoted filler is more plausibly a quotation of someone
/// than debris.
fn widen_over_brackets(text: &str, mut start: usize, mut end: usize) -> (usize, usize) {
    loop {
        let l = skip_ws_left(text, start);
        let r = skip_ws_right(text, end);
        let (Some(open), Some(close)) = (prev_char(text, l), text[r..].chars().next()) else {
            return (start, end);
        };
        let pair = matches!(
            (open, close),
            ('(', ')') | ('[', ']') | ('{', '}') | ('\u{00AB}', '\u{00BB}')
        );
        if !pair {
            return (start, end);
        }
        start = l - open.len_utf8();
        end = r + close.len_utf8();
    }
}

/// The strongest run of droppable punctuation within `text[start..end)`; an
/// empty span when there is none. Ties go to the leftmost run, so "um. uh!"
/// keeps the full stop that actually ended the sentence.
fn strongest_run(text: &str, start: usize, end: usize) -> Span {
    let mut best = Span { start, end: start };
    let mut idx = start;
    while idx < end {
        let c = char_at(text, idx);
        if strength_of(c) == 0 {
            idx += c.len_utf8();
            continue;
        }
        let run_end = punct_end_right(text, idx).min(end);
        let run = Span {
            start: idx,
            end: run_end,
        };
        if strength(word(text, run)) > strength(word(text, best)) {
            best = run;
        }
        idx = run_end;
    }
    best
}

fn leading_ws(region: &str) -> &str {
    &region[..skip_ws_right(region, 0)]
}

fn trailing_ws(region: &str) -> &str {
    &region[skip_ws_left(region, region.len())..]
}

/// One space between the survivors — unless a line break was in there, in which
/// case the line break stays, because a paragraph is structure and not spacing.
fn merged_ws(region: &str) -> &str {
    let mut best: Option<(usize, usize, usize)> = None; // (newlines, start, end)
    let mut idx = 0;
    let mut any = false;
    while idx < region.len() {
        let c = char_at(region, idx);
        if !c.is_whitespace() {
            idx += c.len_utf8();
            continue;
        }
        let start = idx;
        let end = skip_ws_right(region, idx);
        idx = end;
        any = true;
        let newlines = region[start..end].matches('\n').count();
        if newlines > 0 && newlines > best.map_or(0, |b| b.0) {
            best = Some((newlines, start, end));
        }
    }
    match best {
        Some((_, s, e)) => &region[s..e],
        None if any => " ",
        None => "",
    }
}

/// Where the text built so far leaves the word about to be copied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Position {
    /// The start of the text, of a line, or of a list item. Nothing here proves
    /// a sentence began, so the removed word's own case decides.
    Fresh,
    /// Straight after a full stop: the next word starts a sentence.
    AfterTerminal,
    /// Inside a sentence.
    Mid,
}

/// An ellipsis is not a full stop. "Wait... um, yes" is one sentence trailing
/// off, and "Wait... Yes" reads as two.
///
/// A list marker is not a full stop either, even though "1." ends in one. That
/// matters because `spoken` (#45) runs before this transform and synthesizes
/// `- ` and `1. ` from dictated "bullet"/"number" — capitalizing off the back of
/// a marker would be this transform inventing a style decision for a line it
/// only passed through.
fn position(out: &str) -> Position {
    let line = match out.rfind('\n') {
        Some(i) => &out[i + 1..],
        None => out,
    };
    let head = line.trim();
    if head.is_empty() || is_list_marker(head) {
        return Position::Fresh;
    }
    let mut it = head.chars().rev().skip_while(|c| is_open(Some(*c)));
    let Some(last) = it.next() else {
        return Position::Fresh;
    };
    if !matches!(last, '.' | '!' | '?') {
        return Position::Mid;
    }
    if matches!((last, it.next()), ('.', Some('.'))) {
        return Position::Mid; // "..."
    }
    Position::AfterTerminal
}

/// A bullet, or an ordered-list number — the shapes `spoken` (#45) emits.
fn is_list_marker(s: &str) -> bool {
    if matches!(s, "-" | "*" | "+" | "\u{2022}") {
        return true;
    }
    let digits = s.trim_end_matches(['.', ')']);
    !digits.is_empty() && digits.len() < s.len() && digits.bytes().all(|b| b.is_ascii_digit())
}

fn starts_uppercase(w: &str) -> bool {
    w.chars().next().is_some_and(char::is_uppercase)
}

/// Copies `chunk`, capitalizing its first word when a deletion left that word
/// standing at the start of a sentence.
fn push_chunk(out: &mut String, chunk: &str, capitalize: &mut bool) {
    if !*capitalize {
        out.push_str(chunk);
        return;
    }
    *capitalize = false;

    let mut start = None;
    let mut idx = 0;
    while idx < chunk.len() {
        let c = char_at(chunk, idx);
        if is_word_char(c) {
            start = Some(idx);
            break;
        }
        if !(c.is_whitespace() || is_open(Some(c))) {
            break; // something else opens the sentence; leave it alone
        }
        idx += c.len_utf8();
    }
    let Some(start) = start else {
        out.push_str(chunk);
        return;
    };
    let end = scan_word(chunk, start);
    match capitalized(&chunk[start..end]) {
        Some(w) => {
            out.push_str(&chunk[..start]);
            out.push_str(&w);
            out.push_str(&chunk[end..]);
        }
        None => out.push_str(chunk),
    }
}

/// `None` when the word should be left exactly as it is: already capitalized,
/// not a letter at all, or carrying an uppercase somewhere else — "iPhone" and
/// "eBay" are spelled that way on purpose.
fn capitalized(w: &str) -> Option<String> {
    let first = w.chars().next()?;
    if !first.is_lowercase() || w.chars().any(char::is_uppercase) {
        return None;
    }
    let mut upper = first.to_uppercase();
    // Skip anything whose uppercase is more than one character ("ß" → "SS"):
    // growing the word is more surprising than leaving it lowercase.
    let (c, rest) = (upper.next()?, upper.next());
    if rest.is_some() {
        return None;
    }
    let mut out = String::with_capacity(w.len() + 1);
    out.push(c);
    out.push_str(&w[first.len_utf8()..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{prefix_violation, torture_inputs, truncate};

    fn at(level: FillerLevel, text: &str) -> String {
        Fillers::new(FillersConfig {
            enabled: true,
            level,
        })
        .apply(text)
    }
    fn off(text: &str) -> String {
        at(FillerLevel::Off, text)
    }
    fn light(text: &str) -> String {
        at(FillerLevel::Light, text)
    }
    fn medium(text: &str) -> String {
        at(FillerLevel::Medium, text)
    }
    const LEVELS: [FillerLevel; 3] = [FillerLevel::Off, FillerLevel::Light, FillerLevel::Medium];

    /// Utterances that must survive every level byte for byte. This is the test
    /// the issue is really about: every hedge `medium` knows, in the position
    /// where it means something.
    const LOAD_BEARING: &[&str] = &[
        // "like": verb, preposition, noun, and the imperative that makes
        // sentence-initial "like" ambiguous
        "I like this design",
        "I really like it",
        "Do you like the new one?",
        "I would like to schedule a call",
        "It looks like rain",
        "That tastes like chicken",
        "Something like this would work",
        "He runs like the wind",
        "She was like a sister to me",
        "It feels like home, honestly.",
        "like-minded people showed up",
        "Give the post a like, please.",
        "Things I like, and things I do not.",
        "Subscribe, like, and comment.",
        "Like, comment, and subscribe.",
        "The like button is broken.",
        "There is nothing I like.",
        // "you know"
        "you know what you did",
        "You know the answer already.",
        "Do you know where it is?",
        "As you know, the deadline is Friday.",
        "If you know, tell me.",
        "You know it is true.",
        "Let me know if you know anything.",
        // "I mean"
        "I mean it",
        "What do you mean?",
        "I mean business.",
        "That is not what I mean.",
        "I said Tuesday, I mean Wednesday",
        "Do you know what I mean?",
        // "sort of"
        "sort of blue",
        "It was sort of blue.",
        "What sort of person does that?",
        "some sort of error",
        "a sort of homecoming",
        "Sort the list, of course.",
        // "basically" and "actually"
        "Basically, the answer is no",
        "The design is basically sound.",
        "It basically works.",
        "They are basically identical.",
        "it was actually correct",
        "Actually, I disagree.",
        "Did it actually ship?",
        "I actually like this design.",
        "What actually happened?",
        // repetition that is not a stutter
        "I had had enough by then.",
        "The fact that that happened is the problem.",
        "What it is is a scheduling problem.",
        "No, no, I disagree.",
        "It was very very good.",
        "She came in in a hurry.",
        "A can can hold two litres.",
        "That, that is the question.",
        "There, there, it will be fine.",
        "I do do my own taxes.",
        // hesitations that are parts of words, or words themselves
        "an umbrella and a hum",
        "aluminum siding",
        "Umberto called back.",
        "uh-huh, that works",
        "uh huh, that works",
        "Herbert made two errors.",
        "A 5 mm gap is fine.",
        "To err is human.",
    ];

    // ---- level off ---------------------------------------------------------

    #[test]
    fn off_is_byte_identical() {
        for input in torture_inputs() {
            assert_eq!(off(&input), input, "off changed {:?}", truncate(&input));
        }
        for input in LOAD_BEARING {
            assert_eq!(&off(input), input);
        }
        for input in ["um, uh, the the thing", "It was, like, cold."] {
            assert_eq!(off(input), input);
        }
    }

    #[test]
    fn off_is_the_default_and_disabled_is_the_default() {
        let cfg = FillersConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.level, FillerLevel::Off);
        // enabled on its own must still be a no-op: two decisions, not one
        assert_eq!(
            Fillers::new(FillersConfig {
                enabled: true,
                ..Default::default()
            })
            .apply("um, hello"),
            "um, hello"
        );
    }

    // ---- the false-positive corpus ----------------------------------------

    #[test]
    fn load_bearing_words_survive_every_level() {
        for level in LEVELS {
            for input in LOAD_BEARING {
                assert_eq!(
                    &at(level, input),
                    input,
                    "{level} rewrote a load-bearing utterance"
                );
            }
        }
    }

    /// The same corpus with a real filler bolted on: the filler goes, the
    /// load-bearing words stay. Catches a rule that only looks safe because it
    /// never fires, and catches a filler that drags its neighbour out with it.
    #[test]
    fn load_bearing_words_survive_next_to_a_real_filler() {
        for input in LOAD_BEARING {
            // lowercase filler: nothing proves a sentence started, so the
            // survivor keeps its own case
            assert_eq!(
                medium(&format!("um, {input}")),
                *input,
                "after a leading filler"
            );
            // capitalized filler: the survivor inherits the sentence start
            assert_eq!(
                medium(&format!("Um, {input}")),
                capitalized_first(input),
                "after a leading capitalized filler"
            );
            // and at the other end
            assert_eq!(
                medium(&format!("{input} uh")),
                format!("{input}"),
                "before a filler"
            );
        }
    }

    fn capitalized_first(s: &str) -> String {
        let end = scan_word(s, 0);
        match capitalized(&s[..end]) {
            Some(w) => format!("{w}{}", &s[end..]),
            None => s.to_string(),
        }
    }

    // ---- light -------------------------------------------------------------

    #[test]
    fn light_removes_hesitations() {
        let cases = [
            ("Um, hello there", "Hello there"),
            ("um, hello there", "hello there"),
            (
                "So, um, I think we should ship it",
                "So, I think we should ship it",
            ),
            // the accepted cost of never deleting a comma the model wrote:
            // this one reads better without it, and we keep it anyway
            ("It was, uh, complicated.", "It was, complicated."),
            ("hello um", "hello"),
            ("Um. Hello.", "Hello."),
            ("Er, well, maybe", "Well, maybe"),
            ("I think uh we should go", "I think we should go"),
            ("Uhm the build is green", "The build is green"),
            ("Mmm that is good", "That is good"),
            ("It works, um.", "It works."),
            ("Ship it um!", "Ship it!"),
            ("UM, hello", "Hello"),
        ];
        for (input, want) in cases {
            assert_eq!(light(input), want, "on {input:?}");
        }
    }

    /// Regression, adversarial review of PR #72. The old rule threw away a
    /// matched pair of commas on the theory that it was the filler's own
    /// bracketing. It is not: a comma before a filler belongs to whatever came
    /// before it just as often — a sentence adverbial, a vocative, a list item
    /// — and deleting it is deleting something the user said.
    #[test]
    fn a_comma_the_model_wrote_is_never_deleted() {
        let cases = [
            ("However, um, we shipped.", "However, we shipped."),
            ("Hello, um, John.", "Hello, John."),
            ("First, um, second, and third.", "First, second, and third."),
            ("Yes, um, I agree.", "Yes, I agree."),
            ("Right, uh, moving on.", "Right, moving on."),
            ("Anyway, um, we shipped it.", "Anyway, we shipped it."),
        ];
        for (input, want) in cases {
            assert_eq!(light(input), want, "on {input:?}");
        }
    }

    /// Regression, adversarial review of PR #72. `splice` used to compare only
    /// the punctuation on the two *outer* edges of a deleted run, so a full
    /// stop *between* two hesitations lost to the commas around them and two
    /// sentences were welded into one. "hesitation, restart, hesitation" is
    /// exactly how people rethink a sentence, so this fired constantly.
    #[test]
    fn a_sentence_break_inside_the_removed_run_survives() {
        let cases = [
            (
                "I think so, um. Uh, what's next?",
                "I think so. What's next?",
            ),
            ("Is it done, um? Uh, yes.", "Is it done? Yes."),
            ("We shipped, um! Uh, yesterday.", "We shipped! Yesterday."),
            ("Right, um... uh, later.", "Right... later."),
            ("It works, um; uh, mostly.", "It works; mostly."),
        ];
        for (input, want) in cases {
            assert_eq!(light(input), want, "on {input:?}");
        }
    }

    /// Regression, adversarial review of PR #72. "Dashes and brackets bound the
    /// repair" was true and still produced corrupt output, because bounding it
    /// left the delimiters standing with nothing between them.
    #[test]
    fn dashes_and_brackets_do_not_leave_debris() {
        let cases = [
            (
                "It was \u{2014} um \u{2014} complicated.",
                "It was \u{2014} complicated.",
            ),
            ("It was (um) complicated.", "It was complicated."),
            ("It was [um] complicated.", "It was complicated."),
            ("It was ((um)) complicated.", "It was complicated."),
            ("It was fine \u{2014} um.", "It was fine."),
            ("It was fine \u{2014} um", "It was fine"),
            ("It was \u{2013} uh \u{2013} fine.", "It was \u{2013} fine."),
            // the brackets held more than the filler, so they stay
            (
                "It was (um yeah) complicated.",
                "It was (yeah) complicated.",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(light(input), want, "on {input:?}");
        }
    }

    /// `spoken` (#45) runs two places earlier in the chain and synthesizes
    /// `\n\n`, `- `, `1. ` and `.` from dictated words. By the time text
    /// reaches this transform they are indistinguishable from model output, so
    /// the repair has to leave every one of them intact.
    #[test]
    fn structure_synthesized_by_spoken_survives() {
        // untouched: no filler anywhere in them
        for input in [
            "- buy milk\n- walk the dog",
            "1. first item\n2. second item",
            "first para\n\nsecond para",
            "we shipped it.",
            "- buy milk\n- buy milk",
            "1. the first\n2. the second",
        ] {
            assert_eq!(&light(input), input, "on {input:?}");
        }
        // a filler inside the structure: the filler goes, the structure stays
        let cases = [
            ("- um, buy milk", "- buy milk"),
            ("- Um, buy milk", "- Buy milk"),
            ("1. um, first item", "1. first item"),
            ("1. Um, first item", "1. First item"),
            ("- buy um, milk", "- buy, milk"),
            (
                "- um, buy milk\n- uh, walk the dog",
                "- buy milk\n- walk the dog",
            ),
            ("first para\n\num, second para", "first para\n\nsecond para"),
            ("we shipped it. um", "we shipped it."),
            ("um. we shipped it.", "we shipped it."),
            ("we shipped, um, it.", "we shipped, it."),
        ];
        for (input, want) in cases {
            assert_eq!(light(input), want, "on {input:?}");
        }
    }

    /// The one case where this transform could plausibly have taken over
    /// #45's deferred lowercase list items — and deliberately does not. A list
    /// item's case is #45's decision to make; all this transform promises is
    /// not to change it, in either direction, when it removes a filler from
    /// the front of one.
    #[test]
    fn list_item_case_is_left_to_spoken() {
        assert_eq!(light("- um, buy milk"), "- buy milk");
        assert_eq!(light("- walk the dog"), "- walk the dog");
        assert_eq!(light("1. um, first item"), "1. first item");
        assert_eq!(light("* um, star bullet"), "* star bullet");
    }

    #[test]
    fn snake_case_identifiers_are_one_word() {
        assert_eq!(light("Um, foo_bar is broken."), "Foo_bar is broken.");
        assert_eq!(light("the um_var is unset"), "the um_var is unset");
        assert_eq!(light("call er_handler now"), "call er_handler now");
        let got: Vec<&str> = words("um_var and foo_bar")
            .into_iter()
            .map(|s| word("um_var and foo_bar", s))
            .collect();
        assert_eq!(got, ["um_var", "and", "foo_bar"]);
    }

    #[test]
    fn light_collapses_stutters() {
        let cases = [
            ("the the the thing", "the thing"),
            ("I I think so", "I think so"),
            ("we we need to to go", "we need to go"),
            ("The The Thing", "The Thing"),
            ("and and then it broke", "and then it broke"),
            ("a a a bug", "a bug"),
            ("it is the the same", "it is the same"),
        ];
        for (input, want) in cases {
            assert_eq!(light(input), want, "on {input:?}");
        }
    }

    #[test]
    fn light_leaves_hedges_alone() {
        for input in [
            "It was, like, really cold.",
            "I think, you know, we should go.",
            "I mean, we could try again.",
            "It is, sort of, done.",
            "We, basically, need more time.",
        ] {
            assert_eq!(&light(input), input, "light stripped a hedge");
        }
    }

    // ---- medium (gated; see FillerLevel::parse and #74) ---------------------
    //
    // These record what the gated level does today, not a shipping contract.
    // They stay because `light` shares the machinery and because whoever picks
    // #74 up needs to see both halves: the cases the design gets right, and
    // `medium_still_has_the_false_positives_that_gate_it` next door.

    #[test]
    fn medium_removes_parenthetical_hedges() {
        let cases = [
            ("It was, like, really cold.", "It was, really cold."),
            ("I think, you know, we should go.", "I think, we should go."),
            ("You know, the thing is broken.", "The thing is broken."),
            ("I mean, we could try again.", "We could try again."),
            ("Like, I do not know.", "I do not know."),
            ("It is, sort of, done.", "It is, done."),
            ("We, basically, need more time.", "We, need more time."),
            ("It was, actually, quite good.", "It was, quite good."),
            ("It was cold, like.", "It was cold."),
            ("It is fine, you know.", "It is fine."),
            ("Um, I mean, like, the thing", "The thing"),
            ("So, like, it works", "So, it works"),
        ];
        for (input, want) in cases {
            assert_eq!(medium(input), want, "on {input:?}");
        }
    }

    /// The collision with #48, and the reason the gate helps it too.
    ///
    /// `light` — the level that ships — cannot touch "I mean" in any form, so
    /// with `self_correct` still a stub on `main` there is no way for filler
    /// removal to eat a correction marker. At `medium` the comma-less marker
    /// form is still left alone, but the comma-closed one
    /// ("Tuesday, I mean, Wednesday") is not, and that is one of the false
    /// positives holding the level back.
    #[test]
    fn light_cannot_touch_a_correction_marker_at_all() {
        for input in [
            "meet Tuesday, I mean Wednesday",
            "Send it to Bob, I mean Rob.",
            "It is on the 3rd, I mean the 4th.",
            "Let's meet Tuesday, I mean, Wednesday.",
            "I mean, we could try again.",
        ] {
            assert_eq!(&light(input), input, "light took a #48 correction marker");
        }
        // at medium the comma-less form is still safe...
        assert_eq!(
            medium("meet Tuesday, I mean Wednesday"),
            "meet Tuesday, I mean Wednesday"
        );
        // ...and the comma-closed one is not, which is #74's problem to solve
        assert_eq!(
            medium("Let's meet Tuesday, I mean, Wednesday."),
            "Let's meet Tuesday, Wednesday."
        );
        // What #48 leaves behind is already clean, and stays clean here.
        assert_eq!(light("meet Wednesday"), "meet Wednesday");
    }

    #[test]
    fn hedges_outside_parenthetical_position_survive() {
        // one comma is not enough on either side
        let cases = [
            "I like, honestly, nothing about it", // "like" opens nothing
            "You know the drill, obviously.",
            "It is sort of, well, complicated.", // "sort of" has no opening comma
            "Basically, we need more time.",
            "Actually, that is wrong.",
        ];
        for input in cases {
            assert_eq!(&medium(input), input, "on {input:?}");
        }
    }

    // ---- adversarial -------------------------------------------------------

    #[test]
    fn an_utterance_of_nothing_but_fillers_is_left_alone() {
        for input in [
            "um",
            "Um.",
            "um, uh",
            "um uh er",
            "um, um, um.",
            "Like,",
            "I mean,",
            "you know",
            "uh-",
            "Um... uh?",
        ] {
            assert_eq!(&medium(input), input, "emptied {input:?}");
            assert!(!medium(input).is_empty());
        }
    }

    #[test]
    fn fillers_at_both_ends() {
        assert_eq!(light("um hello world uh"), "hello world");
        assert_eq!(light("Um, hello world, uh."), "Hello world.");
        assert_eq!(medium("Like, we shipped it, you know."), "We shipped it.");
        assert_eq!(light("   um hello   "), "   hello   ");
        assert_eq!(light("um\nhello"), "hello");
    }

    #[test]
    fn hesitations_inside_words_are_untouched() {
        for input in [
            "umbrella",
            "hum",
            "aluminum",
            "Umberto",
            "um-hum",
            "erm-no",
            "summer",
            "the drum uh beat",
        ] {
            let want = if input == "the drum uh beat" {
                "the drum beat".to_string()
            } else {
                input.to_string()
            };
            assert_eq!(light(input), want, "on {input:?}");
        }
    }

    #[test]
    fn stutters_do_not_span_punctuation() {
        for input in [
            "the, the thing",
            "The. The thing.",
            "and — and then",
            "it is a; a problem",
        ] {
            assert_eq!(&light(input), input, "collapsed across punctuation");
        }
    }

    #[test]
    fn capitalization_is_repaired_only_where_a_filler_was_removed() {
        assert_eq!(light("Um, the thing broke"), "The thing broke");
        assert_eq!(light("Yes. Um, the thing broke"), "Yes. The thing broke");
        assert_eq!(light("Yes, um, the thing broke"), "Yes, the thing broke");
        assert_eq!(
            medium("You know, iPhone sales are up."),
            "iPhone sales are up."
        );
        assert_eq!(medium("You know, 5 people came."), "5 people came.");
        assert_eq!(light("Um, élan is a word"), "Élan is a word");
        assert_eq!(light("Um, 日本語です"), "日本語です");
        // untouched sentences keep their own capitalization, wrong or not
        assert_eq!(
            light("Um, hello. and then i left"),
            "Hello. and then i left"
        );
        // a lowercase filler is no evidence that a sentence started here — the
        // user may be dictating into the middle of one
        assert_eq!(light("um and then i left"), "and then i left");
    }

    #[test]
    fn punctuation_is_repaired_not_orphaned() {
        let cases = [
            ("Wait... um, yes", "Wait... yes"),
            ("Really? um, yes", "Really? Yes"),
            ("one, um; two", "one; two"),
            ("hi,um,there", "hi,there"),
            ("It is fine, um!", "It is fine!"),
            ("I agree. Um, uh. Let's go.", "I agree. Let's go."),
            ("done, um", "done"),
            ("done. um", "done."),
            ("It was, uh, fine, um, honestly.", "It was, fine, honestly."),
        ];
        for (input, want) in cases {
            assert_eq!(light(input), want, "on {input:?}");
        }
    }

    #[test]
    fn quotes_and_brackets_bound_the_repair() {
        let cases = [
            (r#"He said, "um, hello""#, r#"He said, "hello""#),
            (
                r#"He said "hello, um" and left"#,
                r#"He said "hello" and left"#,
            ),
        ];
        for (input, want) in cases {
            assert_eq!(light(input), want, "on {input:?}");
        }
    }

    #[test]
    fn line_structure_survives() {
        assert_eq!(
            light("first line um\nsecond line"),
            "first line\nsecond line"
        );
        assert_eq!(
            light("first paragraph\n\num, second paragraph"),
            "first paragraph\n\nsecond paragraph"
        );
        assert_eq!(
            light("First paragraph.\n\nUm, second paragraph"),
            "First paragraph.\n\nSecond paragraph"
        );
        assert_eq!(light("a um\n\nb"), "a\n\nb");
        assert_eq!(light("trailing um\n"), "trailing\n");
    }

    // ---- properties --------------------------------------------------------

    #[test]
    fn every_level_is_idempotent() {
        let corpus: Vec<String> = torture_inputs()
            .into_iter()
            .chain(LOAD_BEARING.iter().map(|s| s.to_string()))
            .chain(
                [
                    "um, I mean, like, the thing",
                    "So, um, the the thing, you know.",
                    "Like, I do not know, you know.",
                    "um, uh, er, hello, um.",
                ]
                .iter()
                .map(|s| s.to_string()),
            )
            .collect();
        for level in LEVELS {
            for input in &corpus {
                let once = at(level, input);
                assert_eq!(
                    at(level, &once),
                    once,
                    "{level} is not idempotent on {:?}",
                    truncate(input)
                );
            }
        }
    }

    #[test]
    fn torture_inputs_are_handled() {
        for input in torture_inputs() {
            for level in LEVELS {
                let out = at(level, &input);
                // the only two torture inputs with a filler in them
                let expected_change = matches!(
                    input.as_str(),
                    "um, I mean, like, the thing" | "I said Tuesday, I mean Wednesday"
                );
                if !expected_change {
                    assert_eq!(out, input, "{level} changed {:?}", truncate(&input));
                }
            }
        }
        assert_eq!(
            light("um, I mean, like, the thing"),
            "I mean, like, the thing"
        );
        assert_eq!(medium("um, I mean, like, the thing"), "the thing");
        assert_eq!(
            medium("I said Tuesday, I mean Wednesday"),
            "I said Tuesday, I mean Wednesday"
        );
    }

    #[test]
    fn output_is_always_valid_and_never_grows() {
        for input in torture_inputs() {
            for level in LEVELS {
                let out = at(level, &input);
                assert!(
                    out.len() <= input.len(),
                    "{level} grew {:?}",
                    truncate(&input)
                );
                // no double spaces introduced where the input had none
                if !input.contains("  ") {
                    assert!(!out.contains("  "), "{level} doubled a space");
                }
            }
        }
    }

    /// Every level is a superset of the one below it, which is what "graded"
    /// means and what a Settings slider (#49) will assume. Asserted on the
    /// marks rather than the output, because that is where the property lives:
    /// two levels can splice the same deletions into different punctuation.
    #[test]
    fn medium_deletes_everything_light_does() {
        let corpus: Vec<&str> = LOAD_BEARING
            .iter()
            .copied()
            .chain([
                "um, the the thing",
                "So, um, I think, you know, we should ship it",
                "uh, I I mean, like, tomorrow",
                "It was, uh, sort of, fine.",
            ])
            .collect();
        for input in corpus {
            let ws = words(input);
            let none = mark(input, &ws, FillerLevel::Off);
            let some = mark(input, &ws, FillerLevel::Light);
            let more = mark(input, &ws, FillerLevel::Medium);
            assert!(none.iter().all(|d| !d), "off marked something on {input:?}");
            for i in 0..ws.len() {
                assert!(!some[i] || more[i], "medium kept word {i} of {input:?}");
            }
        }
    }

    /// Every truncation of a real utterance, run through both levels: the
    /// streaming loop (#50) will hand this transform half-finished text, and a
    /// panic there is a dictation that never arrives. Also the cheapest fuzz
    /// available for the byte-slicing this module does.
    #[test]
    fn every_prefix_of_an_utterance_is_safe() {
        let inputs = [
            "So, um, I mean, like, the the thing, you know.",
            "He said, \"um, hello\" and left.",
            "naïve café, um, résumé… um!",
            "👩‍💻 um, shipped it 🚀, you know.",
            "\u{200b}um\u{200b}, zero width",
            "первое, um, второе",
        ];
        for input in inputs {
            for level in LEVELS {
                for (i, _) in input.char_indices().chain([(input.len(), ' ')]) {
                    let prefix = &input[..i];
                    let out = at(level, prefix);
                    assert_eq!(at(level, &out), out, "{level} drifted on {prefix:?}");
                }
            }
        }
    }

    #[test]
    fn unicode_neighbours_are_left_where_they_are() {
        assert_eq!(medium("👩‍💻 um shipped it 🚀"), "👩‍💻 shipped it 🚀");
        assert_eq!(light("shipped it, um, 🚀"), "shipped it, 🚀");
        assert_eq!(light("naïve café, um, résumé"), "naïve café, résumé");
        assert_eq!(light("первое um второе"), "первое второе");
        assert_eq!(light("日本語 um です"), "日本語 です");
        // U+200B is not whitespace, so "um" stands alone between two invisible
        // characters and goes — and nothing is welded or invented in its place.
        assert_eq!(light("\u{200b}um\u{200b} hello"), "\u{200b}\u{200b} hello");
    }

    #[test]
    fn apply_is_deterministic() {
        let input = "So, um, I mean, like, the the thing, you know.";
        let first = medium(input);
        for _ in 0..100 {
            assert_eq!(medium(input), first);
        }
    }

    // ---- the chain, with this transform actually doing something -----------

    /// `lib.rs` proves the chain is idempotent and prefix-stable-free over the
    /// torture corpus, but it builds its config with `testing::cfg_with`, which
    /// only flips `enabled` — so for this transform every one of those tests
    /// runs at `Off` and proves nothing. `testing.rs` is shared by six parallel
    /// issues and must not grow a per-transform knob, so the level-aware half
    /// of that coverage lives here instead.
    fn chain_at(level: FillerLevel) -> crate::Polish {
        let cfg = crate::PolishConfig {
            fillers: FillersConfig {
                enabled: true,
                level,
            },
            ..Default::default()
        };
        let chain = crate::Polish::from_config(&cfg);
        assert_eq!(chain.names(), ["fillers"]);
        chain
    }

    #[test]
    fn the_chain_is_idempotent_with_a_level_actually_set() {
        for level in LEVELS {
            let chain = chain_at(level);
            for input in torture_inputs() {
                let once = chain.apply(&input);
                assert_eq!(
                    chain.apply(&once),
                    once,
                    "{level} in a chain is not idempotent on {:?}",
                    truncate(&input)
                );
            }
        }
    }

    /// The streaming promise (#50), checked at a level that deletes: a chain
    /// containing this transform must still type exactly what the model said.
    #[test]
    fn the_chain_runs_nothing_prefix_stable_at_any_level() {
        for level in LEVELS {
            let chain = chain_at(level);
            assert!(chain.has_rewriting_transforms(), "{level} must warn #50");
            for input in torture_inputs() {
                assert_eq!(
                    chain.apply_prefix_stable(&input),
                    input,
                    "{level} polished a streaming pass on {:?}",
                    truncate(&input)
                );
            }
        }
    }

    /// The seam's byte-identity promise, at the only level a default config can
    /// reach: `off` through the real chain, not just through `apply`.
    #[test]
    fn the_chain_is_byte_identical_at_off() {
        let chain = chain_at(FillerLevel::Off);
        for input in torture_inputs() {
            assert_eq!(chain.apply(&input), input, "on {:?}", truncate(&input));
        }
    }

    // ---- prefix stability --------------------------------------------------

    #[test]
    fn is_not_prefix_stable() {
        assert!(!Fillers::new(FillersConfig::default()).prefix_stable());
    }

    /// Run against the real implementation, as the seam asks. The
    /// counterexample is smaller than "it deletes a word": one character into
    /// "um", before anything has been deleted at all, the streaming pass has
    /// already typed text the finished utterance does not contain.
    #[test]
    fn prefix_violation_finds_the_documented_counterexample() {
        let t = Fillers::new(FillersConfig {
            enabled: true,
            level: FillerLevel::Light,
        });
        let (prefix, got, whole) = prefix_violation(&t, "so um yeah").expect("must violate");
        assert_eq!(
            (prefix.as_str(), got.as_str(), whole.as_str()),
            ("so u", "so u", "so yeah")
        );

        // and the medium hedges violate it too, one character after the comma
        // the streaming pass has already typed
        let m = Fillers::new(FillersConfig {
            enabled: true,
            level: FillerLevel::Medium,
        });
        let (prefix, got, whole) =
            prefix_violation(&m, "it was, like, cold").expect("must violate");
        assert_eq!(
            (prefix.as_str(), got.as_str(), whole.as_str()),
            ("it was, l", "it was, l", "it was, cold")
        );

        // off never does: it is the identity function
        let off = Fillers::new(FillersConfig::default());
        for input in ["so um yeah", "it was, like, cold", "the the thing"] {
            assert_eq!(prefix_violation(&off, input), None);
        }
    }

    // ---- config ------------------------------------------------------------

    #[test]
    fn level_round_trips_through_toml_and_json() {
        for level in FillerLevel::SELECTABLE {
            let cfg = FillersConfig {
                enabled: true,
                level,
            };
            let text = toml::to_string_pretty(&cfg).unwrap();
            assert!(text.contains(level.as_str()), "{text}");
            let back: FillersConfig = toml::from_str(&text).unwrap();
            assert_eq!(back.level, level);

            let json = serde_json::to_string(&cfg).unwrap();
            let back: FillersConfig = serde_json::from_str(&json).unwrap();
            assert_eq!(back.level, level);
        }
    }

    /// `medium` is held back until #74 answers whether the models emit the
    /// commas its safety argument depends on. No config can select it, and the
    /// serialized form still round-trips to `off` rather than failing, so a
    /// config written by a build that ships it later still loads here.
    #[test]
    fn medium_cannot_be_selected_from_config() {
        assert_eq!(FillerLevel::parse("medium"), None);
        assert_eq!(FillerLevel::parse("Medium"), None);
        assert!(!FillerLevel::SELECTABLE.contains(&FillerLevel::Medium));
        for raw in [
            "enabled = true\nlevel = \"medium\"\n",
            "enabled = true\nlevel = \"MEDIUM\"\n",
        ] {
            let cfg: FillersConfig = toml::from_str(raw).unwrap();
            assert_eq!(cfg.level, FillerLevel::Off, "on {raw:?}");
            assert_eq!(
                Fillers::new(cfg).apply("It was, like, cold."),
                "It was, like, cold."
            );
        }
        let json: FillersConfig =
            serde_json::from_str(r#"{"enabled":true,"level":"medium"}"#).unwrap();
        assert_eq!(json.level, FillerLevel::Off);
    }

    /// Why it is held back, in code. Every one of these is a load-bearing word
    /// that `medium` deletes, and every one of them needs the commas to be
    /// there — so the level is live exactly where its safety is unproven.
    /// **When someone fixes these, this test fails, and that is the signal to
    /// ungate the level** (see #74).
    #[test]
    fn medium_still_has_the_false_positives_that_gate_it() {
        let known_bad = [
            ("You know, don't you?", "Don't you?"),
            ("Well, sort of, yes.", "Well, yes."),
            ("Did he, actually, do it?", "Did he, do it?"),
            ("Like, the video please.", "The video please."),
            (
                "Let's meet Tuesday, I mean, Wednesday.",
                "Let's meet Tuesday, Wednesday.",
            ),
        ];
        for (input, still_wrong) in known_bad {
            assert_eq!(medium(input), still_wrong, "on {input:?}");
            // and the level that ships is untouched by every one of them
            assert_eq!(&light(input), input, "light must not touch {input:?}");
        }
    }

    #[test]
    fn an_unknown_level_falls_back_to_off_instead_of_failing_the_config() {
        for raw in [
            "enabled = true\nlevel = \"meduim\"\n",
            "enabled = true\nlevel = \"aggressive\"\n",
            "enabled = true\nlevel = \"\"\n",
        ] {
            let cfg: FillersConfig = toml::from_str(raw).unwrap();
            assert_eq!(cfg.level, FillerLevel::Off, "on {raw:?}");
        }
    }

    #[test]
    fn level_parsing_is_case_and_space_insensitive() {
        for raw in ["light", "Light", "LIGHT", " light "] {
            assert_eq!(
                FillerLevel::parse(raw),
                Some(FillerLevel::Light),
                "on {raw:?}"
            );
        }
        assert_eq!(FillerLevel::parse("nope"), None);
        for raw in ["off", "Off", "OFF", " off "] {
            assert_eq!(
                FillerLevel::parse(raw),
                Some(FillerLevel::Off),
                "on {raw:?}"
            );
        }
        let cfg: FillersConfig = toml::from_str("level = \"LIGHT\"\n").unwrap();
        assert_eq!(cfg.level, FillerLevel::Light);
    }

    /// The lenient landing is for *any* unusable value, not just a misspelled
    /// string: `level = 3` reached `main` as a fatal config error before, which
    /// is a daemon that will not start over one character in a file the user
    /// hand-edited.
    #[test]
    fn a_level_of_the_wrong_type_lands_on_off_too() {
        for raw in [
            "level = 3\n",
            "level = true\n",
            "level = 1.5\n",
            "level = [\"light\"]\n",
            "level = { name = \"light\" }\n",
        ] {
            let cfg: FillersConfig =
                toml::from_str(raw).unwrap_or_else(|e| panic!("{raw:?} failed to parse: {e}"));
            assert_eq!(cfg.level, FillerLevel::Off, "on {raw:?}");
        }
        let json: FillersConfig = serde_json::from_str(r#"{"level":null}"#).unwrap();
        assert_eq!(json.level, FillerLevel::Off);
    }

    #[test]
    fn a_config_without_a_level_still_loads() {
        let cfg: FillersConfig = toml::from_str("enabled = true\n").unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.level, FillerLevel::Off);
    }

    #[test]
    fn nothing_to_validate() {
        assert!(Fillers::new(FillersConfig::default()).validate().is_empty());
        assert!(Fillers::new(FillersConfig {
            enabled: true,
            level: FillerLevel::Medium
        })
        .validate()
        .is_empty());
    }

    // ---- the word lists themselves ----------------------------------------

    #[test]
    fn word_lists_are_well_formed() {
        let lists: [(&str, &[&str]); 5] = [
            ("HESITATIONS", HESITATIONS),
            ("PAIR_FOLLOWERS", PAIR_FOLLOWERS),
            ("STUTTER_WORDS", STUTTER_WORDS),
            ("CLAUSE_STARTERS", CLAUSE_STARTERS),
            ("CONJUNCTIONS", CONJUNCTIONS),
        ];
        for (name, list) in lists {
            let mut sorted = list.to_vec();
            sorted.sort_unstable();
            assert_eq!(sorted, list, "{name} is not sorted");
            sorted.dedup();
            assert_eq!(sorted.len(), list.len(), "{name} has a duplicate");
            for w in list {
                assert!(
                    w.bytes().all(|b| b.is_ascii_lowercase() || b == b'\'') && !w.is_empty(),
                    "{name} entry {w:?} must be lowercase ASCII"
                );
                assert!(
                    w.len() <= MAX_LIST_WORD,
                    "{name} entry {w:?} exceeds MAX_LIST_WORD"
                );
            }
        }
        for h in HEDGES {
            for w in h.words {
                assert!(
                    w.len() <= MAX_LIST_WORD,
                    "hedge {w:?} exceeds MAX_LIST_WORD"
                );
            }
        }
        // the exclusions the comments promise
        for w in [
            "had", "that", "is", "was", "can", "do", "in", "on", "no", "very", "he", "will",
        ] {
            assert!(
                !STUTTER_WORDS.contains(&w),
                "{w:?} must stay out of STUTTER_WORDS"
            );
        }
        assert!(!HESITATIONS.contains(&"mm"), "mm is millimetres");
        assert!(!HESITATIONS.contains(&"hmm"));
    }

    // ---- tokenizer ---------------------------------------------------------

    #[test]
    fn words_splits_the_way_the_rules_say() {
        let cases: [(&str, &[&str]); 8] = [
            ("hello world", &["hello", "world"]),
            ("don't stop", &["don't", "stop"]),
            ("like-minded folk", &["like-minded", "folk"]),
            ("hi,um,there", &["hi", "um", "there"]),
            ("e\u{0301}gal", &["e\u{0301}gal"]),
            ("it\u{2019}s fine", &["it\u{2019}s", "fine"]),
            ("  ", &[]),
            ("2026-08-15 ok", &["2026-08-15", "ok"]),
        ];
        for (input, want) in cases {
            let got: Vec<&str> = words(input).into_iter().map(|s| word(input, s)).collect();
            assert_eq!(got, want, "on {input:?}");
        }
    }

    // ---- cost --------------------------------------------------------------

    /// Not a benchmark, a guard: linear code finishes this instantly, and
    /// anything quadratic hangs the release build the first time a user
    /// dictates for ten minutes. `--nocapture` prints the real numbers.
    #[test]
    fn cost_is_linear_enough_to_ignore() {
        let utterance = "So, um, I think, you know, we should, like, ship the the thing on \
                         Tuesday, I mean Wednesday, because the build is, basically, green.";
        for level in [FillerLevel::Light, FillerLevel::Medium] {
            for _ in 0..50 {
                std::hint::black_box(at(level, utterance)); // warm up
            }
            let start = std::time::Instant::now();
            let runs = 500;
            for _ in 0..runs {
                std::hint::black_box(at(level, utterance));
            }
            let each = start.elapsed() / runs;
            println!("{level} on a {}-char utterance: {each:?}", utterance.len());
        }

        let long = "the quick brown fox jumps over the lazy dog. ".repeat(45_000);
        let start = std::time::Instant::now();
        std::hint::black_box(medium(&long));
        let big = start.elapsed();
        println!("medium on {} bytes: {big:?}", long.len());
        assert!(big.as_secs() < 5, "2 MB took {big:?} — that is not linear");
    }
}
