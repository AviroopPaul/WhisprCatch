//! Marked self-correction — "meet Tuesday, I mean Wednesday" keeps only the
//! correction. Marked phrases only; guessing at unmarked ones is out of scope
//! for the whole milestone (#36).
//!
//! # The shape of the problem
//!
//! A spoken repair has three parts: the **reparandum** (what the speaker is
//! taking back), the **marker** that announces the repair, and the **repair**
//! itself. This transform deletes the first two and keeps the third:
//!
//! ```text
//!   I said Tuesday, I mean Wednesday
//!          ~~~~~~~~ ^^^^^^ ---------
//!          retract  marker  keep
//! ```
//!
//! Everything hard about this is deciding *how far back* to retract, and
//! deciding whether a marker is a marker at all. Both answers are biased the
//! same way throughout: **when in doubt, change nothing.** Leaving a hedge in
//! costs the user a word that [`crate::Fillers`] will probably remove one
//! transform later; deleting a word they meant to keep costs them a sentence
//! they have to notice and retype. Those are not the same mistake.
//!
//! # How far back
//!
//! Every candidate window is bounded by three hard barriers that nothing
//! crosses:
//!
//! * the previous **phrase** boundary (`,` `;` `:` `—`),
//! * the previous **sentence** boundary (`.` `!` `?`) or line break, which is
//!   never crossed backwards under any circumstances,
//! * an opening quote or bracket, so a repair inside a quotation cannot eat
//!   the attribution that introduced it.
//!
//! Inside those, the window is chosen by two rules, in order:
//!
//! 1. **Alignment.** Speakers repair by restarting a constituent, so the
//!    repair usually repeats its own first word: "book the 9am, no wait, the
//!    10am slot". Retracting back to that repeat is right where counting words
//!    would lose the verb. Numbers align by *kind* — "40, correction, 45"
//!    replaces one with the other precisely because they differ — but an exact
//!    repeat still wins, or "we lost 3 to 1, I mean 3 to 2" becomes "we lost 3
//!    to 3 to 2".
//! 2. **Length.** With nothing to align on, the window is the repair's own
//!    word count, additionally stopped in front of the verb holding the clause
//!    together (`CLAUSE_SPINE`). That is what keeps "meet at the coffee shop
//!    on Tuesday, I mean Wednesday" from collapsing to "Wednesday", and "our
//!    target is Q4, I mean end of year" from losing its subject.
//!
//! The phrase boundary is a *ceiling*, not the target. Reaching for it
//! directly is the naive implementation that eats correct text.
//!
//! # Is it really a marker?
//!
//! Four gates, in order of how much work they do:
//!
//! 1. **Set-off punctuation.** A repair interrupts the utterance, and both
//!    shipped models render that break as punctuation. So a marker only counts
//!    when the word before it or its own last word carries `,` `;` `:` `—` or
//!    a full stop. This one gate is what separates `"Tuesday, no wait,
//!    Wednesday"` from `"we found no wait staff"`, and `"…, I mean Wednesday"`
//!    from `"honestly I mean it"`. The cost is that a completely unpunctuated
//!    transcript gets no self-correction at all, which is the right way round.
//! 2. **The repair head** (`i mean` family only). A repair replaces a content
//!    constituent, so it never *begins* with a pronoun, an auxiliary, a
//!    complementiser or a discourse particle. When it does — "I mean, that's
//!    just my opinion", "I mean it sincerely" — "I mean" is hedging, not
//!    repairing. See `HEDGE_HEADS`, and `CLAUSE_BEFORE` for the mirror
//!    case, "do you know what I mean".
//! 3. **The retracted span must contain content.** If everything the window
//!    would delete is a function word, the match is a coincidence, not a
//!    repair: "you know what I mean, right?" would otherwise retract "what".
//! 4. **A clause needs parallelism.** An "I mean" introducing a whole clause
//!    is a speaker elaborating unless the clause visibly restarts an earlier
//!    one, first *two* words and all. "the plan is Tuesday, I mean the plan is
//!    Wednesday" resolves; "we sold the house on the corner, I mean the price
//!    was good" does not.
//!
//! # The seam with filler removal (#44)
//!
//! "I mean" is this transform's marker and filler removal's hedge, which is why
//! `Polish::from_config` runs `self_correct` first. The split is by what
//! precedes the marker, not by its punctuation: this transform claims every
//! "I mean" with something to retract in front of it, and leaves every one
//! without — the sentence-initial hedge — for filler removal.
//!
//! That includes the **both-commas** shape, "Let's meet Tuesday, I mean,
//! Wednesday.": it resolves here, so filler removal never sees it. The claim
//! rests on chain order alone. As merged, #44 ships `off` and `light` only,
//! and `i mean` lives in a hedge list consulted solely at `medium` — so at the
//! shipping level nothing else in the chain touches "I mean" in any form, and
//! this transform is its only claimant. `the_i_mean_shapes_split_cleanly_with_fillers`
//! pins the table.
//!
//! # Deliberately not handled
//!
//! * **Unmarked corrections** ("meet Tuesday. Wednesday.") need a model to
//!   detect. They belong to #37, and this transform must pass them through
//!   untouched rather than guess — `unmarked_corrections_pass_through` is the
//!   test that says so.
//! * **Repairs across a sentence boundary** ("Let's meet Tuesday. Sorry, I
//!   meant Wednesday.") resolve to nothing, because the acceptance criterion in
//!   #48 is that the window never crosses one backwards.
//! * **A user-editable marker list.** The whole safety argument above rests on
//!   these seven phrases and their gates; a config key that let someone add
//!   "the" would hand them a text shredder. [`Transform::validate`] therefore
//!   stays the default empty implementation, because this transform accepts no
//!   user-authored input at all.
//!
//! # What is assumed rather than measured (#74)
//!
//! Nobody has yet sampled what Parakeet and Moonshine actually emit for a
//! spoken self-correction, and two rules here key on exactly that:
//!
//! * gate 1 assumes the prosodic break around a repair is rendered as `,` (or
//!   `;` `:` `—` or a full stop) on one side of the marker. If the models emit
//!   no punctuation there, nothing fires at all;
//! * the sentence-boundary barrier assumes the models do *not* end a sentence
//!   between the retracted phrase and the marker. If they emit "Meet Tuesday.
//!   I mean Wednesday.", the repair resolves to nothing, by design.
//!
//! Both are failures in the safe direction — no rewrite rather than a wrong
//! one — but they are what #74 should be pointed at first.

use serde::{Deserialize, Serialize};

use crate::Transform;

/// Implemented in issue #48.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SelfCorrectConfig {
    /// Off by default. Whether self-correction should ship on is a product
    /// decision that has not been made; the seam's promise is that a default
    /// config is byte-identical to the raw transcript.
    pub enabled: bool,
}

/// Drops the retracted half of a marked correction.
///
/// Runs *before* [`crate::Fillers`] — "I mean" is both this transform's marker
/// and a hedge that filler removal strips. See `Polish::from_config`.
pub struct SelfCorrect {
    cfg: SelfCorrectConfig,
}

impl SelfCorrect {
    pub fn new(cfg: SelfCorrectConfig) -> Self {
        Self { cfg }
    }
}

impl Transform for SelfCorrect {
    fn name(&self) -> &'static str {
        "self_correct"
    }

    fn apply(&self, text: &str) -> String {
        if !self.cfg.enabled {
            return text.to_string();
        }
        rewrite(text)
    }

    /// Not prefix-stable, and the most obviously so: a streaming pass that has
    /// heard `"meet Tuesday"` has already typed it, and the finished
    /// `"meet Tuesday, I mean Wednesday"` polishes to `"meet Wednesday"`. The
    /// retracted words are on the user's screen before the marker that
    /// retracts them is ever spoken.
    ///
    /// `prefix_violation` finds it one character into the divergence:
    /// `apply("I said T") == "I said T"`, which is not a prefix of
    /// `apply("I said Tuesday, I mean Wednesday") == "I said Wednesday"`.
    /// This can never become `true` — a repair is only knowable after the
    /// marker, which always arrives later than the words it retracts.
    fn prefix_stable(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// markers
// ---------------------------------------------------------------------------

/// Which marker matched. Only the family, not the exact wording, changes what
/// happens afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    /// "I mean", "I meant", "sorry, I mean(t)". Ambiguous with hedging, so
    /// this is the only family that gets the [`HEDGE_HEADS`] gate.
    IMean,
    /// "no wait", "scratch that". Unambiguous as repair markers; the set-off
    /// gate carries the weight of not matching them inside ordinary prose.
    Retract,
    /// A bare "correction," — needs the strictest gate of all, because the
    /// word is an ordinary noun.
    Correction,
}

struct Pattern {
    words: &'static [&'static str],
    family: Family,
}

/// The marker list, longest first so "sorry, I meant" wins over "I meant".
///
/// Exactly the five phrases named in #48 plus tense and politeness variants.
/// Every entry earns a false-positive test in this module's corpus; nothing
/// gets added here without one. "or rather", "my mistake" and a bare "sorry,"
/// were all considered and cut: they are rare in dictation and each one is a
/// live grenade in ordinary prose ("tell him sorry, Wednesday works").
const MARKERS: &[Pattern] = &[
    Pattern {
        words: &["sorry", "i", "meant"],
        family: Family::IMean,
    },
    Pattern {
        words: &["sorry", "i", "mean"],
        family: Family::IMean,
    },
    Pattern {
        words: &["i", "meant"],
        family: Family::IMean,
    },
    Pattern {
        words: &["i", "mean"],
        family: Family::IMean,
    },
    Pattern {
        words: &["no", "wait"],
        family: Family::Retract,
    },
    Pattern {
        words: &["scratch", "that"],
        family: Family::Retract,
    },
    Pattern {
        words: &["correction"],
        family: Family::Correction,
    },
];

/// Substrings without which no marker can possibly match. One case-insensitive
/// scan of these turns the common case — a transcript with no repair in it —
/// into a single linear pass and no allocation beyond the returned copy.
const ANCHORS: &[&str] = &["mean", "wait", "scratch", "correction"];

