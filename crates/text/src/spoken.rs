//! Spoken formatting — "new paragraph", "bullet point" and friends become the
//! characters the user meant rather than the words they said (#45).
//!
//! Two independently switchable sets, because they are not the same bet:
//!
//! * **Structural** (`structural`, on whenever the transform is): "new
//!   paragraph", "new line", "bullet point", "numbered list". Parakeet and
//!   Moonshine cannot emit a line break or a list marker at all — there is no
//!   way to dictate structure today — so this fights nothing and is pure gain.
//! * **Punctuation** (`punctuation`, off): "comma", "period", "full stop",
//!   "question mark", "open quote", "close quote", "colon", "dash". Both models
//!   already punctuate and capitalize natively. Switching this on replaces the
//!   model's judgement with a literal word-for-character rule, so it is off by
//!   default and labelled redundant everywhere the user can read it.
//!
//! The whole difficulty is telling a *command* from the same words used as
//! *prose*: "the comma splice problem" must keep its comma as a word, and
//! "a period of time" must not sprout a full stop. Five guards decide, and all
//! five are biased the same way — **when in doubt, leave the text alone**. A
//! missed command is a papercut the user fixes by typing one character; a
//! command fired inside prose is a bug report.
//!
//! 1. **Quoted.** A phrase wrapped in quotes is being talked *about*:
//!    `he wrote "new paragraph" in the margin`.
//! 2. **The word before.** A determiner, possessive, quantifier or preposition
//!    immediately in front makes the phrase a noun: "a period", "the new line",
//!    "of dash". Punctuation in the gap lifts this one — "…John's. New
//!    paragraph." is a command, not a genitive.
//! 3. **A genitive before.** "the article's period" is prose.
//! 4. **The word after.** A preposition, copula, auxiliary, relative or
//!    conjunction right after means the phrase was the subject of a clause:
//!    "full stop *is* a British idiom", "bullet point *about* pricing", "dash
//!    *to* the shop". Punctuation does **not** lift this one: blocking costs a
//!    papercut, firing costs prose.
//! 5. **A determiner within three words, and the phrase ends a clause.** Catches
//!    the noun-phrase-at-a-clause-boundary shape that guard 2 misses: "I added
//!    an important bullet point." The clause-end half is what keeps real lists
//!    working — "buy **the** milk bullet point walk the dog" still fires,
//!    because content follows the command rather than a full stop.
//!
//! Per-command word lists carry the compound nouns the generic lists cannot
//! know about: "grace period", "Oxford comma", "colon cancer", "line manager",
//! "mad dash".

use serde::{Deserialize, Serialize};

use crate::Transform;

// ------------------------------------------------------------------ config --

/// Which spoken commands to honour. Both sub-options are independent, and both
/// are ignored unless [`SpokenConfig::enabled`] is set.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SpokenConfig {
    /// Master switch. Off by default, like every transform in this crate.
    pub enabled: bool,

    /// "new paragraph", "new line", "bullet point", "numbered list".
    ///
    /// **On** whenever the transform is enabled: no shipping model can produce
    /// a line break or a list marker, so nothing is being overridden.
    pub structural: bool,

    /// "comma", "period", "full stop", "question mark", "open quote",
    /// "close quote", "colon", "dash".
    ///
    /// **Off** by default, and *redundant with the model*: Parakeet and
    /// Moonshine both punctuate natively, so turning this on means overriding
    /// their judgement with a literal word-for-character rule. Settings (#49)
    /// must label it that way where the user can read it.
    pub punctuation: bool,
}

impl Default for SpokenConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            structural: true,
            punctuation: false,
        }
    }
}

// ----------------------------------------------------------------- command --

/// What a matched phrase turns into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `\n\n`, and the end of any numbered run.
    Paragraph,
    /// `\n`.
    Line,
    /// `- ` on a line of its own.
    Bullet,
    /// `1. ` on a line of its own, counting up within the run.
    Numbered,
    /// Punctuation that hugs the word before it: `,` `.` `?` `:`.
    Tight(&'static str),
    /// An opening quote: hugs the word after it.
    Open(&'static str),
    /// A closing quote: hugs the word before it, but never eats the sentence
    /// punctuation already sitting there.
    Close(&'static str),
    /// A spaced dash, the Dragon convention: "dash" is ` - `, not `-`.
    Dash,
}

struct Command {
    /// One or two words, lowercase. Matched case-insensitively; no command's
    /// words are a prefix of another's, so table order does not matter.
    words: &'static [&'static str],
    kind: Kind,
    /// True for the structural set, false for the punctuation set.
    structural: bool,
    /// Words that, immediately in front, make this phrase a noun.
    before: &'static [&'static str],
    /// Words that, immediately after, make this phrase a noun or a verb.
    after: &'static [&'static str],
}