/// Words that cannot begin a repair, so an "I mean" in front of one is a hedge.
///
/// The principle, not the list, is the thing to review: a repair replaces a
/// *content* constituent — a noun phrase, a date, a number, a prepositional
/// phrase. It never opens with a pronoun, an auxiliary, a complementiser or a
/// discourse particle. "I mean, that's just my opinion" and "I mean it
/// sincerely" both die here.
///
/// This is the gate #44 should be read against: everything this list blocks is
/// an "I mean" that filler removal is welcome to strip as a hedge, and
/// everything it lets through is an "I mean" that has already been deleted by
/// the time #44 runs.
///
/// Seven categories, in order below: pronouns and their contractions;
/// determiner-pronouns and existentials; complementisers and wh-words;
/// auxiliaries and copulas; the objects that make "mean" a plain verb ("I mean
/// to…", "I mean by that…", "I mean business"); conjunctions, because a repair
/// replaces a constituent rather than starting a new clause ("know what I
/// mean, **or** should I explain again?"); and discourse particles, adverbs and
/// adjuncts, because "the total is 40 dollars, I mean **including** tax" is a
/// speaker adding to what they said rather than replacing it.
///
/// Possessive determiners (my, your, his, her, our, their) are deliberately
/// *absent*: they head a noun phrase, so "call Bob, I mean my brother" is a
/// repair. Blocking the adjuncts costs the occasional real repair ("at noon, I
/// mean after lunch") and saves every clarification, which is the trade #48
/// asks for.
const HEDGE_HEADS: &str = "\
    i i'm i've i'd i'll you you're you've you'd you'll we we're we've we'd we'll he he's he'd \
    she she's she'd they they're they've they'd it it's its me him them us let's \
    that that's this these those there there's \
    what what's when where why how who whom whose which if whether because \
    is am are was were be been being do does did don't doesn't didn't have has had \
    will would won't can can't could should shall may might must \
    to by business \
    and or but nor so then though although unless while \
    well yeah yes no not nope okay ok sure right really just like maybe actually honestly \
    obviously basically seriously literally kind sort \
    nothing anything something everything anyone everyone everybody somebody \
    including excluding plus minus with without after before since assuming counting \
    roughly approximately about around only also too even especially specifically mainly mostly \
    probably possibly usually always never still either neither both instead rather \
    technically essentially effectively apparently presumably arguably personally frankly \
    clearly certainly definitely hopefully unfortunately anyway anyhow regardless otherwise";

/// A word before "I mean" that makes it a relative clause rather than an
/// aside: "do you know what I mean", "that is what I meant". Blocks the whole
/// construction regardless of what follows it.
const CLAUSE_BEFORE: &str = "what whatever that which whichever who whom";

/// The verb holding a clause together. A window guessed from the repair's
/// length stops in front of one of these rather than reaching through it —
/// "our target is Q4, I mean end of year" only got "Q4" wrong.
const CLAUSE_SPINE: &str = "\
    is am are was were be been being has have had do does did \
    will would can could should shall may might must \
    i'm i've i'd i'll you're you've you'll we're we've we'll they're they've \
    it's that's there's he's she's isn't aren't wasn't don't doesn't didn't can't won't";

/// Hesitation noise that may sit between the retracted phrase and the marker.
/// Absorbed into the deletion for free — it is part of the repair, not part of
/// what the user wanted — but never counted against the window budget, so
/// "Tuesday, well, no wait, Wednesday" still retracts "Tuesday".
const ABSORBED: &str = "um uh er erm ah oh hmm hm no well so like okay ok yeah sorry actually";

/// A retracted span made of nothing but these is a coincidence, not a repair.
/// "you know what I mean, right?" retracts "what" without this gate.
///
/// Determiners, pronouns and their contractions, auxiliaries and copulas,
/// prepositions and conjunctions, and discourse particles — everything that
/// carries grammar rather than meaning.
const FUNCTION_WORDS: &str = "\
    a an the this that these those my your his her its our their some any \
    i you he she it we they me him them us who whom whose what which there here \
    i'm i've i'll i'd you're you've you'll we're we've we'll they're they've it's that's \
    there's he's she's let's don't doesn't didn't can't won't isn't aren't wasn't \
    is am are was were be been being do does did have has had \
    will would can could should shall may might must \
    to of in on at for with by from and but or nor so if as than then \
    about into over up out because \
    um uh er erm ah oh hmm hm well like okay ok yeah yes no not right just actually sorry \
    please very really";

/// Determiners, for the stranded-modifier check. Deliberately *only*
/// determiners: a pronoun in front of a content word is an ordinary
/// subject-verb ("I said Tuesday"), not a modified noun phrase.
const DETERMINERS: &str = "a an the this that these those my your his her its our their \
    some any every each no another one";

/// Numbers spelled out, so numeric alignment works on Parakeet's output as
/// well as Moonshine's. See [`is_numeric`].
const NUMBER_WORDS: &str = "\
    zero one two three four five six seven eight nine ten eleven twelve thirteen fourteen \
    fifteen sixteen seventeen eighteen nineteen twenty thirty forty fifty sixty seventy \
    eighty ninety hundred thousand million billion \
    first second third fourth fifth sixth seventh eighth ninth tenth \
    eleventh twelfth thirteenth fifteenth twentieth thirtieth";

/// Determiners, prepositions and conjunctions — the words a fronted adverbial
/// is made of. A retained fragment of nothing but these has no subject and no
/// verb left in it, so the repair would be dangling off "By the …".
///
/// Narrower than [`FUNCTION_WORDS`] on purpose: pronouns and copulas are a
/// perfectly good thing to leave behind ("it was ~~Tuesday~~, I mean
/// Wednesday"), so they are absent here.
const FRAGMENT_WORDS: &str = "\
    a an the this that these those my your his her its our their some any every each another \
    to of in on at for with by from into onto over under about across after before during \
    per via since until through toward towards within without between among \
    and but or nor so if as than then because";

/// Words whose trailing full stop is an abbreviation, not the end of a
/// sentence. See [`is_abbreviation`] — a false positive here costs a repair and
/// can never damage text, so the list errs long.
const ABBREVIATIONS: &str = "\
    mr mrs ms mx dr prof sr jr st mt ft rd ave blvd dept univ inc ltd co corp \
    jan feb mar apr jun jul aug sep sept oct nov dec \
    mon tue tues wed weds thu thur thurs fri sat sun \
    fig figs no nos vs etc eg ie al approx est min max hr hrs sec secs \
    vol ch pp ed eds cf ibid pt qty ref rev";

/// Hard ceiling on both halves of a rewrite, in words. The phrase boundary
/// almost always bites first; this is the backstop that keeps "bounded rewrite
/// window" true even for a comma-free run-on.
const MAX_WINDOW: usize = 12;

// ---------------------------------------------------------------------------
// tokens
// ---------------------------------------------------------------------------

/// One whitespace-delimited word, as a byte span into the original text.
///
/// Spans, not owned strings: every untouched byte of the input is copied
/// straight out of the original, so combining marks, ZWJ sequences, CJK and
/// the exact whitespace the user dictated survive verbatim. This transform
/// only ever *deletes* ranges — it never re-joins words.
#[derive(Debug, Clone, Copy)]
struct Tok {
    start: usize,
    end: usize,
    /// Whether the whitespace immediately before this token contains a line
    /// break. Treated as a sentence boundary in both directions.
    breaks_line: bool,
}

/// A matched marker, as a half-open token range.
#[derive(Debug, Clone, Copy)]
struct Mark {
    start: usize,
    end: usize,
    family: Family,
}

/// What punctuation ends a token, ignoring any closing quotes or brackets
/// after it (`mind."` ends a sentence; `wait,"` ends a phrase).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Break {
    Sentence,
    Phrase,
    None,
}

fn is_line_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn is_closing(c: char) -> bool {
    matches!(
        c,
        '"' | '\'' | '\u{201d}' | '\u{2019}' | ')' | ']' | '}' | '\u{bb}' | '\u{203a}'
    )
}

fn is_opening(c: char) -> bool {
    matches!(
        c,
        '"' | '\u{201c}' | '\u{201e}' | '\u{2018}' | '\u{ab}' | '\u{2039}' | '(' | '[' | '{'
    )
}

fn is_sentence_punct(c: char) -> bool {
    matches!(
        c,
        '.' | '!' | '?' | '\u{2026}' | '\u{3002}' | '\u{ff01}' | '\u{ff1f}'
    )
}

fn is_phrase_punct(c: char) -> bool {
    matches!(
        c,
        ',' | ';' | ':' | '\u{2014}' | '\u{2013}' | '-' | '\u{3001}' | '\u{ff0c}'
    )
}

fn tokenize(text: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut start: Option<usize> = None;
    let mut broke_line = false;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                toks.push(Tok {
                    start: s,
                    end: i,
                    breaks_line: broke_line,
                });
                broke_line = false;
            }
            if is_line_break(c) {
                broke_line = true;
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        toks.push(Tok {
            start: s,
            end: text.len(),
            breaks_line: broke_line,
        });
    }
    toks
}

fn text_of(text: &str, t: Tok) -> &str {
    &text[t.start..t.end]
}

/// The word inside a token, with leading and trailing punctuation stripped.
/// Interior punctuation stays, so "don't" and "3.5" survive intact.
fn core(s: &str) -> &str {
    s.trim_start_matches(|c: char| !c.is_alphanumeric())
        .trim_end_matches(|c: char| !c.is_alphanumeric())
}

fn trailing_punct(s: &str) -> Option<char> {
    for c in s.chars().rev() {
        if is_closing(c) {
            continue;
        }
        if is_sentence_punct(c) || is_phrase_punct(c) {
            return Some(c);
        }
        return None;
    }
    None
}

fn break_after(s: &str) -> Break {
    match trailing_punct(s) {
        Some(c) if is_sentence_punct(c) => Break::Sentence,
        Some(_) => Break::Phrase,
        None => Break::None,
    }
}

/// Membership in one of the whitespace-separated word lists above.
///
/// They are strings rather than arrays so that a list stays readable as the
/// data it is — grouped by category on its own line — instead of being
/// exploded one word per line the moment somebody adds a word longer than
/// eight characters. `word_lists_are_well_formed` guards the one typo class
/// this trades for.
fn in_list(list: &str, word: &str) -> bool {
    !word.is_empty()
        && list
            .split_ascii_whitespace()
            .any(|w| w.eq_ignore_ascii_case(word))
}

/// Case-insensitive substring search over ASCII needles, without allocating a
/// lowercased copy of a possibly multi-megabyte transcript.
fn contains_ci(haystack: &str, needle: &str) -> bool {
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    if n.is_empty() || n.len() > h.len() {
        return false;
    }
    let first = n[0];
    (0..=h.len() - n.len())
        .any(|i| h[i].eq_ignore_ascii_case(&first) && h[i..i + n.len()].eq_ignore_ascii_case(n))
}

fn has_anchor(text: &str) -> bool {
    ANCHORS.iter().any(|a| contains_ci(text, a))
}

// ---------------------------------------------------------------------------
// matching
// ---------------------------------------------------------------------------

fn match_at(text: &str, toks: &[Tok], i: usize) -> Option<Mark> {
    for p in MARKERS {
        if i + p.words.len() > toks.len() {
            continue;
        }
        // A marker inside a quotation is quoted material, not a repair:
        // `He said "no wait" and left.` must survive intact.
        if text_of(text, toks[i]).starts_with(is_opening) {
            return None;
        }
        let mut ok = true;
        for (k, w) in p.words.iter().enumerate() {
            let t = toks[i + k];
            if !core(text_of(text, t)).eq_ignore_ascii_case(w) {
                ok = false;
                break;
            }
            // The marker is one phrase: a full stop or a line break inside it
            // means these words are not a unit ("no. Wait, who is this?").
            let last = k + 1 == p.words.len();
            if !last
                && (break_after(text_of(text, t)) == Break::Sentence || toks[i + k + 1].breaks_line)
            {
                ok = false;
                break;
            }
        }
        if !ok {
            continue;
        }
        // "correction" is an ordinary noun. Only accept it spoken as an aside,
        // i.e. carrying its own comma or colon: "ship it Tuesday, correction,
        // Wednesday". "the correction was applied" must never match.
        if p.family == Family::Correction
            && !matches!(
                trailing_punct(text_of(text, toks[i])),
                Some(',') | Some(':')
            )
        {
            continue;
        }
        return Some(Mark {
            start: i,
            end: i + p.words.len(),
            family: p.family,
        });
    }
    None
}

fn find_markers(text: &str, toks: &[Tok]) -> Vec<Mark> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        match match_at(text, toks, i) {
            Some(m) => {
                i = m.end;
                out.push(m);
            }
            None => i += 1,
        }
    }
    out
}

/// Gate 1: the marker must be set off by punctuation.
///
/// Both shipped models punctuate natively, and a self-repair is a prosodic
/// interruption they render as a comma. Requiring it is what makes "we found
/// no wait staff" and "honestly I mean it" safe.
///
/// **`no wait`, `scratch that` and `correction` need the break on both sides.**
/// A comma on the preceding token alone is worthless for them, because a
/// fronted adverbial plus comma is one of the commonest openings in dictated
/// English and it sits in exactly the position a repair marker's own comma
/// sits: "Order ahead, no wait times." and "If you are itchy, scratch that
/// rash." are not repairs, and a leading-comma-only rule ate 467 of 475 of
/// that shape.
///
/// `I mean` keeps the looser test, because the issue's headline example —
/// "I said Tuesday, I mean Wednesday" — has no comma after "mean". What pays
/// for that looseness is [`single_comma_restraint`], not this gate.
fn set_off(text: &str, toks: &[Tok], m: &Mark) -> Option<SetOff> {
    let own = break_after(text_of(text, toks[m.end - 1])) != Break::None;
    let before = m.start > 0 && break_after(text_of(text, toks[m.start - 1])) != Break::None;
    let ok = match m.family {
        Family::Correction | Family::Retract => own && before,
        Family::IMean => own || before,
    };
    ok.then_some(SetOff {
        both_sides: own && before,
    })
}

/// How well set off the marker was, which is how much latitude the window gets.
#[derive(Debug, Clone, Copy)]
struct SetOff {
    both_sides: bool,
}

/// How many *words* the repair runs to: up to the next phrase or sentence
/// boundary, the next marker, a line break, or [`MAX_WINDOW`].
///
/// Tokens with no letters or digits in them — a lone emoji, a dash — are
/// walked over but not counted. They are not words the user is replacing, and
/// counting them makes the window reach one phrase too far back.
fn repair_len<'a>(text: &'a str, toks: &[Tok], cover: &[bool], m: &Mark) -> Repair<'a> {
    let mut r = Repair::default();
    let mut i = m.end;
    while i < toks.len() && r.len < MAX_WINDOW && i - m.end < MAX_WINDOW * 2 {
        if toks[i].breaks_line || cover[i] {
            break;
        }
        let word = core(text_of(text, toks[i]));
        if !word.is_empty() {
            r.len += 1;
            r.clause |= in_list(CLAUSE_SPINE, word);
            match r.len {
                1 => {
                    r.head = word;
                    r.numeric_head = is_numeric(word);
                }
                2 => r.second = word,
                _ => {}
            }
        }
        let brk = break_after(text_of(text, toks[i]));
        r.closed_with_comma = brk == Break::Phrase;
        if brk == Break::Sentence && is_abbreviation(text_of(text, toks[i])) {
            r.abbreviated = true;
        }
        i += 1;
        if brk != Break::None {
            break;
        }
    }
    r
}

/// Whether a token ending in `.` is an abbreviation rather than a sentence.
///
/// [`break_after`] deliberately stays strict — treating every trailing full
/// stop as a sentence end is what guarantees the window never crosses a real
/// one backwards, which is #48's acceptance criterion, and loosening it would
/// let a window walk through "Look at the sun." into the sentence before.
///
/// So abbreviations are handled at the other end: the boundary still stops
/// everything, and a rewrite that *relied* on it stands down instead. That
/// turns "Mr. Patel, I mean Dr. Patel, will call you." from the half-repair
/// "Mr. Dr. Patel, will call you." into no rewrite at all. A false positive
/// here therefore only ever costs a repair; it can never damage text.
fn is_abbreviation(token: &str) -> bool {
    let word = core(token);
    // "U.S.", "e.g." — an interior full stop is never a sentence end
    if word.contains('.') {
        return true;
    }
    in_list(ABBREVIATIONS, word)
}

/// What follows the marker.
#[derive(Debug, Clone, Copy, Default)]
struct Repair<'a> {
    /// Length in words, which is what the window is measured against.
    len: usize,
    /// Whether it is a whole clause rather than a constituent. An "I mean"
    /// followed by a full clause is a speaker elaborating unless the clause
    /// visibly restarts an earlier one: "we sold the house on the corner, I
    /// mean the price was good" is left alone, "the plan is Tuesday, I mean
    /// the plan is Wednesday" is not.
    clause: bool,
    /// First word, the one the retract window aligns on.
    head: &'a str,
    /// Whether [`Repair::head`] is a number, in which case any number aligns
    /// with it — "40, correction, 45" replaces one with the other precisely
    /// because they differ.
    numeric_head: bool,
    /// Second word, used to check that an alignment is real parallelism and
    /// not a coincidental "the".
    second: &'a str,
    /// Whether the span was cut short by a full stop that is really an
    /// abbreviation — "I mean **Dr.** Patel". The repair is then half a phrase
    /// and nothing may be built on it.
    abbreviated: bool,
    /// Whether the span ends on a comma, which is half the appositive
    /// signature — see [`is_an_appositive`].
    closed_with_comma: bool,
}

/// What the retracted span is expected to start with, if the repair gives us
/// a usable hint.
///
/// Speakers repair by restarting a constituent, so the repair and the phrase
/// it replaces very often share their first word: "book the 9am, no wait, the
/// 10am slot". Aligning on that shared word beats counting words, which would
/// take "Book the 9am" down to "the 10am slot" and lose the verb. Numbers
/// align by kind rather than by value, because the whole point of "40,
/// correction, 45" is that the two numbers differ.
/// Whether a word is a number, spelled either way.
///
/// The spelled-out half is not decoration: SCOPE.md records that Parakeet
/// "writes numbers as words", so a numeric repair coming out of the shipping
/// Linux default reads "three chairs, I mean four chairs", never "3, I mean 4".
/// A digits-only test would leave numeric alignment dead on that model.
fn is_numeric(word: &str) -> bool {
    word.chars().any(|c| c.is_numeric()) || in_list(NUMBER_WORDS, word)
}

/// The word after token `i`, looking only at real words and stopping at
/// `limit`. Used to check that an alignment is parallel rather than lucky.
fn next_word<'a>(text: &'a str, toks: &[Tok], i: usize, limit: usize) -> &'a str {
    toks[i + 1..limit]
        .iter()
        .map(|t| core(text_of(text, *t)))
        .find(|w| !w.is_empty())
        .unwrap_or("")
}

/// Why a marker set off on only one side may retract only one word, or one it
/// can point at.
///
/// `I mean` cannot be made to require a comma on both sides: the issue's
/// headline example, "I said Tuesday, I mean Wednesday", has no comma after
/// "mean", and neither does most of the corpus. But the same looseness lets an
/// unlisted intensifier eat the subject of the sentence, because gate 2's
/// [`HEDGE_HEADS`] is a curated list and always will be:
///
/// ```text
///   "I love it, I mean absolutely love it"    -> "Absolutely love it"
///   "He runs daily, I mean every single day." -> "Every single day."
///   "It costs money, I mean serious money."   -> "It serious money."
/// ```
///
/// Those are not papercuts — CLAUDE.md is explicit that deleting a word the
/// user meant to keep is a trust problem. So the latitude, not the marker, is
/// what gets cut. A marker without both commas fires only when
///
/// * the repair is a **single word** — the minimal swap, and the shape the
///   headline example and most real corrections have; or
/// * the repair's first word **reappears** before the marker, which is
///   positive evidence of where the retracted phrase starts rather than a
///   guess from its length.
///
/// Every case above needs a multi-word window it cannot point at, so every one
/// of them now leaves the text alone. What it costs is recorded in
/// `the_single_comma_restraint_costs_these` — three real repairs, all of the
/// form "X noun, I mean Y noun" with no shared word.
///
/// The alternative considered and rejected was requiring both commas for every
/// family. It is safer still, and it kills the headline example outright.
/// `Retract` and `Correction` do require both, because for them it costs
/// nothing (see [`set_off`]).
///
/// The first token of the retracted span, or `None` when there is nothing
/// safe to retract and the text must be left exactly as dictated.
fn retract_window(
    text: &str,
    toks: &[Tok],
    cover: &[bool],
    m: &Mark,
    repair: Repair,
    set_off: SetOff,
    guard: usize,
) -> Option<usize> {
    let n = repair.len;
    // An "I mean" introducing a whole clause only counts as a repair when the
    // clause visibly restarts one that came before it.
    let require_exact = repair.clause && m.family == Family::IMean;
    // …and a marker set off on only one side gets no latitude at all: see
    // `single_comma_restraint`.
    let restrained = !set_off.both_sides && repair.len > 1;
    // Hesitation noise and stacked markers between the phrase and this marker
    // come off for free: "Tuesday, no wait, Wednesday, I mean Thursday".
    let mut absorbed_start = m.start;
    while absorbed_start > guard {
        let prev = toks[absorbed_start - 1];
        if toks[absorbed_start].breaks_line
            || break_after(text_of(text, prev)) == Break::Sentence
            || !(cover[absorbed_start - 1] || in_list(ABSORBED, core(text_of(text, prev))))
        {
            break;
        }
        absorbed_start -= 1;
    }

    // Walk back to the phrase boundary once, recording two candidate starts:
    // where the repair's own first word reappears, and where the word count
    // matches. The walk itself is what enforces "bounded, and never across a
    // sentence boundary backwards".
    let mut start = absorbed_start;
    let mut counted = 0;
    let mut by_exact = None;
    let mut by_kind = None;
    let mut by_length = None;
    let mut spine = None;
    let mut stepped = 0;
    while start > guard && counted < MAX_WINDOW && stepped < MAX_WINDOW * 2 {
        stepped += 1;
        let prev = toks[start - 1];
        // A line break or a full stop is the end of the road, always.
        if toks[start].breaks_line || break_after(text_of(text, prev)) == Break::Sentence {
            // …but if that full stop is an abbreviation, the window it hands
            // back is a fragment, not a phrase: "Call Dr. ~~Patel,~~ I mean
            // call the nurse" would leave "Call Dr. call the nurse".
            if !toks[start].breaks_line && is_abbreviation(text_of(text, prev)) {
                return None;
            }
            break;
        }
        // The comma directly before the marker is the repair's own separator,
        // so it does not terminate the walk. Any later one does: that is the
        // previous phrase boundary, and the window stops there.
        if counted > 0 && break_after(text_of(text, prev)) == Break::Phrase {
            break;
        }
        start -= 1;
        let word = core(text_of(text, toks[start]));
        if !word.is_empty() {
            counted += 1;
            if !repair.head.is_empty() {
                if by_exact.is_none() && word.eq_ignore_ascii_case(repair.head) {
                    by_exact = Some(start);
                }
                if repair.numeric_head && by_kind.is_none() && is_numeric(word) {
                    by_kind = Some(start);
                }
            }
            if by_length.is_none() && counted == n {
                by_length = Some(start);
            }
            if spine.is_none() && in_list(CLAUSE_SPINE, word) {
                spine = Some(start);
            }
        }
        // Never reach back past an opening quote or bracket into the clause
        // that introduced the quotation.
        if text_of(text, toks[start]).starts_with(is_opening) {
            break;
        }
    }
    if counted == 0 {
        return None;
    }
    // A clause-shaped "I mean" only repairs when the clause is visibly a
    // restart of an earlier one, and a shared first word is not enough to show
    // that: "we sold the house on the corner, I mean the price was good"
    // repeats "the" by accident. Demand the second word too.
    if require_exact {
        let parallel = by_exact.is_some_and(|p| {
            !repair.second.is_empty()
                && next_word(text, toks, p, absorbed_start).eq_ignore_ascii_case(repair.second)
        });
        if !parallel {
            return None;
        }
    }
    // The single-comma restraint. See `single_comma_restraint` for why this is
    // the price of keeping the issue's headline example working. A numeral
    // pointing at a numeral counts: "that's 15 dollars, I mean 50 dollars"
    // knows exactly which words it replaces.
    if restrained && by_exact.is_none() && by_kind.is_none() {
        return None;
    }
    // A window guessed from the repair's *length* must not swallow the verb
    // holding the clause together: "our target is Q4, I mean end of year" is
    // three words long but only "Q4" was wrong. An aligned window may cross
    // one, because the repeated word is evidence that the speaker restarted
    // the whole clause ("the plan is Tuesday, I mean the plan is Wednesday").
    let by_length = by_length.unwrap_or(start).max(match spine {
        Some(s) => s + 1,
        None => 0,
    });
    let start = by_exact.or(by_kind).unwrap_or(by_length);
    if start >= absorbed_start {
        return None;
    }
    // Gate 3: a window of pure function words is a coincidental match.
    let content = toks[start..absorbed_start].iter().any(|t| {
        let w = core(text_of(text, *t));
        !w.is_empty() && !in_list(FUNCTION_WORDS, w)
    });
    if !content {
        return None;
    }
    Some(start)
}