/// Every token this transform knows. Deliberately closed: the issue's list and
/// two spellings of a full stop, nothing invented.
const COMMANDS: &[Command] = &[
    Command {
        words: &["new", "paragraph"],
        kind: Kind::Paragraph,
        structural: true,
        before: &["brand"],
        after: &[
            "break",
            "breaks",
            "symbol",
            "symbols",
            "sign",
            "signs",
            "mark",
            "marks",
            "style",
            "styles",
            "indent",
            "indents",
            "spacing",
            "tag",
            "tags",
            "format",
            "formatting",
            "here",
            "there",
            "instead",
            "itself",
        ],
    },
    Command {
        words: &["new", "line"],
        kind: Kind::Line,
        structural: true,
        before: &["brand"],
        after: &[
            "manager",
            "managers",
            "management",
            "cook",
            "cooks",
            "cinema",
            "break",
            "breaks",
            "item",
            "items",
            "spacing",
            "indent",
            "character",
            "characters",
            "ending",
            "endings",
            "judge",
            "judges",
            "dance",
            "up",
            "here",
            "there",
            "instead",
            "itself",
        ],
    },
    Command {
        words: &["bullet", "point"],
        kind: Kind::Bullet,
        structural: true,
        before: &["silver", "magic", "stray", "single", "key", "main"],
        after: &[
            "about",
            "regarding",
            "saying",
            "said",
            "says",
            "summary",
            "summaries",
            "format",
            "formatting",
            "formats",
            "style",
            "styles",
            "list",
            "lists",
            "symbol",
            "symbols",
            "character",
            "characters",
            "here",
            "there",
            "instead",
            "itself",
            "above",
            "below",
        ],
    },
    Command {
        words: &["numbered", "list"],
        kind: Kind::Numbered,
        structural: true,
        before: &[],
        after: &[
            "format",
            "formatting",
            "style",
            "styles",
            "item",
            "items",
            "version",
            "here",
            "there",
            "instead",
            "itself",
            "above",
            "below",
        ],
    },
    Command {
        words: &["comma"],
        kind: Kind::Tight(","),
        structural: false,
        before: &["oxford", "serial", "inverted", "decimal", "floating"],
        after: &[
            "splice",
            "splices",
            "splicing",
            "separated",
            "separator",
            "separators",
            "delimited",
            "delimiter",
            "delimiters",
            "usage",
            "rule",
            "rules",
            "here",
            "there",
        ],
    },
    Command {
        words: &["period"],
        kind: Kind::Tight("."),
        structural: false,
        before: &[
            "grace",
            "trial",
            "time",
            "rest",
            "waiting",
            "cooling",
            "notice",
            "probation",
            "transition",
            "incubation",
            "refund",
            "warranty",
            "fiscal",
            "historical",
            "sales",
            "tax",
            "holiday",
            "school",
            "class",
            "lunch",
            "quiet",
            "awkward",
            "dry",
            "long",
            "short",
            "brief",
            "extended",
            "limited",
            "recovery",
            "reporting",
            "accounting",
            "billing",
            "victorian",
            "medieval",
            "colonial",
            "classical",
            "modern",
            "renaissance",
        ],
        after: &[
            "piece",
            "pieces",
            "drama",
            "dramas",
            "costume",
            "costumes",
            "furniture",
            "film",
            "films",
            "romance",
            "romances",
            "style",
            "styles",
            "ending",
            "tracker",
            "trackers",
            "cramps",
            "pain",
            "poverty",
            "product",
            "products",
            "blood",
            "correct",
            "appropriate",
            "instrument",
            "instruments",
            "home",
            "house",
        ],
    },
    Command {
        words: &["full", "stop"],
        kind: Kind::Tight("."),
        structural: false,
        before: &["dead", "abrupt", "sudden", "screeching", "grinding"],
        after: &[
            "sign",
            "signs",
            "ahead",
            "here",
            "there",
            "idiom",
            "idioms",
            "expression",
            "expressions",
            "phrase",
            "phrases",
            "british",
            "american",
        ],
    },
    Command {
        words: &["question", "mark"],
        kind: Kind::Tight("?"),
        structural: false,
        before: &["big", "huge", "giant", "little", "small", "the"],
        after: &[
            "over", "above", "next", "hovering", "icon", "icons", "button", "buttons", "key",
            "keys", "symbol", "symbols", "here", "there",
        ],
    },
    Command {
        words: &["colon"],
        kind: Kind::Tight(":"),
        structural: false,
        before: &[
            "semi",
            "irritable",
            "ascending",
            "descending",
            "transverse",
            "sigmoid",
            "spastic",
            "large",
        ],
        after: &[
            "cancer",
            "cancers",
            "surgery",
            "surgeon",
            "surgeons",
            "cleanse",
            "cleansing",
            "polyp",
            "polyps",
            "health",
            "screening",
            "screenings",
            "cell",
            "cells",
            "tissue",
            "irrigation",
        ],
    },
    Command {
        words: &["open", "quote"],
        kind: Kind::Open("\""),
        structural: false,
        before: &["wide"],
        after: &[
            "mark",
            "marks",
            "character",
            "characters",
            "symbol",
            "symbols",
            "here",
            "there",
        ],
    },
    Command {
        words: &["close", "quote"],
        kind: Kind::Close("\""),
        structural: false,
        before: &[],
        after: &[
            "mark",
            "marks",
            "character",
            "characters",
            "symbol",
            "symbols",
            "here",
            "there",
        ],
    },
    Command {
        words: &["dash"],
        kind: Kind::Dash,
        structural: false,
        before: &[
            "mad", "quick", "wild", "headlong", "final", "last", "hundred", "sprint", "em", "en",
            "cam", "dot",
        ],
        after: &[
            "off", "out", "down", "up", "away", "back", "home", "board", "boards", "cam", "cams",
            "line", "lines", "forward", "past", "ahead",
        ],
    },
];

/// Words that make the phrase after them a noun: articles, demonstratives,
/// possessives, quantifiers, ordinals and prepositions. Checked only against
/// the word *immediately* in front, with no punctuation in the gap.
///
/// "all", "both", "most", "many", "few" and "several" are deliberately absent:
/// they take a plural noun ("all periods"), which never matches a command
/// anyway, and "that is **all** period" is a command people really say.
#[rustfmt::skip]
const BEFORE: &[&str] = &[
    // determiners and demonstratives
    "a", "an", "the", "this", "that", "these", "those", "another", "other", "such",
    "no", "any", "some", "each", "every",
    // possessives
    "my", "your", "his", "her", "its", "our", "their", "whose",
    // counts and ordinals
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    "first", "second", "third", "fourth", "fifth", "next", "last", "previous", "final",
    "single", "double", "triple", "whole", "entire", "same", "own", "only",
    "extra", "additional",
    // prepositions
    "of", "in", "on", "at", "to", "for", "with", "without", "from", "by", "into",
    "onto", "about", "after", "before", "during", "per", "via", "than", "between",
    "through", "upon", "against", "around", "across", "near", "over", "under",
    "within", "along", "like", "as",
];

/// Words that cannot start dictated content but very often continue a noun or
/// verb phrase. Checked against the next word whether or not punctuation
/// separates it: blocking here costs a papercut, firing costs prose.
///
/// Determiners and pronouns are deliberately absent — "new paragraph *the*
/// next thing" and "bullet point *we* should ship" are exactly how people
/// dictate.
#[rustfmt::skip]
const AFTER: &[&str] = &[
    // prepositions and particles
    "of", "in", "on", "at", "to", "for", "with", "without", "from", "by", "into",
    "onto", "about", "after", "before", "during", "per", "via", "than", "between",
    "through", "upon", "toward", "towards", "against", "among", "amongst", "around",
    "across", "behind", "beneath", "below", "beside", "beyond", "near", "within", "along",
    // copulas and auxiliaries
    "is", "isn't", "was", "wasn't", "are", "aren't", "were", "weren't", "be", "been",
    "being", "am", "has", "hasn't", "have", "haven't", "had", "hadn't", "will", "won't",
    "would", "shall", "should", "can", "can't", "could", "may", "might", "must",
    "do", "does", "doesn't", "did", "didn't",
    // relatives and conjunctions
    "that", "which", "who", "whom", "whose", "when", "where", "why", "and", "or",
    "but", "nor", "because", "although", "though",
];

/// The subset of [`BEFORE`] that opens a noun phrase, used by the three-word
/// look-back. Adjectives and prepositions are left out on purpose: they are
/// too common in ordinary dictation to search backwards for.
///
/// So are the demonstratives, which are pronouns just as often as they are
/// determiners — "**that** is all period" is a command, and searching three
/// words back for "that" would have blocked it.
const DETERMINERS: &[&str] = &[
    "a", "an", "the", "my", "your", "his", "her", "its", "our", "their", "no", "any", "some",
    "each", "every", "another", "other", "such",
];

/// How far back to look for a determiner (guard 5).
const LOOKBACK: usize = 3;

// --------------------------------------------------------------- transform --

/// Turns spoken punctuation and layout commands into characters.
pub struct Spoken {
    cfg: SpokenConfig,
}

impl Spoken {
    pub fn new(cfg: SpokenConfig) -> Self {
        Self { cfg }
    }

    /// Is this command's set switched on?
    fn active(&self, cmd: &Command) -> bool {
        if cmd.structural {
            self.cfg.structural
        } else {
            self.cfg.punctuation
        }
    }

    /// The command starting at `word`, if any, with the byte index just past
    /// its last word.
    fn command_at(&self, text: &str, word: &Word) -> Option<(&'static Command, usize)> {
        let head = &text[word.start..word.end];
        COMMANDS.iter().find_map(|cmd| {
            if !self.active(cmd) || !head.eq_ignore_ascii_case(cmd.words[0]) {
                return None;
            }
            if cmd.words.len() == 1 {
                return Some((cmd, word.end));
            }
            let second = next_word(text, word.end)?;
            // "new, paragraph" is not a phrase: only whitespace may separate
            // the words of a command.
            if second.punct_gap
                || !text[second.start..second.end].eq_ignore_ascii_case(cmd.words[1])
            {
                return None;
            }
            Some((cmd, second.end))
        })
    }
}

impl Transform for Spoken {
    fn name(&self) -> &'static str {
        "spoken"
    }

    fn apply(&self, text: &str) -> String {
        if !self.cfg.enabled || (!self.cfg.structural && !self.cfg.punctuation) {
            return text.to_string();
        }

        let mut out = String::with_capacity(text.len());
        // Everything before `copied` is already in `out`; the gap is copied as
        // one slice when a command fires, so untouched text costs one memcpy.
        let mut copied = 0usize;
        let mut cursor = 0usize;
        // Whitespace we emitted ourselves. A later command may not trim it
        // away, or "new paragraph bullet point" would swallow its own break.
        let mut protected = 0usize;
        // List state: "numbered list" switches the run to numbers, so a user
        // who says it once and then "bullet point" three times still gets
        // 1. 2. 3. 4. "new paragraph" ends the run.
        let mut numbered = false;
        let mut item = 1usize;

        while let Some(word) = next_word(text, cursor) {
            cursor = word.end;
            let Some((cmd, end)) = self.command_at(text, &word) else {
                continue;
            };
            if !is_command(text, word.start, end, cmd) {
                // Only the first word is consumed, so "the new line manager"
                // still gets to examine "line" as a fresh candidate.
                continue;
            }

            out.push_str(&text[copied..word.start]);
            trim_end_ws(&mut out, protected);
            cut_one(
                &mut out,
                match cmd.kind {
                    // Explicit punctuation supersedes whatever the model put
                    // in that exact slot.
                    Kind::Tight(_) => &[',', '.', ';', ':', '!', '?'],
                    // Everything else only sheds the comma the model used to
                    // set the spoken command off from the sentence.
                    _ => &[','],
                },
            );

            let mut space_after = false;
            match cmd.kind {
                Kind::Paragraph => {
                    out.push_str("\n\n");
                    numbered = false;
                    item = 1;
                }
                Kind::Line => out.push('\n'),
                Kind::Bullet | Kind::Numbered => {
                    if cmd.kind == Kind::Numbered {
                        numbered = true;
                    }
                    // The marker's newline is a separator, not a break: no
                    // blank line when the output is already at column zero.
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                    if numbered {
                        out.push_str(&item.to_string());
                        out.push_str(". ");
                        item += 1;
                    } else {
                        out.push_str("- ");
                    }
                }
                Kind::Tight(s) | Kind::Close(s) => {
                    out.push_str(s);
                    space_after = true;
                }
                Kind::Open(s) => {
                    if needs_space(&out) {
                        out.push(' ');
                    }
                    out.push_str(s);
                }
                Kind::Dash => {
                    if needs_space(&out) {
                        out.push(' ');
                    }
                    out.push_str("- ");
                }
            }
            protected = out.len();

            cursor = skip_after(
                text,
                end,
                match cmd.kind {
                    Kind::Tight(_) => &[',', '.', ';', ':', '!', '?'],
                    Kind::Paragraph | Kind::Line | Kind::Bullet | Kind::Numbered => {
                        &[',', '.', ';']
                    }
                    _ => &[','],
                },
            );
            copied = cursor;
            if space_after {
                if let Some(c) = text[cursor..].chars().next() {
                    if !hugs_left(c) {
                        out.push(' ');
                    }
                }
            }
        }

        out.push_str(&text[copied..]);
        out
    }

    /// Not prefix-stable, and no multi-word trigger ever can be. A streaming
    /// pass that has heard `"buy milk bullet"` has already typed those 15
    /// characters; the finished `"buy milk bullet point walk the dog"`
    /// polishes to `"buy milk\n- walk the dog"`, which does not start with
    /// them. `prefix_violation` finds it one character earlier still — the
    /// trailing space of `"buy milk "` is not a prefix of `"buy milk\n- …"` —
    /// because a command replaces the whitespace around it too.
    fn prefix_stable(&self) -> bool {
        false
    }

    /// One thing a user can get wrong here: switch the transform on and both
    /// command sets off, which silently does nothing at all.
    fn validate(&self) -> Vec<String> {
        if self.cfg.enabled && !self.cfg.structural && !self.cfg.punctuation {
            return vec!["spoken formatting is on but both command sets are off — \
                 enable structural commands, punctuation commands, or neither"
                .into()];
        }
        Vec::new()
    }
}