// ---------------------------------------------------------------------------
// rewrite
// ---------------------------------------------------------------------------

fn rewrite(text: &str) -> String {
    if !has_anchor(text) {
        return text.to_string();
    }
    let toks = tokenize(text);
    let marks = find_markers(text, &toks);
    if marks.is_empty() {
        return text.to_string();
    }
    let mut cover = vec![false; toks.len()];
    for m in &marks {
        cover[m.start..m.end].fill(true);
    }

    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    // First token still available to retract: everything before it has already
    // been deleted by an earlier repair in the same pass.
    let mut guard = 0;
    let mut capitalize = false;

    for m in &marks {
        if m.start < guard {
            continue;
        }
        let repair = repair_len(text, &toks, &cover, m);
        if repair.len == 0 {
            // Nothing follows the marker, so there is no repair to keep. A
            // trailing "I mean" stays on the screen rather than taking the
            // words before it down with it.
            continue;
        }
        if m.family == Family::IMean {
            // Gate 2: what the repair would start with…
            if in_list(HEDGE_HEADS, core(text_of(text, toks[m.end]))) {
                continue;
            }
            // …and what precedes the marker, for the one construction that
            // survives every other gate: "know what I mean, or shall I…".
            if m.start > 0 && in_list(CLAUSE_BEFORE, core(text_of(text, toks[m.start - 1]))) {
                continue;
            }
        }
        let Some(set_off) = set_off(text, &toks, m) else {
            continue;
        };
        // A full stop we cannot trust truncated the repair, so anything built
        // on it would be half a phrase: "Mr. Patel, I mean Dr. Patel" must not
        // become "Mr. Dr. Patel".
        if repair.abbreviated {
            continue;
        }
        let Some(win) = retract_window(text, &toks, &cover, m, repair, set_off, guard) else {
            continue;
        };
        if strands_a_modifier(text, &toks, win, guard)
            || leaves_a_function_word_fragment(text, &toks, win, guard)
            || is_an_appositive(text, &toks, win, m, repair, guard)
        {
            continue;
        }

        // Delete through the whitespace after the marker too, so the repair
        // lands exactly where the retracted phrase began. `n >= 1` guarantees
        // this token exists.
        let del_end = toks[m.end].start;
        let del_start = toks[win].start + kept_openers(text, toks[win], del_end);

        // The model capitalises the first word of a sentence. If the retracted
        // span held that position, the repair inherits it.
        let sentence_initial = win == 0
            || toks[win].breaks_line
            || break_after(text_of(text, toks[win - 1])) == Break::Sentence;
        let was_capitalised = text[del_start..]
            .chars()
            .next()
            .is_some_and(char::is_uppercase);

        push_segment(&mut out, &text[cursor..del_start], &mut capitalize);
        capitalize |= sentence_initial && was_capitalised;
        cursor = del_end;
        guard = m.end;
    }
    push_segment(&mut out, &text[cursor..], &mut capitalize);
    out
}

/// The first word of the sentence the window starts in, or `None` when the
/// window starts the sentence itself.
fn word_before_window(text: &str, toks: &[Tok], win: usize, guard: usize) -> Option<usize> {
    if win <= guard || toks[win].breaks_line {
        return None;
    }
    let prev = win - 1;
    (break_after(text_of(text, toks[prev])) != Break::Sentence).then_some(prev)
}

/// Refuse a rewrite that would leave a pre-nominal modifier attached to a head
/// noun that is no longer there.
///
/// "I'll bring the red one, no wait, blue" is the general case of correcting a
/// modified noun, and "I'll bring the red blue" is strictly worse than doing
/// nothing: the original reads as a self-evident correction a human can follow,
/// the output reads as a transcription failure. There is no deterministic way
/// to produce the right answer ("the blue one"), so the right move is to stand
/// down.
///
/// The shape is a determiner, then a content word, then the retracted span —
/// "the **red** ~~one~~", "the **morning** ~~flight~~". A determiner directly
/// against the window is fine ("book **the** ~~9am~~, no wait, the 10am slot"),
/// because then the repair replaces the whole noun phrase.
fn strands_a_modifier(text: &str, toks: &[Tok], win: usize, guard: usize) -> bool {
    let Some(prev) = word_before_window(text, toks, win, guard) else {
        return false;
    };
    let modifier = core(text_of(text, toks[prev]));
    if modifier.is_empty() || in_list(FUNCTION_WORDS, modifier) {
        return false;
    }
    // …and the word in front of the modifier has to be a determiner, or this
    // is an ordinary verb-object like "I said ~~Tuesday~~".
    word_before_window(text, toks, prev, guard)
        .is_some_and(|d| in_list(DETERMINERS, core(text_of(text, toks[d]))))
}

/// Refuse a rewrite that would leave the repair dangling off a fragment of
/// nothing but function words.
///
/// A fronted adverbial is one of the commonest openings in dictated English,
/// and the window happily eats its last word: "By the way, I mean Wednesday."
/// becomes "By the Wednesday." The test is not "is the fragment short" but "is
/// there any content left in it" — "meet at the coffee shop on ~~Tuesday~~, I
/// mean Wednesday" keeps its verb and its nouns, so it is fine.
///
/// An *empty* fragment is fine too: "Tuesday, I mean Wednesday" is the
/// canonical repair and there is nothing in front of it at all. So is a
/// subject and a copula — "it was ~~Tuesday~~, I mean Wednesday" — which is
/// why this checks [`FRAGMENT_WORDS`] and not the whole of [`FUNCTION_WORDS`].
fn leaves_a_function_word_fragment(text: &str, toks: &[Tok], win: usize, guard: usize) -> bool {
    let mut i = win;
    let mut saw_word = false;
    while let Some(prev) = word_before_window(text, toks, i, guard) {
        let word = core(text_of(text, toks[prev]));
        if !word.is_empty() {
            saw_word = true;
            if !in_list(FRAGMENT_WORDS, word) {
                return false;
            }
        }
        i = prev;
    }
    saw_word
}

/// Refuse a rewrite that would leave a comma stranded between a subject and its
/// verb.
///
/// "The waiting room, I mean the lobby, is on the left." is not a replacement
/// at all — the commas bracket an *appositive*, where "I mean" identifies
/// rather than corrects. Retracting the first half gives "The lobby, is on the
/// left.", with a comma the user never dictated sitting between subject and
/// verb. The same shape loses the name the sentence was about in "Ask Sarah, I
/// mean the new manager, about it."
///
/// The signature is narrow on purpose: it is `I mean` only, the retracted span
/// has to *start the sentence*, and both halves have to close with a comma. A
/// mid-sentence repair with the same punctuation ("his name is ~~Rob,~~ I mean
/// Robert, with a t") is grammatical after the rewrite and is left to fire, and
/// "no wait" never introduces an appositive at all — in "Tuesday, no wait,
/// Wednesday, I mean Thursday" the trailing comma is the next marker's, not an
/// apposition.
fn is_an_appositive(
    text: &str,
    toks: &[Tok],
    win: usize,
    m: &Mark,
    repair: Repair,
    guard: usize,
) -> bool {
    m.family == Family::IMean
        && word_before_window(text, toks, win, guard).is_none()
        && break_after(text_of(text, toks[m.start - 1])) == Break::Phrase
        && repair.closed_with_comma
}

/// How many bytes of opening quote or bracket at the start of the retracted
/// span survive the deletion.
///
/// `She said "send it Tuesday, I mean send it Wednesday."` must keep the quote
/// that is still open when the repair lands. `the file is "draft.txt", I mean
/// "final.txt"` must not: that quotation opens *and closes* inside the
/// retracted span, so the whole thing goes, punctuation included.
fn kept_openers(text: &str, tok: Tok, del_end: usize) -> usize {
    let mut kept = 0;
    for c in text_of(text, tok).chars() {
        if !is_opening(c) {
            break;
        }
        let after = tok.start + kept + c.len_utf8();
        if text[after..del_end].contains(closer_of(c)) {
            break;
        }
        kept += c.len_utf8();
    }
    kept
}

fn closer_of(c: char) -> char {
    match c {
        '\u{201c}' | '\u{201e}' => '\u{201d}',
        '\u{2018}' => '\u{2019}',
        '\u{ab}' => '\u{bb}',
        '\u{2039}' => '\u{203a}',
        '(' => ')',
        '[' => ']',
        '{' => '}',
        other => other,
    }
}