// ----------------------------------------------------------------- lexicon --

/// A word and whether break punctuation sits in the gap between it and the
/// position it was found from.
struct Word {
    start: usize,
    end: usize,
    punct_gap: bool,
}

/// Punctuation that ends a word. `'` and `’` are excluded so "don't" and
/// "John’s" stay whole; the Unicode arm covers curly quotes and dashes so a
/// typographic quotation is seen as a quotation.
fn is_break_punct(c: char) -> bool {
    (c.is_ascii_punctuation() && c != '\'')
        || matches!(
            c,
            '\u{2010}'
                | '\u{2011}'
                | '\u{2012}'
                | '\u{2013}'
                | '\u{2014}'
                | '\u{2015}'
                | '\u{2018}'
                | '\u{201A}'
                | '\u{201B}'
                | '\u{201C}'
                | '\u{201D}'
                | '\u{201E}'
                | '\u{201F}'
                | '\u{2026}'
                | '\u{00AB}'
                | '\u{00BB}'
                | '\u{2039}'
                | '\u{203A}'
        )
}

/// Anything that is not whitespace and not punctuation — deliberately
/// including CJK, emoji, combining marks and zero-width characters, none of
/// which can match a command but all of which must not split a word in half.
fn is_word_char(c: char) -> bool {
    !c.is_whitespace() && !is_break_punct(c)
}

/// The first word at or after `from`.
fn next_word(text: &str, from: usize) -> Option<Word> {
    let mut punct_gap = false;
    let mut start = None;
    for (off, c) in text[from..].char_indices() {
        if is_word_char(c) {
            start = Some(from + off);
            break;
        }
        if !c.is_whitespace() {
            punct_gap = true;
        }
    }
    let start = start?;
    let mut end = text.len();
    for (off, c) in text[start..].char_indices() {
        if !is_word_char(c) {
            end = start + off;
            break;
        }
    }
    Some(Word {
        start,
        end,
        punct_gap,
    })
}

/// The last word before `at`.
fn prev_word(text: &str, at: usize) -> Option<Word> {
    let mut punct_gap = false;
    let mut end = None;
    for (off, c) in text[..at].char_indices().rev() {
        if is_word_char(c) {
            end = Some(off + c.len_utf8());
            break;
        }
        if !c.is_whitespace() {
            punct_gap = true;
        }
    }
    let end = end?;
    let mut start = 0;
    for (off, c) in text[..end].char_indices().rev() {
        if !is_word_char(c) {
            start = off + c.len_utf8();
            break;
        }
    }
    Some(Word {
        start,
        end,
        punct_gap,
    })
}

// ------------------------------------------------------------------ guards --

fn is_command(text: &str, start: usize, end: usize, cmd: &Command) -> bool {
    // 1. talked about rather than spoken
    if quoted(text, start, end) {
        return false;
    }
    // 2 and 3. the word in front makes it a noun
    if let Some(p) = prev_word(text, start) {
        let w = &text[p.start..p.end];
        if !p.punct_gap && (listed(w, BEFORE) || listed(w, cmd.before) || genitive(w)) {
            return false;
        }
    }
    // 4. the word after continues a phrase
    if let Some(n) = next_word(text, end) {
        let w = &text[n.start..n.end];
        if listed(w, AFTER) || listed(w, cmd.after) {
            return false;
        }
    }
    // 5. a noun phrase sitting at a clause boundary
    if ends_clause(text, end) && determiner_within(text, start, LOOKBACK) {
        return false;
    }
    true
}

fn listed(word: &str, list: &[&str]) -> bool {
    list.iter().any(|w| word.eq_ignore_ascii_case(w))
}

/// "the article's period" — a genitive in front is always a noun phrase.
fn genitive(word: &str) -> bool {
    let mut back = word.chars().rev();
    matches!(back.next(), Some('s' | 'S')) && matches!(back.next(), Some('\'' | '\u{2019}'))
}

/// Directly wrapped in quotes, as in `he wrote "new paragraph" in the margin`.
///
/// Straight single quotes need no case here: `'` is a word character, so
/// `'new paragraph'` never matches the phrase in the first place.
fn quoted(text: &str, start: usize, end: usize) -> bool {
    let opens = matches!(
        text[..start].chars().next_back(),
        Some('"' | '\u{201C}' | '\u{2018}' | '\u{00AB}' | '\u{2039}')
    );
    let closes = matches!(
        text[end..].chars().next(),
        Some('"' | '\u{201D}' | '\u{2019}' | '\u{00BB}' | '\u{203A}')
    );
    opens && closes
}

/// Is a determiner within `depth` words in front, without crossing punctuation?
fn determiner_within(text: &str, start: usize, depth: usize) -> bool {
    let mut at = start;
    for _ in 0..depth {
        let Some(w) = prev_word(text, at) else {
            return false;
        };
        if w.punct_gap {
            return false;
        }
        if listed(&text[w.start..w.end], DETERMINERS) {
            return true;
        }
        at = w.start;
    }
    false
}

/// Does the phrase end a clause — punctuation next, or nothing at all?
fn ends_clause(text: &str, at: usize) -> bool {
    match text[at..].chars().find(|c| !c.is_whitespace()) {
        None => true,
        Some(c) => matches!(
            c,
            ',' | '.'
                | ';'
                | ':'
                | '!'
                | '?'
                | ')'
                | ']'
                | '}'
                | '"'
                | '\''
                | '\u{201D}'
                | '\u{2019}'
        ),
    }
}

// -------------------------------------------------------------- edit helpers --

/// Trim trailing whitespace, but never past `protected` — text this transform
/// emitted itself is not the model's spacing to clean up.
fn trim_end_ws(out: &mut String, protected: usize) {
    let keep = out.trim_end().len().max(protected.min(out.len()));
    out.truncate(keep);
}

/// Drop one trailing character if it is in `set`, then re-trim whitespace.
/// This is how "buy milk, bullet point" loses the comma the model added to set
/// the spoken command off from the sentence.
fn cut_one(out: &mut String, set: &[char]) {
    let Some(c) = out.chars().next_back() else {
        return;
    };
    if !set.contains(&c) {
        return;
    }
    let keep = out.len() - c.len_utf8();
    out.truncate(keep);
    let keep = out.trim_end().len();
    out.truncate(keep);
}

/// Skip whitespace after a command, plus at most one punctuation character the
/// model attached to the command itself ("new paragraph. Then we…").
fn skip_after(text: &str, from: usize, drop: &[char]) -> usize {
    let mut i = from + ws_len(&text[from..]);
    if let Some(c) = text[i..].chars().next() {
        if drop.contains(&c) {
            i += c.len_utf8();
            i += ws_len(&text[i..]);
        }
    }
    i
}

fn ws_len(s: &str) -> usize {
    s.find(|c: char| !c.is_whitespace()).unwrap_or(s.len())
}

/// A space is wanted before an opening quote or a dash unless there is one
/// already, or nothing at all, in front of it.
fn needs_space(out: &str) -> bool {
    match out.chars().next_back() {
        None => false,
        Some(c) => !c.is_whitespace() && !matches!(c, '(' | '[' | '{' | '\u{201C}' | '\u{00AB}'),
    }
}