/// Copy a kept segment, restoring the capitalisation the retracted words held.
///
/// Both models capitalise the first word of a sentence, so "The red one, I
/// mean the blue one" would otherwise resolve to a lower-case "the blue one" —
/// text the user could never have produced by simply not misspeaking. Only
/// fires when the retracted span both started a sentence and was itself
/// capitalised, and never re-cases a word already carrying an upper-case
/// letter, so "iPhone" is safe. An empty segment leaves the flag pending for
/// the next one, which is what back-to-back repairs need.
fn push_segment(out: &mut String, seg: &str, capitalize: &mut bool) {
    if seg.is_empty() {
        return;
    }
    if *capitalize {
        *capitalize = false;
        let first = seg.chars().next().unwrap_or(' ');
        let rest = &seg[first.len_utf8()..];
        let word_rest = rest.split(char::is_whitespace).next().unwrap_or("");
        if first.is_lowercase() && !word_rest.chars().any(char::is_uppercase) {
            out.extend(first.to_uppercase());
            out.push_str(rest);
            return;
        }
    }
    out.push_str(seg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{assert_noop, prefix_violation, torture_inputs, truncate};

    fn on() -> SelfCorrect {
        SelfCorrect::new(SelfCorrectConfig { enabled: true })
    }

    fn go(input: &str) -> String {
        on().apply(input)
    }

    /// The one torture input that this transform is *supposed* to rewrite.
    const TORTURE_CORRECTION: (&str, &str) =
        ("I said Tuesday, I mean Wednesday", "I said Wednesday");

    // ---- the corpus -------------------------------------------------------
    //
    // The most important test in the module. Every marker appears in both
    // directions: once where it is a real repair and must resolve, once where
    // the same words are ordinary prose and must survive byte for byte.

    /// Marked repairs that must resolve to the corrected text.
    const MARKED: &[(&str, &str)] = &[
        // "I mean" / "I meant"
        ("I said Tuesday, I mean Wednesday", "I said Wednesday"),
        (
            "Let's meet Tuesday, I meant Wednesday",
            "Let's meet Wednesday",
        ),
        (
            "The meeting is at three, I mean at four",
            "The meeting is at four",
        ),
        (
            "we need three chairs, I mean four chairs, by Friday",
            "we need four chairs, by Friday",
        ),
        // "sorry, I meant"
        (
            "we need three chairs, sorry, I meant four chairs",
            "we need four chairs",
        ),
        (
            "ship it Tuesday, sorry, I mean Wednesday",
            "ship it Wednesday",
        ),
        // "no wait"
        (
            "send it to Bob, no wait, send it to Alice",
            "send it to Alice",
        ),
        (
            "I'll take the red one, no wait, the blue one",
            "I'll take the blue one",
        ),
        ("Tuesday, no wait, it's Wednesday", "It's Wednesday"),
        // "scratch that"
        (
            "Let's meet Tuesday, scratch that, let's meet Wednesday",
            "Let's meet Wednesday",
        ),
        (
            "invoice them for 200, scratch that, 250",
            "invoice them for 250",
        ),
        // "correction"
        (
            "Ship it Tuesday, correction, Wednesday",
            "Ship it Wednesday",
        ),
        // mixed case
        ("Book it Tuesday, NO WAIT, Wednesday", "Book it Wednesday"),
        ("Book it Tuesday, I Mean Wednesday", "Book it Wednesday"),
        // nested
        ("Tuesday, no wait, Wednesday, I mean Thursday", "Thursday"),
        // hesitation noise between the phrase and the marker
        (
            "we ship Tuesday, well, no wait, Wednesday",
            "we ship Wednesday",
        ),
        // a repair inside a quotation retracts inside the quotation only
        (
            "She said, \"meet Tuesday, I mean Wednesday.\"",
            "She said, \"meet Wednesday.\"",
        ),
        (
            "She said, \"send it Tuesday, I mean send it Wednesday.\"",
            "She said, \"send it Wednesday.\"",
        ),
        // sentence-initial capitalisation is inherited by the repair
        ("The red one, I mean the blue one", "The blue one"),
        // …and the whole second batch, from a sweep over realistic dictation
        (
            "Book the 9am, no wait, the 10am slot.",
            "Book the 10am slot.",
        ),
        (
            "It costs 40, correction, 45 dollars.",
            "It costs 45 dollars.",
        ),
        (
            "Push to main, scratch that, push to the branch.",
            "Push to the branch.",
        ),
        (
            "I said the red one, I mean the one on the left",
            "I said the one on the left",
        ),
        ("we ship on the 3rd, I mean the 13th", "we ship on the 13th"),
        (
            "the box is in the garage, I mean the shed",
            "the box is in the shed",
        ),
        (
            "I will call the client on Monday, I mean the other client",
            "I will call the other client",
        ),
        (
            "the report covers Q1 and Q2, I mean Q3",
            "the report covers Q1 and Q3",
        ),
        (
            "the flight lands at 6, I mean at 7 in the morning",
            "the flight lands at 7 in the morning",
        ),
        (
            "she works at Google, I mean at Meta now",
            "she works at Meta now",
        ),
        ("add 3 cups of flour, no wait, 4 cups", "add 4 cups"),
        ("we lost 3 to 1, I mean 3 to 2", "we lost 3 to 2"),
        (
            "meet me at the station, scratch that, at the airport",
            "meet me at the airport",
        ),
        (
            "his name is Rob, I mean Robert, with a t",
            "his name is Robert, with a t",
        ),
        ("reply to Bob, I mean reply all", "reply all"),
        (
            "the plan is Tuesday, I mean the plan is Wednesday",
            "the plan is Wednesday",
        ),
    ];

    /// The same words, used the way people actually use them. Every one of
    /// these must come out byte-identical.
    const UNMARKED: &[&str] = &[
        // the four "I mean" hedges named in #48
        "I mean, that's just my opinion",
        "I mean it sincerely",
        "what do you mean by that",
        "I mean well",
        // more hedging, in positions where a naive window would bite
        "and I mean it sincerely",
        "Honestly I mean it.",
        "You know what I mean, right?",
        "That is not what I meant.",
        "I said it was fine, I mean, that's just my opinion",
        "um, I mean, like, the thing",
        "I mean, we could always ship it on Friday",
        "the things I mean are all on the list",
        // "no wait" as ordinary prose
        "we found no wait staff at the venue",
        "There's no wait, come on in.",
        "the queue had no wait times today",
        // "scratch that" as ordinary prose
        "don't scratch that mosquito bite",
        "tell the cat not to scratch that couch",
        // "correction" as an ordinary noun
        "the correction was applied to the invoice",
        "Please make a correction, Wednesday works better",
        "I need to file a correction",
        // markers inside quotations
        "He said \"no wait\" and left.",
        "The prompt says \"I mean Wednesday\" verbatim.",
        // unmarked corrections — #37's job, not ours
        "meet Tuesday. Wednesday.",
        "let's do it Tuesday Wednesday",
        "the red one the blue one",
        // …and the second sweep batch: prose that survived every gate
        "I mean, you know, it depends on the day",
        "Do you know what I mean, or should I explain again?",
        "I did not mean to interrupt, sorry about that.",
        "The mean of the sample is 4.2, the median is 4.",
        "What does that mean, exactly?",
        "It was a mean trick, honestly.",
        "Well, I mean, sure, if you insist.",
        "Please wait, the file is still uploading.",
        "Wait, did you say Tuesday or Wednesday?",
        "We had to wait, no one told us the room had moved.",
        "Do not scratch that, it will leave a mark.",
        "Scratch that off the list, we already shipped it.",
        "Start from scratch, that is the only way.",
        "I filed a correction, the numbers were off.",
        "The correction, however, arrived too late.",
        "correction: the total was 4200",
        "Sorry, I did not catch that.",
        "I am sorry, I meant no offence.",
        "Tell him sorry, Wednesday works better for us.",
        "No, wait for the second batch instead.",
        "the answer is yes, I mean no",
        "we sold the house on the corner, I mean the price was good",
        "the total is 40 dollars, I mean including tax",
        "we can ship Tuesday, I mean, honestly, whenever",
        "I mean, I think we should wait",
        "wait for the green light, then go",
        "does that mean, I wonder, that we are done",
    ];

    #[test]
    fn marked_corrections_resolve() {
        for (input, want) in MARKED {
            assert_eq!(&go(input), want, "on {input:?}");
        }
    }

    #[test]
    fn unmarked_and_hedged_text_passes_through_untouched() {
        for input in UNMARKED {
            assert_eq!(&go(input), input, "rewrote {input:?}");
        }
    }

    /// #48's headline acceptance criterion, called out on its own so it cannot
    /// be quietly deleted with a corpus edit: an unmarked correction is passed
    /// through rather than guessed at. Detecting these needs a model and
    /// belongs to #37.
    #[test]
    fn unmarked_corrections_pass_through() {
        for input in [
            "meet Tuesday. Wednesday.",
            "let's do it on Tuesday. no, Wednesday.",
            "his name is Bob. Rob.",
        ] {
            assert_eq!(&go(input), input, "guessed at an unmarked correction");
        }
    }

    /// Every marker, in both directions, in one table. Belt and braces over
    /// the corpus above: if someone adds a marker to `MARKERS` without a
    /// false-positive case, this is where the omission shows up.
    #[test]
    fn every_marker_fires_and_every_marker_can_be_ordinary_prose() {
        let pairs = [
            (
                "meet Tuesday, I mean Wednesday",
                "meet Wednesday",
                "I mean it",
            ),
            (
                "meet Tuesday, I meant Wednesday",
                "meet Wednesday",
                "that is what I meant",
            ),
            (
                "meet Tuesday, sorry, I mean Wednesday",
                "meet Wednesday",
                "sorry I mean it",
            ),
            (
                "meet Tuesday, sorry, I meant Wednesday",
                "meet Wednesday",
                "sorry that is what I meant",
            ),
            (
                "meet Tuesday, no wait, Wednesday",
                "meet Wednesday",
                "the clinic has no wait times",
            ),
            (
                "meet Tuesday, scratch that, Wednesday",
                "meet Wednesday",
                "do not scratch that surface",
            ),
            (
                "meet Tuesday, correction, Wednesday",
                "meet Wednesday",
                "the correction is in the appendix",
            ),
        ];
        for (marked, want, prose) in pairs {
            assert_eq!(go(marked), want, "marker did not fire in {marked:?}");
            assert_eq!(go(prose), prose, "marker fired in prose {prose:?}");
        }
    }

    // ---- the window -------------------------------------------------------

    /// The window is the repair's own length, so a long comma-free run-on
    /// loses one word, not the whole clause. This is the case the "drop back
    /// to the previous phrase boundary" reading gets wrong.
    #[test]
    fn the_window_matches_the_length_of_the_repair() {
        assert_eq!(
            go("meet at the coffee shop on Tuesday, I mean Wednesday"),
            "meet at the coffee shop on Wednesday"
        );
        assert_eq!(
            go("send the quarterly report to Bob, I mean the annual report to Alice"),
            "send the annual report to Alice"
        );
    }

    /// The acceptance criterion from #48. The repair here is six words long,
    /// so a window that only counted words would reach back through "Friday."
    /// and eat the previous sentence; the sentence boundary stops it at one.
    #[test]
    fn the_window_never_crosses_a_sentence_boundary_backwards() {
        // Both commas, so the four-word repair gets full latitude and the only
        // thing holding the window to one word is the full stop after "Friday".
        let input = "Call Bob on Friday. Tuesday, I mean, Wednesday afternoon works better.";
        let got = go(input);
        assert_eq!(got, "Call Bob on Friday. Wednesday afternoon works better.");
        assert!(
            got.starts_with("Call Bob on Friday."),
            "ate the previous sentence: {got:?}"
        );

        // …and the same for ! ? and a bare line break.
        assert_eq!(
            go("Book it now! Tuesday, I mean, Wednesday morning at nine sharp"),
            "Book it now! Wednesday morning at nine sharp"
        );
        assert_eq!(
            go("Which day? Tuesday, I mean, Wednesday morning at nine sharp"),
            "Which day? Wednesday morning at nine sharp"
        );
        assert_eq!(
            go("Call Bob on Friday\nTuesday, I mean, Wednesday morning at nine"),
            "Call Bob on Friday\nWednesday morning at nine"
        );
    }

    /// A repair that starts a new sentence has nothing to retract inside it,
    /// and reaching into the previous sentence is exactly what is forbidden.
    /// Conservative on purpose: the marker stays rather than the sentence
    /// before it disappearing.
    #[test]
    fn a_marker_after_a_full_stop_resolves_to_nothing() {
        for input in [
            "Let's meet Tuesday. Sorry, I meant Wednesday.",
            "Let's meet Tuesday. No wait, Wednesday.",
            "Let's meet Tuesday. Scratch that, Wednesday.",
        ] {
            assert_eq!(&go(input), input, "crossed a sentence boundary");
        }
    }

    #[test]
    fn the_window_stops_at_the_previous_phrase_boundary() {
        // the repair is four words, but only two are available before the comma
        assert_eq!(
            go("on Monday, at three, I mean at four in the afternoon"),
            "on Monday, at four in the afternoon"
        );
    }

    /// [`MAX_WINDOW`] is the backstop for a transcript with no punctuation
    /// except the marker's own comma.
    #[test]
    fn the_window_is_bounded_even_with_no_other_punctuation() {
        let long = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike \
                    november oscar papa";
        let input = format!("{long}, I mean, {long}");
        let got = go(&input);
        let kept: Vec<&str> = got.split_whitespace().collect();
        // 16 words in, 16 out, minus the 12-word ceiling on the retraction.
        // A literal, not `16 + 16 - MAX_WINDOW`: written that way the ceiling
        // could be changed to anything in 1..16 and this test would follow it.
        assert_eq!(kept.len(), 20);
        assert!(got.starts_with("alpha bravo charlie delta"), "{got}");
    }

    // ---- adversarial ------------------------------------------------------

    #[test]
    fn a_marker_with_nothing_before_it_never_empties_the_output() {
        for input in [
            "No wait, Wednesday works",
            "I mean Wednesday",
            "Scratch that, Wednesday",
            "Correction, Wednesday",
            "Sorry, I meant Wednesday",
        ] {
            let got = go(input);
            assert!(!got.is_empty(), "emptied {input:?}");
            assert_eq!(&got, input, "rewrote {input:?} with nothing to retract");
        }
    }

    #[test]
    fn a_marker_with_nothing_after_it_retracts_nothing() {
        for input in [
            "I said Tuesday, I mean",
            "I said Tuesday, no wait",
            "I said Tuesday, scratch that",
            "I said Tuesday, correction,",
            "I said Tuesday, sorry, I meant",
        ] {
            assert_eq!(&go(input), input, "retracted on a dangling marker");
        }
    }

    #[test]
    fn an_utterance_that_is_only_a_marker_is_left_alone() {
        for input in [
            "I mean",
            "I mean,",
            "no wait",
            "No wait.",
            "scratch that",
            "Scratch that.",
            "correction,",
            "sorry, I meant",
        ] {
            assert_eq!(&go(input), input, "rewrote a bare marker");
        }
    }

    #[test]
    fn two_markers_in_a_row_resolve_once() {
        assert_eq!(go("Tuesday, I mean, no wait, Wednesday"), "Wednesday");
        assert_eq!(go("Tuesday, no wait, I mean, Wednesday"), "Wednesday");
        assert_eq!(
            go("ship it Tuesday, no wait, sorry, I meant Wednesday"),
            "ship it Wednesday"
        );
    }

    #[test]
    fn nested_corrections_keep_only_the_last() {
        assert_eq!(
            go("Tuesday, no wait, Wednesday, I mean Thursday"),
            "Thursday"
        );
        assert_eq!(
            go("meet Tuesday, I mean Wednesday, no wait, Thursday"),
            "meet Thursday"
        );
        assert_eq!(
            go("call Bob, I mean Rob, no wait, Robert, scratch that, Bobby"),
            "call Bobby"
        );
    }

    #[test]
    fn a_marker_inside_a_quotation_is_quoted_material() {
        for input in [
            "He said \"no wait\" and left.",
            "He said \"I mean Wednesday\" and left.",
            "The rule is \"scratch that\" in full.",
        ] {
            assert_eq!(&go(input), input, "rewrote quoted material");
        }
        // but a repair *of* a quotation still works, and stays balanced
        assert_eq!(
            go("the file is \"draft.txt\", I mean \"final.txt\""),
            "the file is \"final.txt\""
        );
        assert_eq!(
            go("She said \"Tuesday\", I mean \"Wednesday\""),
            "She said \"Wednesday\""
        );
    }

    #[test]
    fn mixed_case_markers_match() {
        for input in [
            "meet Tuesday, I MEAN Wednesday",
            "meet Tuesday, i mean Wednesday",
            "meet Tuesday, No Wait, Wednesday",
            "meet Tuesday, SCRATCH THAT, Wednesday",
            "meet Tuesday, Correction, Wednesday",
        ] {
            assert_eq!(go(input), "meet Wednesday", "on {input:?}");
        }
    }

    /// Repairs of proper nouns and numbers, the two things dictation gets
    /// wrong most often, and the reason anyone wants this feature.
    #[test]
    fn repairs_names_numbers_and_dates() {
        assert_eq!(go("call Bob, I mean Rob"), "call Rob");
        assert_eq!(
            go("that's 15 dollars, I mean 50 dollars"),
            "that's 50 dollars"
        );
        assert_eq!(
            go("the deadline is March 3rd, no wait, March 13th"),
            "the deadline is March 13th"
        );
    }

    // ---- the "I mean" collision with #44 ----------------------------------

    /// The four cases #48 calls out by name, plus the rule that separates
    /// them. Cross-check against #44: everything here stays in the text for
    /// filler removal to handle as a hedge.
    #[test]
    fn the_four_i_mean_hedges_from_the_issue() {
        for input in [
            "I mean, that's just my opinion",
            "I mean it sincerely",
            "what do you mean by that",
            "I mean well",
        ] {
            assert_eq!(&go(input), input, "treated a hedge as a repair: {input:?}");
        }
    }

    /// Gate 2 in isolation: a repair replaces a content constituent, so a
    /// pronoun, auxiliary, complementiser or discourse particle after "I mean"
    /// means hedging. Same left-hand side, opposite verdicts.
    #[test]
    fn a_repair_never_begins_with_a_function_word() {
        for head in [
            "it", "that's", "this", "there's", "you", "we", "he", "is", "was", "will", "would",
            "what", "when", "because", "well", "yeah", "okay", "just", "like", "actually",
            "honestly", "to", "by", "business", "nothing",
        ] {
            let input = format!("I said it was fine, I mean {head} something");
            assert_eq!(
                go(&input),
                input,
                "hedge head {head:?} was treated as a repair"
            );
        }
        // The same left-hand side with a head that *can* start a repair. Both
        // commas, so `single_comma_restraint` is not what is being measured.
        for head in [
            "Wednesday",
            "the",
            "a",
            "four",
            "on",
            "at",
            "my",
            "our",
            "Bob",
        ] {
            let input = format!("I said Tuesday, I mean, {head} something");
            assert_ne!(go(&input), input, "repair head {head:?} was blocked");
        }
    }

    /// Gate 1 in isolation. Without the prosodic break the models render as a
    /// comma, "I mean" is a verb and "no wait" is a noun phrase.
    #[test]
    fn a_marker_with_no_punctuation_around_it_is_not_a_marker() {
        for input in [
            "honestly I mean it",
            "the things I mean are on the list",
            "we found no wait staff",
            "please do not scratch that surface",
        ] {
            assert_eq!(&go(input), input, "fired without set-off punctuation");
        }
        // the same words, set off, do fire
        assert_eq!(go("it was Tuesday, I mean Wednesday"), "it was Wednesday");
    }

    /// Gate 3 in isolation: retracting nothing but function words means the
    /// match was a coincidence.
    #[test]
    fn a_window_of_pure_function_words_is_not_a_repair() {
        for input in [
            "You know what I mean, right?",
            "There's no wait, come on in.",
            "and I mean it works",
            // gate 3 on its own: every other gate passes here, and the only
            // thing wrong is that the window would retract the word "it"
            "we talked about it, I mean Wednesday",
            "she waited for them, I mean Wednesday",
        ] {
            assert_eq!(&go(input), input, "retracted a function word: {input:?}");
        }
    }

    /// The boundary with filler removal, shape by shape: this transform claims
    /// every "I mean" with something to retract in front of it, and leaves
    /// every one without.
    ///
    /// The claim rests on chain order, not on any behaviour of #44 — as merged
    /// it ships `off` and `light`, and "i mean" is only ever consulted at the
    /// gated `medium` level, so today nothing else in the chain touches these
    /// strings at all. The row worth stating out loud is **both commas**,
    /// "Let's meet Tuesday, I mean, Wednesday.": it resolves here, so it never
    /// reaches filler removal in any configuration.
    #[test]
    fn the_i_mean_shapes_split_cleanly_with_fillers() {
        // ours: something to retract, whatever the commas do
        assert_eq!(
            go("Let's meet Tuesday, I mean, Wednesday."),
            "Let's meet Wednesday."
        );
        assert_eq!(go("Meet Tuesday, I mean Wednesday."), "Meet Wednesday.");
        assert_eq!(go("call Bob, I mean, Rob"), "call Rob");
        assert_eq!(go("call Bob, I mean, call Rob"), "call Rob");
        assert_eq!(go("it costs 40, I mean, 45"), "it costs 45");

        // not ours: a hedge with nothing in front of it to retract
        for hedge in [
            "I mean, we could try",
            "I mean Wednesday.",
            "Meet Tuesday. I mean Wednesday.",
            "I mean, I think we should wait",
        ] {
            assert_eq!(&go(hedge), hedge, "claimed a hedge that is not ours");
        }
    }

    /// Speakers repair by restarting the constituent, so the repair usually
    /// repeats its first word. Aligning on that beats counting words, which
    /// loses the verb ("Book the 9am, no wait, the 10am slot") or duplicates
    /// one ("call Bob at the office, I mean at home").
    #[test]
    fn the_window_aligns_on_the_repeated_word() {
        assert_eq!(
            go("call Bob at the office, I mean at home"),
            "call Bob at home"
        );
        assert_eq!(
            go("put the box on the shelf, I mean the big table"),
            "put the box on the big table"
        );
        assert_eq!(
            go("send the quarterly report to Bob, I mean the annual report to Alice"),
            "send the annual report to Alice"
        );
    }

    /// Numbers align by kind, because the whole point of a numeric repair is
    /// that the two numbers differ — but an exact repetition still wins, or
    /// "we lost 3 to 1, I mean 3 to 2" becomes "we lost 3 to 3 to 2".
    #[test]
    fn numbers_align_by_kind_but_an_exact_repeat_wins() {
        assert_eq!(
            go("it costs 40, correction, 45 dollars"),
            "it costs 45 dollars"
        );
        assert_eq!(
            go("that's 15 dollars, I mean 50 dollars"),
            "that's 50 dollars"
        );
        assert_eq!(go("we lost 3 to 1, I mean 3 to 2"), "we lost 3 to 2");
        assert_eq!(go("call 555 1234, I mean 555 4321"), "call 555 4321");
    }

    /// A window guessed from the repair's *length* stops in front of the verb
    /// holding the clause together. Without this, a three-word repair reaches
    /// through "is" and takes the subject with it.
    #[test]
    fn a_length_guessed_window_does_not_cross_the_clause_verb() {
        // both commas, so `single_comma_restraint` is out of the way and this
        // is the clause-spine stop on its own
        assert_eq!(
            go("our target is Q4, I mean, end of year"),
            "our target is end of year"
        );
        assert_eq!(
            go("the deadline was Friday, I mean, start of next week"),
            "the deadline was start of next week"
        );
        // an *aligned* window may cross one: the repeated words are evidence
        // that the speaker restarted the whole clause
        assert_eq!(
            go("the plan is Tuesday, I mean the plan is Wednesday"),
            "the plan is Wednesday"
        );
    }

    /// An "I mean" introducing a whole clause is a speaker elaborating unless
    /// the clause visibly restarts an earlier one — and a shared "the" is not
    /// evidence of that, so the second word has to match too.
    #[test]
    fn a_clause_shaped_i_mean_needs_real_parallelism() {
        for elaboration in [
            "we sold the house on the corner, I mean the price was good",
            "the invoice went out Tuesday, I mean the client has not paid",
            "I booked the room, I mean the projector is broken",
        ] {
            assert_eq!(&go(elaboration), elaboration, "retracted an elaboration");
        }
        // parallel restarts still resolve
        assert_eq!(
            go("the plan is Tuesday, I mean the plan is Wednesday"),
            "the plan is Wednesday"
        );
        // and "no wait" is unambiguous enough not to need the check
        assert_eq!(go("Tuesday, no wait, it's Wednesday"), "It's Wednesday");
    }

    /// The ordering contract from `lib.rs`, from this side of the seam: the
    /// markers this transform needs are still in the text when it runs,
    /// because it runs before filler removal. `lib.rs` owns the chain-order
    /// test; this one owns the consequence.
    #[test]
    fn markers_survive_only_because_fillers_run_later() {
        // what this transform sees
        assert_eq!(go("meet Tuesday, I mean Wednesday"), "meet Wednesday");
        // what it would see if #44 had already stripped the hedge
        assert_eq!(go("meet Tuesday, Wednesday"), "meet Tuesday, Wednesday");
    }

    // ---- invariants -------------------------------------------------------

    #[test]
    fn disabled_is_byte_identical() {
        assert_noop(&SelfCorrect::new(SelfCorrectConfig::default()));
        let off = SelfCorrect::new(SelfCorrectConfig::default());
        for (input, _) in MARKED {
            assert_eq!(&off.apply(input), input, "disabled rewrote {input:?}");
        }
    }

    #[test]
    fn disabled_by_default() {
        assert!(!SelfCorrectConfig::default().enabled);
    }

    /// Enabled, the torture corpus is untouched except for the one entry that
    /// is a marked correction. Covers ZWJ sequences, skin-tone modifiers,
    /// combining marks, zero-width characters, CJK, Cyrillic and the 2 MB
    /// input.
    #[test]
    fn the_torture_corpus_survives_except_the_one_marked_correction() {
        let t = on();
        for input in torture_inputs() {
            let out = t.apply(&input);
            if input == TORTURE_CORRECTION.0 {
                assert_eq!(out, TORTURE_CORRECTION.1);
            } else {
                assert_eq!(out, input, "changed {:?}", truncate(&input));
            }
        }
    }

    #[test]
    fn is_idempotent() {
        let t = on();
        let mut corpus: Vec<String> = torture_inputs();
        corpus.extend(MARKED.iter().map(|(i, _)| (*i).to_string()));
        corpus.extend(MARKED.iter().map(|(_, o)| (*o).to_string()));
        corpus.extend(UNMARKED.iter().map(|s| (*s).to_string()));
        for input in corpus {
            let once = t.apply(&input);
            assert_eq!(
                t.apply(&once),
                once,
                "not idempotent on {:?}",
                truncate(&input)
            );
        }
    }

    /// The trap #48 calls out: a repair whose own text contains a marker word.
    /// The second pass must not fire again.
    #[test]
    fn a_repair_containing_a_marker_word_does_not_fire_twice() {
        let t = on();
        for input in [
            "I said Tuesday, I mean I mean it",
            "call Bob, I mean tell him I mean it",
            "the sign said Tuesday, no wait, the sign said no wait",
            "book it Tuesday, scratch that, tell them to scratch that",
            "file it Tuesday, correction, file the correction, please",
        ] {
            let once = t.apply(input);
            assert_eq!(t.apply(&once), once, "fired twice on {input:?}");
        }
    }

    #[test]
    fn is_not_prefix_stable() {
        assert!(!SelfCorrect::new(SelfCorrectConfig::default()).prefix_stable());
        assert!(!on().prefix_stable());
    }

    /// The counterexample recorded in [`Transform::prefix_stable`], produced
    /// by the real implementation rather than asserted from a doc comment.
    #[test]
    fn prefix_violation_finds_the_documented_counterexample() {
        let (prefix, polished_prefix, polished_whole) =
            prefix_violation(&on(), TORTURE_CORRECTION.0)
                .expect("must violate the prefix property");
        assert_eq!(prefix, "I said T");
        assert_eq!(polished_prefix, "I said T");
        assert_eq!(polished_whole, "I said Wednesday");
        assert!(!polished_whole.starts_with(&polished_prefix));

        // and it violates on every marked case in the corpus, not just that one
        for (input, _) in MARKED {
            assert!(
                prefix_violation(&on(), input).is_some(),
                "no violation found for {input:?}"
            );
        }
    }

    /// Nothing here is user-authored, so there is nothing to validate. If a
    /// marker list ever becomes configurable, this test is the reminder that
    /// it needs a `validate` override.
    #[test]
    fn nothing_to_validate() {
        assert!(on().validate().is_empty());
        assert!(SelfCorrect::new(SelfCorrectConfig::default())
            .validate()
            .is_empty());
    }

    #[test]
    fn output_is_never_empty_for_non_empty_input() {
        let t = on();
        let mut corpus: Vec<String> = MARKED.iter().map(|(i, _)| (*i).to_string()).collect();
        corpus.extend(UNMARKED.iter().map(|s| (*s).to_string()));
        for input in corpus {
            assert!(!t.apply(&input).trim().is_empty(), "emptied {input:?}");
        }
    }

    // ---- performance ------------------------------------------------------

    /// Anti-quadratic tripwire, not a benchmark: the bound is deliberately far
    /// looser than anything a linear pass needs, because CI runners are shared.
    ///
    /// Measured on Apple Silicon, release build, and printed by this test
    /// under `--nocapture`: a realistic 13-word utterance containing a
    /// correction costs **7.7 us**, and one containing no marker word at all
    /// costs **575 ns**, because [`has_anchor`] short-circuits before anything
    /// is tokenized. Against a transcription pass measured in hundreds of
    /// milliseconds, both are noise.
    #[test]
    fn cost_is_linear_in_the_length_of_the_transcript() {
        let t = on();

        // the realistic case, reported with `--nocapture`
        for utterance in [
            "Let's push the release to Tuesday, I mean Wednesday, and tell the team.",
            "Let's push the release to Wednesday and tell the team afternoon.",
        ] {
            let start = std::time::Instant::now();
            for _ in 0..1000 {
                let _ = t.apply(utterance);
            }
            println!("{:?}/utterance for {utterance:?}", start.elapsed() / 1000);
        }

        // The tripwire is a *ratio*, not a wall clock: doubling the input must
        // roughly double the work. A wall-clock bound flakes on a shared CI
        // runner under parallel load, and a generous one stops catching
        // anything; this catches quadratic growth regardless of machine speed.
        let unit = "send it Tuesday, I mean Wednesday. ";
        let time = |reps: usize| {
            let input = unit.repeat(reps);
            let want = "send it Wednesday. ".repeat(reps);
            let start = std::time::Instant::now();
            let got = t.apply(&input);
            let elapsed = start.elapsed();
            assert_eq!(got, want, "wrong output at {reps} repetitions");
            elapsed.as_secs_f64()
        };

        // warm up, then measure n and 2n
        let _ = time(2_000);
        let n = time(25_000).max(1e-6);
        let two_n = time(50_000);
        let ratio = two_n / n;
        println!("2n/n = {ratio:.2} ({n:.4}s -> {two_n:.4}s)");
        assert!(
            ratio < 3.0,
            "cost is growing faster than linearly: doubling the input multiplied the time by \
             {ratio:.2} ({n:.4}s -> {two_n:.4}s)"
        );

        // the marker-free corpus, including a 2 MB input and a 100k-character
        // single token, must not be quadratic either
        let start = std::time::Instant::now();
        for input in torture_inputs() {
            let _ = t.apply(&input);
        }
        println!("torture sweep {:?}", start.elapsed());
    }

    // ---- unit-level behaviour ---------------------------------------------

    #[test]
    fn whitespace_and_punctuation_around_the_repair_are_preserved() {
        assert_eq!(
            go("   meet Tuesday, I mean Wednesday   "),
            "   meet Wednesday   "
        );
        assert_eq!(go("meet Tuesday, I mean Wednesday.\n"), "meet Wednesday.\n");
        assert_eq!(
            go("line one\nmeet Tuesday, I mean Wednesday\nline three"),
            "line one\nmeet Wednesday\nline three"
        );
    }

    #[test]
    fn capitalisation_is_inherited_only_from_a_capitalised_sentence_start() {
        // sentence-initial and capitalised: the repair inherits the capital
        assert_eq!(go("The red one, I mean the blue one"), "The blue one");
        assert_eq!(
            go("Ship it. The red one, I mean the blue one"),
            "Ship it. The blue one"
        );
        // sentence-initial but dictated lower-case: leave it lower-case
        assert_eq!(go("the red one, I mean the blue one"), "the blue one");
        assert_eq!(
            go("send it to Bob, no wait, send it to Alice"),
            "send it to Alice"
        );
        // mid-sentence: never re-case, even after a capitalised proper noun
        assert_eq!(
            go("I'll take the red one, I mean the blue one"),
            "I'll take the blue one"
        );
        assert_eq!(go("I'll take Red, I mean blue"), "I'll take blue");
        // never re-case a word that already carries capitals
        assert_eq!(
            go("The Android one, I mean the iPhone one"),
            "The iPhone one"
        );
        assert_eq!(go("Red models, I mean, iPhone models"), "iPhone models");
        // and it is Unicode-aware, not ASCII-only
        assert_eq!(go("Правда, I mean истина"), "Истина");
    }

    #[test]
    fn multibyte_text_around_a_repair_is_untouched() {
        assert_eq!(
            go("naïve café, I mean résumé, is the word"),
            "naïve résumé, is the word"
        );
        assert_eq!(
            go("👩‍💻 shipped Tuesday, I mean Wednesday 🚀"),
            "👩‍💻 shipped Wednesday 🚀"
        );
        // a repair works the same in a script with no ASCII in it at all
        assert_eq!(go("говорю правда, I mean истина"), "говорю истина");
        assert_eq!(
            go("日本語です、I mean 中国語です"),
            "日本語です、I mean 中国語です"
        );
    }

    #[test]
    fn tokenizer_keeps_zero_width_characters_inside_words() {
        // the zero-width run is a word like any other: kept when it is not
        // retracted, and byte-identical when it is kept
        assert_eq!(
            go("\u{200b}zero\u{200b}width\u{200b} Tuesday, I mean Wednesday"),
            "\u{200b}zero\u{200b}width\u{200b} Wednesday"
        );
        // …and it is a word the user can retract, not invisible padding
        assert_eq!(
            go("call it \u{200b}zero\u{200b}width\u{200b}, I mean invisible"),
            "call it invisible"
        );
    }

    #[test]
    fn core_strips_only_edge_punctuation() {
        assert_eq!(core("wait,"), "wait");
        assert_eq!(core("\"no"), "no");
        assert_eq!(core("don't"), "don't");
        assert_eq!(core("3.5"), "3.5");
        assert_eq!(core("..."), "");
        assert_eq!(core("mind.\""), "mind");
    }

    #[test]
    fn break_after_looks_through_closing_quotes() {
        assert_eq!(break_after("mind.\""), Break::Sentence);
        assert_eq!(break_after("wait,\""), Break::Phrase);
        assert_eq!(break_after("Wednesday"), Break::None);
        assert_eq!(break_after("3.5"), Break::None);
        // "Dr." really is a sentence end as far as `break_after` is concerned,
        // and that is deliberate — the window must never cross a real one. The
        // abbreviation is caught at the other end, by standing the rewrite
        // down; `is_abbreviation` and `the_adversarial_review_corpus` pin that.
        assert_eq!(break_after("Dr."), Break::Sentence);
        assert!(is_abbreviation("Dr."));
        assert!(is_abbreviation("Jan."));
        assert!(is_abbreviation("U.S."));
        assert!(!is_abbreviation("Friday."));
        assert!(!is_abbreviation("Wednesday"));
        assert_eq!(break_after("\""), Break::None);
    }

    /// The word lists are whitespace-separated strings, which buys compact,
    /// category-grouped source at the cost of one typo class: a missing space
    /// silently creates a nonsense entry ("i i'mi've"). This is the guard.
    #[test]
    fn word_lists_are_well_formed() {
        for (name, list, min) in [
            ("HEDGE_HEADS", HEDGE_HEADS, 150),
            ("CLAUSE_BEFORE", CLAUSE_BEFORE, 5),
            ("CLAUSE_SPINE", CLAUSE_SPINE, 40),
            ("ABSORBED", ABSORBED, 15),
            ("FUNCTION_WORDS", FUNCTION_WORDS, 100),
        ] {
            let words: Vec<&str> = list.split_ascii_whitespace().collect();
            assert!(words.len() >= min, "{name} lost entries: {}", words.len());
            for w in &words {
                assert!(
                    w.chars().all(|c| c.is_ascii_lowercase() || c == '\''),
                    "{name} has a malformed entry {w:?} — probably a missing space"
                );
                assert!(w.len() <= 14, "{name} entry {w:?} looks like two words");
            }
            let mut sorted = words.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), words.len(), "{name} has a duplicate entry");
        }
        // and the lookups themselves still work in both directions
        assert!(in_list(HEDGE_HEADS, "It"));
        assert!(in_list(HEDGE_HEADS, "unfortunately"));
        assert!(!in_list(HEDGE_HEADS, "Wednesday"));
        assert!(!in_list(HEDGE_HEADS, ""));
        assert!(in_list(ABSORBED, "UM"));
        assert!(!in_list(FUNCTION_WORDS, "Tuesday"));
    }

    /// Gate 2b in isolation. The review found this guard could be reverted
    /// without failing a single test even though it is load-bearing — the
    /// corpus only ever hit it through cases gate 2a already caught.
    ///
    /// Here "I mean" is the object of a relative clause, and only the word in
    /// front of the marker says so: the repair head ("Wednesday") is a perfectly
    /// good one, and the marker carries its own comma.
    #[test]
    fn a_relative_clause_before_the_marker_is_not_a_repair() {
        // Every other gate passes on the first two: the marker carries its own
        // comma, the repair head is a good one, the window has content in it
        // and is aligned on a repeated word. Only the relative pronoun in front
        // of "I mean" says the marker is the clause's object, not an aside.
        for input in [
            "the meeting that I mean, the Wednesday one",
            "the file which I mean, the final draft",
            "the day that I mean, Wednesday afternoon works.",
            "the thing which I mean, Wednesday, is the deadline.",
            "the person who I mean, Wednesday's speaker, is late.",
        ] {
            assert_eq!(&go(input), input, "rewrote a relative clause: {input:?}");
        }
    }

    /// The quotation guards in isolation, for the same reason. Both were
    /// reachable only through inputs the corpus did not contain.
    ///
    /// Without the marker-inside-quotation check the first one retracts the
    /// attribution that introduced the quote; without the backward barrier the
    /// second one reaches out of the quotation to do it.
    #[test]
    fn a_quotation_is_never_retracted_through() {
        assert_eq!(
            go("She said, \"no wait, I changed my mind.\""),
            "She said, \"no wait, I changed my mind.\""
        );
        // The barrier is inclusive — the quoted phrase may go, the attribution
        // that introduced it may not. No comma after "said", so the phrase
        // boundary is not what stops the walk here; the opening quote is.
        let got = go("She said \"meet Tuesday, I mean, Wednesday afternoon works.\"");
        assert_eq!(got, "She said \"Wednesday afternoon works.\"");
        assert!(
            got.starts_with("She said \""),
            "reached out of the quotation"
        );
    }

    /// Every input the adversarial review destroyed, kept verbatim. Each block
    /// is a finding; if one of these starts rewriting again, the guard that
    /// stopped it has been removed.
    #[test]
    fn the_adversarial_review_corpus() {
        for input in [
            // 1 — a fronted adverbial's comma satisfied gate 1 for `no wait`
            // and `scratch that`. 467 of 475 of this shape were destroyed.
            "Order ahead, no wait times.",
            "Come early, no wait guaranteed.",
            "If you are itchy, scratch that rash.",
            "Take a note, scratch that idea entirely.",
            // 2 — an unlisted intensifier ate the subject
            "I love it, I mean absolutely love it",
            "He runs daily, I mean every single day.",
            "It costs money, I mean serious money.",
            "The team is small, I mean four people.",
            // 3 — the length-guessed window reached for the phrase boundary
            "For the record, I mean Wednesday works.",
            "In short, I mean Wednesday.",
            "By the way, I mean Wednesday.",
            // 4 — an abbreviation is not a sentence boundary. The first three
            // are the review's; the last two isolate the two halves of the
            // fix, because in the review's cases both halves fire at once.
            "Mr. Patel, I mean Dr. Patel, will call you.",
            "Ship by Jan. 5, I mean Jan. 6.",
            "See Fig. 3, I mean Fig. 4, for the chart.",
            // …abbreviation only in the repair, nothing before the window
            "I said Tuesday, I mean Dr. Patel",
            // …abbreviation only before the window, nothing in the repair
            "Call Dr. Patel, I mean, call the nurse",
            // 5 — correcting a pre-nominal modifier strands it
            "I'll bring the red one, no wait, blue",
            "Book the morning flight, I mean, evening",
            "Order the house salad, no wait, caesar",
            // 7 — apposition, where "I mean" identifies rather than replaces
            "The waiting room, I mean the lobby, is on the left.",
            "Ask Sarah, I mean the new manager, about it.",
            "the wait, I mean the queue, was 40 minutes",
        ] {
            assert_eq!(&go(input), input, "review case rewritten: {input:?}");
        }
    }

    /// What [`single_comma_restraint`] costs, stated in full rather than
    /// buried. These are real marked corrections that no longer resolve: all
    /// of the form "X noun, I mean Y noun" with no word shared between the two
    /// halves and no number to align on, which is exactly the shape that is
    /// indistinguishable from an intensifying "I mean".
    ///
    /// Adding the second comma resolves every one of them, which is the point:
    /// the information was never in the words, only in the punctuation.
    #[test]
    fn the_single_comma_restraint_costs_these() {
        for (single, both, resolved) in [
            (
                "our target is Q4, I mean end of year",
                "our target is Q4, I mean, end of year",
                "our target is end of year",
            ),
            (
                "the deadline was Friday, I mean start of next week",
                "the deadline was Friday, I mean, start of next week",
                "the deadline was start of next week",
            ),
            (
                "we ship on Monday, I mean next Tuesday",
                "we ship on Monday, I mean, next Tuesday",
                "we ship next Tuesday",
            ),
        ] {
            assert_eq!(&go(single), single, "single-comma form should stand down");
            assert_eq!(go(both), resolved, "both-comma form should resolve");
        }
    }

    /// …and what it deliberately does **not** cost: the issue's headline
    /// example and every other single-word repair still resolve on one comma.
    /// This is the whole reason `I mean` did not get the both-sides gate.
    #[test]
    fn the_single_comma_restraint_keeps_the_headline_example() {
        assert_eq!(go("I said Tuesday, I mean Wednesday"), "I said Wednesday");
        assert_eq!(go("call Bob, I mean Rob"), "call Rob");
        assert_eq!(go("it was Tuesday, I mean Wednesday"), "it was Wednesday");
        // …and multi-word repairs that can point at what they replace
        assert_eq!(
            go("that's 15 dollars, I mean 50 dollars"),
            "that's 50 dollars"
        );
        assert_eq!(
            go("we need three chairs, I mean four chairs, by Friday"),
            "we need four chairs, by Friday"
        );
    }

    /// Parakeet writes numbers as words (SCOPE.md), so a numeric repair from
    /// the shipping Linux default never contains a digit. Alignment has to
    /// work on both spellings or it is dead on that model.
    #[test]
    fn numbers_align_when_spelled_out_as_words() {
        assert_eq!(
            go("bring three chairs, no wait, four chairs"),
            "bring four chairs"
        );
        assert_eq!(go("it costs twenty, no wait, thirty"), "it costs thirty");
        assert_eq!(go("I have one dog, I mean, two cats"), "I have two cats");
        // and the digit spelling still works
        assert_eq!(go("bring 3 chairs, no wait, 4 chairs"), "bring 4 chairs");
    }

    #[test]
    fn contains_ci_matches_without_allocating() {
        assert!(contains_ci("I MEANT it", "mean"));
        assert!(contains_ci("mean", "mean"));
        assert!(!contains_ci("mea", "mean"));
        assert!(!contains_ci("", "mean"));
        assert!(!contains_ci("Правда", "mean"));
    }
}