/// Characters that must not have a space inserted before them.
fn hugs_left(c: char) -> bool {
    matches!(
        c,
        ',' | '.'
            | ';'
            | ':'
            | '!'
            | '?'
            | ')'
            | ']'
            | '}'
            | '%'
            | '"'
            | '\''
            | '\u{201D}'
            | '\u{2019}'
            | '\u{00BB}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{prefix_violation, torture_inputs, truncate};

    /// Everything this transform can do, for the corpus tests: both command
    /// sets on, so every token is live at once.
    fn all() -> Spoken {
        Spoken::new(SpokenConfig {
            enabled: true,
            structural: true,
            punctuation: true,
        })
    }

    /// What the user gets by switching the transform on and nothing else.
    fn shipped_default() -> Spoken {
        Spoken::new(SpokenConfig {
            enabled: true,
            ..SpokenConfig::default()
        })
    }

    fn apply(s: &Spoken, text: &str) -> String {
        s.apply(text)
    }

    // ---- config ----------------------------------------------------------

    #[test]
    fn disabled_by_default() {
        assert!(!SpokenConfig::default().enabled);
    }

    /// The two sub-option defaults the issue argues for: structure is pure
    /// gain because no model can produce it, punctuation would be fighting a
    /// model that already punctuates.
    #[test]
    fn structural_defaults_on_and_punctuation_defaults_off() {
        let cfg = SpokenConfig::default();
        assert!(cfg.structural);
        assert!(!cfg.punctuation);
    }

    #[test]
    fn a_config_with_only_enabled_set_gets_the_documented_defaults() {
        let cfg: SpokenConfig = toml::from_str("enabled = true\n").unwrap();
        assert!(cfg.enabled && cfg.structural && !cfg.punctuation);
    }

    #[test]
    fn config_round_trips_through_toml() {
        let cfg = SpokenConfig {
            enabled: true,
            structural: false,
            punctuation: true,
        };
        let back: SpokenConfig = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
        assert!(back.enabled && !back.structural && back.punctuation);
    }

    /// The config key is `[polish.spoken]` and the sub-options are named in
    /// Settings (#49) and in the user's `config.toml`. Renaming one silently
    /// resets that user's choice to the default, so pin the spelling.
    #[test]
    fn config_keys_are_stable() {
        let text = toml::to_string(&SpokenConfig::default()).unwrap();
        for key in ["enabled", "structural", "punctuation"] {
            assert!(text.contains(key), "{key} missing from {text:?}");
        }
    }

    // ---- disabled --------------------------------------------------------

    /// Disabled must be byte-identical, and so must "enabled with both sets
    /// off" — the config `validate` warns about.
    #[test]
    fn disabled_is_byte_identical() {
        let off = [
            SpokenConfig::default(),
            SpokenConfig {
                enabled: false,
                structural: true,
                punctuation: true,
            },
            SpokenConfig {
                enabled: true,
                structural: false,
                punctuation: false,
            },
        ];
        let corpus: Vec<String> = torture_inputs()
            .into_iter()
            .chain(COMMANDS_CORPUS.iter().map(|(i, _)| (*i).to_string()))
            .chain(PROSE_CORPUS.iter().map(|p| (*p).to_string()))
            .collect();
        for cfg in off {
            let s = Spoken::new(cfg.clone());
            for input in &corpus {
                assert_eq!(
                    s.apply(input),
                    *input,
                    "{cfg:?} changed {:?}",
                    truncate(input)
                );
            }
        }
    }

    #[test]
    fn one_set_off_leaves_the_other_set_alone() {
        let structural_only = shipped_default();
        assert_eq!(
            apply(&structural_only, "hello comma world new line bye"),
            "hello comma world\nbye"
        );
        let punctuation_only = Spoken::new(SpokenConfig {
            enabled: true,
            structural: false,
            punctuation: true,
        });
        assert_eq!(
            apply(&punctuation_only, "hello comma world new line bye"),
            "hello, world new line bye"
        );
    }

    // ---- the corpus ------------------------------------------------------

    /// Every supported token, used as a command. Both sets on.
    const COMMANDS_CORPUS: &[(&str, &str)] = &[
        // structural
        (
            "that is the draft new paragraph now the second half",
            "that is the draft\n\nnow the second half",
        ),
        ("first line new line second line", "first line\nsecond line"),
        (
            "bullet point buy milk bullet point walk the dog",
            "- buy milk\n- walk the dog",
        ),
        (
            "numbered list buy milk numbered list walk the dog",
            "1. buy milk\n2. walk the dog",
        ),
        // punctuation
        ("hello comma world", "hello, world"),
        ("that is all period", "that is all."),
        ("that is all full stop", "that is all."),
        ("are you sure question mark", "are you sure?"),
        (
            "he said open quote ship it close quote yesterday",
            "he said \"ship it\" yesterday",
        ),
        (
            "the plan is as follows colon buy milk",
            "the plan is as follows: buy milk",
        ),
        (
            "wait dash actually never mind",
            "wait - actually never mind",
        ),
    ];

    /// The same words used as prose. Every one of these must survive byte for
    /// byte with **both** command sets on — the strictest setting there is.
    const PROSE_CORPUS: &[&str] = &[
        // the issue's own acceptance case
        "the comma splice problem",
        "the comma splice problem is easy to miss",
        // the brief's list
        "a period of time",
        "we waited a period of time",
        "the new line manager starts on Monday",
        "full stop is a British idiom",
        "add a bullet point about the pricing",
        "I had to dash to the shop",
        "he wrote \"new paragraph\" in the margin",
        "she typed “new paragraph” and moved on",
        // determiners and possessives in front
        "I need a new paragraph here",
        "put it on a new line",
        "make a numbered list of the tasks",
        "use a comma there",
        "the question mark was missing",
        "she used an open quote and forgot the other one",
        "the close quote mark is missing",
        "a colon separates the two clauses",
        "type the dash key",
        "the article's period was wrong",
        // compound nouns the per-command lists carry
        "an Oxford comma is standard",
        "she was in a grace period",
        "colon cancer screening saves lives",
        "the mad dash to the exit",
        "the car came to a dead full stop",
        // the phrase ends a clause after a determiner (guard 5)
        "I added an important bullet point.",
        "that was a really long period.",
        // plurals never match at all
        "bullet points are fine",
        "commas and periods",
        "new lines are cheap",
        "numbered lists work well",
        // the token as a verb
        "I will dash off a note",
    ];

    /// The most important test in this module: both directions, every token.
    #[test]
    fn command_versus_prose() {
        let s = all();
        for (input, want) in COMMANDS_CORPUS {
            assert_eq!(&apply(&s, input), want, "command case {input:?}");
        }
        for prose in PROSE_CORPUS {
            assert_eq!(&apply(&s, prose), prose, "prose case {prose:?}");
        }
    }

    /// Every token appears in the command corpus, so adding one without
    /// showing it firing is a test failure rather than an oversight.
    #[test]
    fn every_token_is_exercised_as_a_command() {
        for cmd in COMMANDS {
            let phrase = cmd.words.join(" ");
            assert!(
                COMMANDS_CORPUS
                    .iter()
                    .any(|(i, _)| i.to_lowercase().contains(&phrase)),
                "{phrase:?} never appears as a command in the corpus"
            );
        }
    }

    /// …and as prose. "close quote" is the one token with no natural prose
    /// use, and it rides along with "open quote".
    #[test]
    fn every_token_is_exercised_as_prose() {
        for cmd in COMMANDS {
            let phrase = cmd.words.join(" ");
            assert!(
                PROSE_CORPUS
                    .iter()
                    .any(|p| p.to_lowercase().contains(&phrase)),
                "{phrase:?} never appears as prose in the corpus"
            );
        }
    }

    // ---- the issue's acceptance criteria ---------------------------------

    /// "Dictating a three-item bulleted list produces a real list" — in both
    /// shapes the models actually emit: run together, and set off with the
    /// commas and full stops Parakeet likes to add.
    #[test]
    fn a_three_item_bulleted_list_is_a_real_list() {
        let s = shipped_default();
        assert_eq!(
            apply(
                &s,
                "bullet point buy milk bullet point walk the dog bullet point feed the cat"
            ),
            "- buy milk\n- walk the dog\n- feed the cat"
        );
        assert_eq!(
            apply(
                &s,
                "Bullet point, buy milk. Bullet point, walk the dog. Bullet point, feed the cat."
            ),
            "- buy milk.\n- walk the dog.\n- feed the cat."
        );
    }

    /// "'the comma splice problem' does not produce a comma" — with the
    /// punctuation set on, which is the only way it could.
    #[test]
    fn the_comma_splice_problem_produces_no_comma() {
        let s = all();
        for text in [
            "the comma splice problem",
            "The comma splice problem is easy to miss.",
            "Comma splices are a style question.",
        ] {
            assert_eq!(apply(&s, text), text);
        }
    }

    // ---- lists -----------------------------------------------------------

    #[test]
    fn a_numbered_list_counts_up() {
        assert_eq!(
            apply(
                &shipped_default(),
                "numbered list alpha numbered list beta numbered list gamma"
            ),
            "1. alpha\n2. beta\n3. gamma"
        );
    }

    /// Say "numbered list" once and then "bullet point" for each item — the
    /// other way people dictate a numbered list — and it still numbers.
    #[test]
    fn bullet_points_continue_a_numbered_run() {
        assert_eq!(
            apply(
                &shipped_default(),
                "numbered list alpha bullet point beta bullet point gamma"
            ),
            "1. alpha\n2. beta\n3. gamma"
        );
    }

    /// A paragraph break ends the run, so the next list starts at 1 again.
    #[test]
    fn a_paragraph_restarts_the_numbering() {
        assert_eq!(
            apply(
                &shipped_default(),
                "numbered list alpha numbered list beta new paragraph numbered list gamma"
            ),
            "1. alpha\n2. beta\n\n1. gamma"
        );
    }

    #[test]
    fn a_bullet_run_after_a_numbered_run_is_still_numbered_until_a_paragraph() {
        assert_eq!(
            apply(
                &shipped_default(),
                "numbered list alpha bullet point beta new paragraph bullet point gamma"
            ),
            "1. alpha\n2. beta\n\n- gamma"
        );
    }

    #[test]
    fn consecutive_bullets_do_not_stack_blank_lines() {
        assert_eq!(
            apply(&shipped_default(), "new paragraph bullet point alpha"),
            "\n\n- alpha"
        );
    }

    // ---- the guards, one at a time ---------------------------------------

    /// Guard 2 is lifted by punctuation in the gap, so a sentence that happens
    /// to end in a determiner-ish word does not swallow the next command.
    #[test]
    fn punctuation_in_front_lifts_the_word_before_guard() {
        let s = all();
        assert_eq!(
            apply(&s, "that was John's. New paragraph. Next."),
            "that was John's.\n\nNext."
        );
        // …but with no punctuation the genitive still blocks it
        assert_eq!(
            apply(&s, "the article's period was wrong"),
            "the article's period was wrong"
        );
    }

    /// Guard 4 is **not** lifted by punctuation: the cost of blocking is a
    /// papercut, the cost of firing is corrupted prose.
    #[test]
    fn punctuation_behind_does_not_lift_the_word_after_guard() {
        let s = all();
        for text in [
            "Bullet point. Of the many things, this one.",
            "New line, and then we ship.",
        ] {
            assert_eq!(apply(&s, text), text, "{text:?}");
        }
    }

    /// Guard 5 needs *both* halves. A determiner three words back only blocks
    /// when the phrase also ends a clause — otherwise every list whose items
    /// contain "the" would break.
    #[test]
    fn a_determiner_three_words_back_only_blocks_at_a_clause_boundary() {
        let s = all();
        // content follows: still a command, even with "the" two words back
        assert_eq!(
            apply(&s, "buy the milk bullet point walk the dog"),
            "buy the milk\n- walk the dog"
        );
        // nothing follows: a noun phrase, left alone
        assert_eq!(
            apply(&s, "I added an important bullet point."),
            "I added an important bullet point."
        );
    }

    /// Only whitespace may separate the words of a phrase.
    #[test]
    fn punctuation_inside_a_phrase_is_not_a_phrase() {
        let s = all();
        for text in ["hello new, paragraph world", "hello bullet. point world"] {
            assert_eq!(apply(&s, text), text, "{text:?}");
        }
    }

    /// A blocked two-word phrase must not eat its second word: "line" gets its
    /// own look, and "the new line manager" stays put either way.
    #[test]
    fn a_blocked_phrase_releases_its_second_word() {
        let s = all();
        assert_eq!(
            apply(&s, "the new line manager wants a numbered list"),
            "the new line manager wants a numbered list"
        );
    }

    // ---- adversarial -----------------------------------------------------

    #[test]
    fn a_command_at_the_very_start() {
        let s = all();
        assert_eq!(apply(&s, "new line hello"), "\nhello");
        assert_eq!(apply(&s, "new paragraph hello"), "\n\nhello");
        // a list starts at the cursor rather than pushing an empty line first
        assert_eq!(apply(&s, "bullet point milk"), "- milk");
        assert_eq!(apply(&s, "comma hello"), ", hello");
    }

    #[test]
    fn a_command_at_the_very_end() {
        let s = all();
        assert_eq!(apply(&s, "hello new line"), "hello\n");
        assert_eq!(apply(&s, "hello new paragraph"), "hello\n\n");
        assert_eq!(apply(&s, "that is all period"), "that is all.");
        // a dangling marker is honest: the user asked for an item and said
        // nothing in it
        assert_eq!(apply(&s, "hello bullet point"), "hello\n- ");
    }

    #[test]
    fn two_commands_in_a_row() {
        let s = all();
        assert_eq!(
            apply(&s, "hello new paragraph new line world"),
            "hello\n\n\nworld"
        );
        assert_eq!(apply(&s, "hello comma period world"), "hello. world");
        assert_eq!(
            apply(&s, "alpha new paragraph bullet point beta"),
            "alpha\n\n- beta"
        );
    }

    /// The model already emitted punctuation around the spoken command. Both
    /// the comma in front and the full stop behind belong to the command, not
    /// to the sentence.
    #[test]
    fn punctuation_the_model_already_emitted() {
        let s = all();
        assert_eq!(
            apply(&s, "New paragraph. Then we ship."),
            "\n\nThen we ship."
        );
        assert_eq!(apply(&s, "hello comma, world"), "hello, world");
        assert_eq!(apply(&s, "hello, comma world"), "hello, world");
        assert_eq!(apply(&s, "that is all, period."), "that is all.");
        assert_eq!(
            apply(&s, "buy milk, bullet point, walk the dog"),
            "buy milk\n- walk the dog"
        );
    }

    #[test]
    fn mixed_case_still_commands() {
        let s = all();
        assert_eq!(apply(&s, "hello New Paragraph world"), "hello\n\nworld");
        assert_eq!(apply(&s, "hello NEW LINE world"), "hello\nworld");
        assert_eq!(apply(&s, "hello Comma world"), "hello, world");
        assert_eq!(apply(&s, "Bullet Point milk"), "- milk");
    }

    /// A quoted command is being talked about. Straight single quotes make the
    /// phrase unmatchable in the first place, which is the same answer by a
    /// different route.
    #[test]
    fn a_command_inside_a_quotation_is_left_alone() {
        let s = all();
        for text in [
            "he wrote \"new paragraph\" in the margin",
            "she typed “bullet point” instead",
            "the manual says 'new line' twice",
            "he said \"comma\" out loud",
        ] {
            assert_eq!(apply(&s, text), text, "{text:?}");
        }
    }

    /// An utterance that is nothing but commands. Each one still emits exactly
    /// what it says, including the run of breaks.
    #[test]
    fn an_utterance_of_only_commands() {
        let s = all();
        assert_eq!(apply(&s, "new paragraph"), "\n\n");
        assert_eq!(apply(&s, "new paragraph new paragraph"), "\n\n\n\n");
        assert_eq!(apply(&s, "comma"), ",");
        assert_eq!(apply(&s, "bullet point bullet point"), "- \n- ");
    }

    /// Commands sitting next to text the naive implementations break on.
    #[test]
    fn commands_next_to_non_ascii_text() {
        let s = all();
        assert_eq!(apply(&s, "日本語 new paragraph 日本語"), "日本語\n\n日本語");
        assert_eq!(apply(&s, "café comma résumé"), "café, résumé");
        assert_eq!(apply(&s, "👩‍💻 bullet point 🚀"), "👩‍💻\n- 🚀");
        assert_eq!(
            apply(&s, "e\u{0301}gal new line e\u{0301}gal"),
            "e\u{0301}gal\ne\u{0301}gal"
        );
    }

    /// Zero-width characters glue a word together, so a command wearing one is
    /// not a command. Conservative, and better than splitting on it.
    #[test]
    fn a_zero_width_character_inside_a_phrase_blocks_the_match() {
        let text = "hello new\u{200b} line world";
        assert_eq!(apply(&all(), text), text);
    }

    /// The shipping default, end to end on an utterance that mixes both sets:
    /// structure appears, the punctuation words stay words.
    #[test]
    fn the_documented_default_does_structure_only() {
        assert_eq!(
            apply(
                &shipped_default(),
                "here is the plan colon new paragraph bullet point ship it comma today"
            ),
            "here is the plan colon\n\n- ship it comma today"
        );
    }

    /// Leading and trailing whitespace around a command is the model's, not
    /// the user's, and goes with the command.
    #[test]
    fn whitespace_around_a_command_is_absorbed() {
        let s = all();
        assert_eq!(apply(&s, "   bullet point   milk   "), "- milk   ");
        assert_eq!(apply(&s, "alpha    new line    beta"), "alpha\nbeta");
    }

    // ---- untouched -------------------------------------------------------

    /// None of the torture inputs contains a command, so every one of them
    /// must come back byte-identical even with both sets on.
    #[test]
    fn torture_inputs_are_untouched() {
        let s = all();
        for input in torture_inputs() {
            assert_eq!(s.apply(&input), input, "changed {:?}", truncate(&input));
        }
    }

    // ---- idempotence -----------------------------------------------------

    /// `apply(apply(x)) == apply(x)`. The trap: once "new line" is a `\n`, the
    /// second pass sees different neighbours — a command may now sit at the
    /// start of a line, or next to punctuation that was not there before.
    #[test]
    fn applying_twice_changes_nothing() {
        let s = all();
        let corpus: Vec<String> = torture_inputs()
            .into_iter()
            .chain(COMMANDS_CORPUS.iter().map(|(i, _)| (*i).to_string()))
            .chain(PROSE_CORPUS.iter().map(|p| (*p).to_string()))
            .chain(
                [
                    "bullet point buy milk bullet point walk the dog",
                    "numbered list alpha bullet point beta",
                    "hello new paragraph new line world",
                    "that is all, period.",
                    "he said open quote ship it close quote yesterday",
                ]
                .iter()
                .map(|s| (*s).to_string()),
            )
            .collect();
        for input in corpus {
            let once = s.apply(&input);
            assert_eq!(
                s.apply(&once),
                once,
                "not idempotent on {:?}",
                truncate(&input)
            );
        }
    }

    // ---- prefix stability -------------------------------------------------

    #[test]
    fn is_not_prefix_stable() {
        assert!(!all().prefix_stable());
        assert!(!shipped_default().prefix_stable());
    }

    /// The counterexample, run against the real implementation rather than
    /// asserted from a doc comment. A streaming pass that stops between the
    /// two words of a command has already typed characters the finished
    /// utterance does not begin with.
    #[test]
    fn prefix_violation_finds_a_counterexample() {
        let s = shipped_default();
        let whole = "buy milk bullet point walk the dog";
        let (prefix, polished_prefix, polished_whole) = prefix_violation(&s, whole).unwrap();
        assert_eq!(prefix, "buy milk ");
        assert_eq!(polished_prefix, "buy milk ");
        assert_eq!(polished_whole, "buy milk\n- walk the dog");
        assert!(!polished_whole.starts_with(&polished_prefix));

        // and the deeper one, past the whitespace: a pass that has heard the
        // first word of the phrase has typed six characters that vanish
        assert_eq!(s.apply("buy milk bullet"), "buy milk bullet");
        assert!(!polished_whole.starts_with("buy milk bullet"));

        // the punctuation set violates it too, one word at a time
        let s = all();
        assert!(prefix_violation(&s, "hello comma world").is_some());
    }

    // ---- validation -------------------------------------------------------

    #[test]
    fn validation_flags_a_config_that_cannot_do_anything() {
        let both_off = Spoken::new(SpokenConfig {
            enabled: true,
            structural: false,
            punctuation: false,
        });
        assert_eq!(both_off.validate().len(), 1);
        assert!(both_off.validate()[0].contains("both command sets are off"));
    }

    #[test]
    fn a_usable_config_validates_clean() {
        for s in [
            Spoken::new(SpokenConfig::default()),
            shipped_default(),
            all(),
            // disabled with both sets off is not a complaint: that is just off
            Spoken::new(SpokenConfig {
                enabled: false,
                structural: false,
                punctuation: false,
            }),
        ] {
            assert!(s.validate().is_empty(), "{:?} complained", s.cfg);
        }
    }

    // ---- shape ------------------------------------------------------------

    #[test]
    fn the_name_is_stable() {
        assert_eq!(all().name(), "spoken");
    }

    /// No command's words may be a prefix of another's, or the table order
    /// would silently decide which one wins.
    #[test]
    fn no_command_shadows_another() {
        for a in COMMANDS {
            for b in COMMANDS {
                if std::ptr::eq(a, b) {
                    continue;
                }
                assert!(
                    !(a.words.len() < b.words.len() && b.words.starts_with(a.words)),
                    "{:?} shadows {:?}",
                    a.words,
                    b.words
                );
            }
        }
    }

    #[test]
    fn the_supported_token_list_is_the_issues_list() {
        let structural: Vec<String> = COMMANDS
            .iter()
            .filter(|c| c.structural)
            .map(|c| c.words.join(" "))
            .collect();
        let punctuation: Vec<String> = COMMANDS
            .iter()
            .filter(|c| !c.structural)
            .map(|c| c.words.join(" "))
            .collect();
        assert_eq!(
            structural,
            ["new paragraph", "new line", "bullet point", "numbered list"]
        );
        assert_eq!(
            punctuation,
            [
                "comma",
                "period",
                "full stop",
                "question mark",
                "colon",
                "open quote",
                "close quote",
                "dash"
            ]
        );
    }

    // ---- cost -------------------------------------------------------------

    /// The transform is one linear scan with a lookahead of at most two words,
    /// so a 2 MB input is milliseconds, not minutes. The bound is 200x the
    /// measured cost on an M1: it exists to catch an accidentally quadratic
    /// rewrite, not to police a few milliseconds on a loaded CI runner.
    #[test]
    fn cost_is_linear_in_the_input() {
        let s = all();
        let big = "the quick brown fox jumps over the lazy dog. ".repeat(45_000);
        let start = std::time::Instant::now();
        let out = s.apply(&big);
        let elapsed = start.elapsed();
        assert_eq!(out, big);
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "2 MB took {elapsed:?} — this should be a linear scan"
        );
    }
}
