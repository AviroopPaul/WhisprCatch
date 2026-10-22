//! Number normalization — Parakeet writes numbers as words ("twenty five"),
//! and nobody wants that in a Slack message. SCOPE.md flagged it as a known
//! model quirk; this is where it gets fixed.
//!
//! Inverse text normalization: spoken number words become the written form.
//! Nine categories, each independently switchable, because the right answer
//! genuinely differs by user *and* by category — a novelist wants "twenty five"
//! left alone and almost everybody wants "$25".
//!
//! # Where the line is drawn
//!
//! Context decides, and getting it wrong is worse than doing nothing. Three
//! rules do nearly all of the work of not guessing:
//!
//! 1. **A run of adjacent number words is all-or-nothing.** The scanner first
//!    finds the *maximal* run of number words glued together by a single space
//!    or a hyphen, and converts it only if the whole run parses as one number.
//!    "three four twenty twenty five" (a spoken date, locale-ambiguous) does
//!    not parse as one number, so none of it is touched — not even the
//!    "twenty five" hiding at the end. Converting a fragment of a run is the
//!    single easiest way to mangle a sentence, and this rule makes it
//!    structurally impossible.
//! 2. **A unit word licenses digits; nothing else does.** "twenty five dollars"
//!    → "$25" and "five percent" → "5%" at any size, because "dollars" and
//!    "percent" say out loud that a quantity was meant. A *bare* number has no
//!    such witness, so it is converted only at or above
//!    [`NumbersConfig::spell_out_below`] (default 10, the usual style-guide
//!    line). That one threshold is what keeps "one of the things", "no one",
//!    "at first", "the second time" and "a quarter of the budget" intact,
//!    without any part-of-speech tagging.
//! 3. **Never invent a separator or a reordering where it could carry
//!    meaning.** Thousands separators start at five digits, so "two thousand
//!    and twenty four" is "2024" whether the user meant the year or the count
//!    — the ambiguity the issue calls out never has to be resolved, because
//!    both readings are written the same way — while "twenty five thousand
//!    dollars" still gets the "$25,000" it deserves. Phone numbers are not
//!    grouped at all (grouping is locale-specific and guessing it wrong is
//!    mangling), and dates keep the word order they were spoken in ("the 25th
//!    of May", "May 25").
//! 4. **No rule may depend on how a neighbouring number is written *right
//!    now*.** `apply` must be idempotent, and the whole of the text around a
//!    number changes once the pass has run: "ten dollars" becomes "$10", "twenty
//!    fifth" becomes "25th", "twenty thirty dollars" leaves its "dollars"
//!    stranded next to digits. Every idempotence bug in this module's history
//!    was a guard that asked "is a number next to me?" and got a different
//!    answer on the second pass. If a rule needs that answer, it must ask the
//!    scan itself (`written_in_digits`) or recognise both spellings
//!    (`number_follows`, `unit_phrase_follows`), and it must decline when it
//!    would strand a unit word. `fuzz_is_idempotent_and_never_panics` is the
//!    only reason any of this is known; hand-written cases found none of it.
//!
//! Deliberate pass-throughs, each of which a naive implementation gets wrong:
//!
//! ```text
//! "one of the things"          one is below the threshold
//! "chapter twenty five"        converts to "chapter 25" only because you asked
//!                              for cardinals; there is no signal that separates
//!                              this from "twenty five widgets", so it is a
//!                              setting, not a guess
//! "a quarter of the budget"    "of" is not "past" or "to"
//! "the first of many"          "many" is not a month
//! "the point is"               "point" needs a number on both sides
//! "oh, I see"                  "oh" is only a zero inside a phone number, a
//!                              year, or a clock time
//! "three four twenty twenty five"  does not parse as one number (rule 1)
//! "Twenty One Pilots"          a capital inside a run reads as a proper noun
//! "five to ten"                a range, never a time — "N to M" is a British
//!                              clock reading at most half the time, so it
//!                              becomes "5 to 10" and never "9:55"
//! "twenty five pounds"         "25 pounds" — pounds are a weight as well as a
//!                              currency, so the number is digitised but no
//!                              symbol is invented
//! ```
//!
//! Everything here is pure: same input, same output, no I/O, no allocation
//! beyond the returned `String` and small per-run scratch.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::Transform;

/// Which spoken forms get rewritten, and how far down the number line to go.
///
/// `enabled` ships `false` like every transform in this crate. The categories
/// ship *on*, so that switching the transform on does something useful without
/// a second visit to the config file; a prose writer who wants "$25" but not
/// "chapter 25" turns `cardinals` off and keeps the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NumbersConfig {
    /// Master switch for the whole transform.
    pub enabled: bool,

    /// Bare whole numbers: "twenty five" → "25".
    pub cardinals: bool,

    /// Bare ordinals: "twenty fifth" → "25th".
    pub ordinals: bool,

    /// "three point one four" → "3.14".
    pub decimals: bool,

    /// "twenty five percent" → "25%" (also "per cent").
    pub percentages: bool,

    /// "twenty five dollars" → "$25", "fifty cents" → "50 cents".
    pub currency: bool,

    /// "at three thirty" → "at 3:30", "a quarter past three" → "3:15",
    /// "three o'clock" → "3 o'clock".
    pub times: bool,

    /// "May twenty fifth" → "May 25", "the third of March" → "the 3rd of
    /// March", "twenty twenty four" → "2024".
    pub dates: bool,

    /// Seven or more spoken digits in a row: "five five five one two three
    /// four" → "5551234".
    pub phone_numbers: bool,

    /// "version two point one point three" → "version 2.1.3".
    pub version_strings: bool,

    /// Bare numbers below this stay spelled out. A unit word ("percent",
    /// "dollars", "o'clock") licenses digits at any size, so this only governs
    /// numbers with no witness. 10 is the common style-guide line — and the
    /// reason "one of the things" and "the second time" survive untouched.
    /// Set to 0 to convert everything down to "zero".
    pub spell_out_below: u32,
}

impl Default for NumbersConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cardinals: true,
            ordinals: true,
            decimals: true,
            percentages: true,
            currency: true,
            times: true,
            dates: true,
            phone_numbers: true,
            version_strings: true,
            spell_out_below: 10,
        }
    }
}

impl NumbersConfig {
    fn any_category(&self) -> bool {
        self.cardinals
            || self.ordinals
            || self.decimals
            || self.percentages
            || self.currency
            || self.times
            || self.dates
            || self.phone_numbers
            || self.version_strings
    }
}

/// Rewrites spelled-out numbers as digits.
///
/// Runs last, so it sees the final wording rather than digits some later pass
/// would have rewritten anyway.
pub struct Numbers {
    cfg: NumbersConfig,
}

impl Numbers {
    pub fn new(cfg: NumbersConfig) -> Self {
        Self { cfg }
    }
}

impl Transform for Numbers {
    fn name(&self) -> &'static str {
        "numbers"
    }

    fn apply(&self, text: &str) -> String {
        if !self.cfg.enabled || !self.cfg.any_category() {
            return text.to_string();
        }
        self.convert(text)
    }

    /// Not prefix-stable. A streaming pass that has heard `"twenty"` types
    /// `"20"`, and the finished `"twenty five percent"` polishes to
    /// `"25 percent"` — the `0` already on screen has to become a `5`.
    /// Compound numerals are exactly the case this transform exists for, and
    /// they always straddle the prefix boundary partway through.
    /// `prefix_violation_is_real` pins the counterexample.
    fn prefix_stable(&self) -> bool {
        false
    }

    fn validate(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.cfg.any_category() {
            out.push(
                "numbers: every category is off, so the transform cannot change anything — \
                 turn one on or leave `enabled` false"
                    .to_string(),
            );
        }
        if self.cfg.spell_out_below > 1000 {
            out.push(format!(
                "numbers: spell_out_below = {} keeps every bare number under a thousand spelled \
                 out, which is close to switching cardinals off — set `cardinals = false` if that \
                 is what you meant",
                self.cfg.spell_out_below
            ));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// vocabulary
// ---------------------------------------------------------------------------

const THOUSAND: u128 = 1_000;
const MILLION: u128 = 1_000_000;

/// A number word, resolved. `And` and `Point` are *bridges*: they are only part
/// of a run when a real number word follows them, which is what keeps "and then"
/// and "the point is" out of the parser. `Oh` is a full member of a run but may
/// not start one, so "oh, I see" is safe while "three oh five" is not split.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum W {
    /// zero..nine
    Digit(u128),
    /// ten..nineteen
    Teen(u128),
    /// twenty..ninety
    Tens(u128),
    Hundred,
    /// thousand and up, with the word kept for "2 million"
    Scale(u128, &'static str),
    /// "a" immediately before "hundred"/"thousand"/…
    A,
    And,
    Point,
    /// spoken zero, e.g. "nineteen oh five", "three oh five"
    Oh,
    /// first..ninetieth, as its cardinal value
    Ord(u128),
    /// hundredth/thousandth/millionth/billionth, as its scale
    OrdScale(u128),
}

impl W {
    fn is_bridge(self) -> bool {
        matches!(self, W::And | W::Point)
    }

    /// A run may not begin with a bridge, nor with a spoken "oh".
    fn can_start_run(self) -> bool {
        !self.is_bridge() && self != W::Oh
    }
}

fn vocab(lower: &str) -> Option<W> {
    Some(match lower {
        "zero" => W::Digit(0),
        "one" => W::Digit(1),
        "two" => W::Digit(2),
        "three" => W::Digit(3),
        "four" => W::Digit(4),
        "five" => W::Digit(5),
        "six" => W::Digit(6),
        "seven" => W::Digit(7),
        "eight" => W::Digit(8),
        "nine" => W::Digit(9),
        "ten" => W::Teen(10),
        "eleven" => W::Teen(11),
        "twelve" => W::Teen(12),
        "thirteen" => W::Teen(13),
        "fourteen" => W::Teen(14),
        "fifteen" => W::Teen(15),
        "sixteen" => W::Teen(16),
        "seventeen" => W::Teen(17),
        "eighteen" => W::Teen(18),
        "nineteen" => W::Teen(19),
        "twenty" => W::Tens(20),
        "thirty" => W::Tens(30),
        "forty" => W::Tens(40),
        "fifty" => W::Tens(50),
        "sixty" => W::Tens(60),
        "seventy" => W::Tens(70),
        "eighty" => W::Tens(80),
        "ninety" => W::Tens(90),
        "hundred" => W::Hundred,
        "thousand" => W::Scale(THOUSAND, "thousand"),
        "million" => W::Scale(MILLION, "million"),
        "billion" => W::Scale(1_000_000_000, "billion"),
        "trillion" => W::Scale(1_000_000_000_000, "trillion"),
        "quadrillion" => W::Scale(1_000_000_000_000_000, "quadrillion"),
        "quintillion" => W::Scale(1_000_000_000_000_000_000, "quintillion"),
        "and" => W::And,
        "point" => W::Point,
        "oh" => W::Oh,
        "first" => W::Ord(1),
        "second" => W::Ord(2),
        "third" => W::Ord(3),
        "fourth" => W::Ord(4),
        "fifth" => W::Ord(5),
        "sixth" => W::Ord(6),
        "seventh" => W::Ord(7),
        "eighth" => W::Ord(8),
        "ninth" => W::Ord(9),
        "tenth" => W::Ord(10),
        "eleventh" => W::Ord(11),
        "twelfth" => W::Ord(12),
        "thirteenth" => W::Ord(13),
        "fourteenth" => W::Ord(14),
        "fifteenth" => W::Ord(15),
        "sixteenth" => W::Ord(16),
        "seventeenth" => W::Ord(17),
        "eighteenth" => W::Ord(18),
        "nineteenth" => W::Ord(19),
        "twentieth" => W::Ord(20),
        "thirtieth" => W::Ord(30),
        "fortieth" => W::Ord(40),
        "fiftieth" => W::Ord(50),
        "sixtieth" => W::Ord(60),
        "seventieth" => W::Ord(70),
        "eightieth" => W::Ord(80),
        "ninetieth" => W::Ord(90),
        "hundredth" => W::OrdScale(100),
        "thousandth" => W::OrdScale(THOUSAND),
        "millionth" => W::OrdScale(MILLION),
        "billionth" => W::OrdScale(1_000_000_000),
        _ => return None,
    })
}

/// The cardinal word an ordinal was built from, so "twenty fifth" can be parsed
/// by exactly the same grammar as "twenty five".
fn ord_as_cardinal(v: u128) -> Option<W> {
    Some(match v {
        0..=9 => W::Digit(v),
        10..=19 => W::Teen(v),
        20 | 30 | 40 | 50 | 60 | 70 | 80 | 90 => W::Tens(v),
        _ => return None,
    })
}

/// 1st / 2nd / 3rd / 4th, with the 11-12-13 exception.
fn ord_suffix(v: u128) -> &'static str {
    if (11..=13).contains(&(v % 100)) {
        return "th";
    }
    match v % 10 {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    }
}

/// A spoken digit, phone-number style. "oh" is a zero *here* and nowhere else.
fn spoken_digit(lower: &str) -> Option<char> {
    Some(match lower {
        "zero" => '0',
        "oh" => '0',
        "one" => '1',
        "two" => '2',
        "three" => '3',
        "four" => '4',
        "five" => '5',
        "six" => '6',
        "seven" => '7',
        "eight" => '8',
        "nine" => '9',
        _ => return None,
    })
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Cur {
    /// A symbol that goes in front: "$25".
    Symbol(&'static str),
    /// The spoken word stays where it is and only the number changes. "pounds"
    /// is weight at least as often as it is money, and "£25 of flour" is the
    /// kind of mangling this transform exists to avoid.
    KeepWord,
}

fn currency_word(lower: &str) -> Option<Cur> {
    Some(match lower {
        "dollar" | "dollars" => Cur::Symbol("$"),
        "euro" | "euros" => Cur::Symbol("€"),
        "yen" => Cur::Symbol("¥"),
        "rupee" | "rupees" => Cur::Symbol("₹"),
        "pound" | "pounds" | "cent" | "cents" => Cur::KeepWord,
        _ => return None,
    })
}

/// A plural number word right after a run means the run was modifying it, not
/// standing on its own: "the nineteen sixties" is a decade, and "the 19
/// sixties" is nonsense. None of these are number words themselves, so without
/// this the run in front of them converts happily.
fn is_plural_number_word(lower: &str) -> bool {
    matches!(
        lower,
        "hundreds"
            | "thousands"
            | "millions"
            | "billions"
            | "trillions"
            | "tens"
            | "teens"
            | "dozens"
            | "dozen"
            | "twenties"
            | "thirties"
            | "forties"
            | "fifties"
            | "sixties"
            | "seventies"
            | "eighties"
            | "nineties"
    )
}

/// Something that reads as a number sits directly after this word.
///
/// It has to recognise both forms — the words the scan has not reached yet, and
/// the digits or currency symbol an earlier pass already wrote — or the answer
/// changes between passes. "one dollars ten dollars" declines the first symbol
/// on pass one because "ten" follows; on pass two the text reads "one dollars
/// $10", and a check that only looked for *words* would happily emit "$1".
fn number_follows(toks: &[Tok<'_>], after: usize) -> bool {
    let Some(sep) = toks.get(after + 1) else {
        return false;
    };
    if sep.word {
        return false;
    }
    // The tokeniser glues runs of non-word characters together, so a symbol an
    // earlier pass wrote arrives as part of the separator: " $" here, not "$".
    match sep.text.strip_prefix(' ') {
        // A word that *starts* with a digit counts too: "one cent twentieth"
        // declines on the first pass because "twentieth" is a number, and the
        // second pass sees "20th", which has to count for the same reason.
        Some("") => toks.get(after + 2).is_some_and(|t| {
            t.word
                && (run_at(toks, after + 2).is_some()
                    || t.text.starts_with(|c: char| c.is_ascii_digit()))
        }),
        Some(rest) => rest.starts_with(['$', '€', '¥', '₹', '-']),
        None => false,
    }
}

/// True when a unit word sits just past `end`, i.e. the number is wearing one.
///
/// Rules whose output is not a plain number — a year, a clock time, a version
/// string, a date — must decline when this is true. Otherwise they leave the
/// unit word stranded next to their digits, and the *next* pass hands it to the
/// unit rule: "twenty thirty dollars" becomes "2030 dollars" and then "$2030".
/// Declining leaves the phrase alone, which is the right answer for something
/// nobody actually says.
fn unit_word_follows(toks: &[Tok<'_>], end: usize) -> bool {
    unit_phrase_follows(toks, end - 1)
}

/// A unit word, or the two-word "per cent", directly after token `after`.
///
/// The two-word form has to be here as well as in the unit rule itself: "twelve
/// euros per cent" takes the "euros" and strands "per cent", which the next
/// pass then reads as a percentage of the digits. "per" alone is not a unit —
/// "fifty percent per year" is ordinary English.
fn unit_phrase_follows(toks: &[Tok<'_>], after: usize) -> bool {
    let Some(w) = next_word(toks, after) else {
        return false;
    };
    let Some(lw) = word_lower(toks, w) else {
        return false;
    };
    if is_unit_word(&lw) {
        return true;
    }
    lw == "per"
        && next_word(toks, w)
            .and_then(|c| word_lower(toks, c))
            .is_some_and(|c| c == "cent" || c == "cents")
}

/// A word that claims a number in front of it. "per" is missing on purpose: it
/// is only half a unit ("per cent"), and it is also an ordinary word in "fifty
/// percent per year".
fn is_unit_word(lower: &str) -> bool {
    lower == "percent" || lower == "o'clock" || currency_word(lower).is_some()
}

fn is_month(lower: &str) -> bool {
    matches!(
        lower,
        "january"
            | "february"
            | "march"
            | "april"
            | "may"
            | "june"
            | "july"
            | "august"
            | "september"
            | "october"
            | "november"
            | "december"
    )
}

/// Words that make a bare "three thirty" a clock time rather than a quantity.
fn is_time_cue(lower: &str) -> bool {
    matches!(
        lower,
        "at" | "around"
            | "by"
            | "until"
            | "till"
            | "before"
            | "after"
            | "from"
            | "between"
            | "it's"
            | "its"
    )
}

// ---------------------------------------------------------------------------
// tokens
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Tok<'a> {
    text: &'a str,
    word: bool,
}

fn is_apostrophe(c: char) -> bool {
    c == '\'' || c == '\u{2019}'
}

/// Split into alternating word / non-word runs. Concatenating every `text`
/// reproduces the input byte for byte, which is what makes "leave it alone"
/// free of charge.
///
/// An apostrophe counts as part of a word only *between* letters, so "o'clock"
/// and "it's" survive while a quoted 'twenty five' still parses.
fn tokenize(text: &str) -> Vec<Tok<'_>> {
    let mut toks = Vec::new();
    let mut start = 0usize;
    let mut cur: Option<bool> = None;
    let mut prev_alnum = false;
    let mut it = text.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let next_alpha = it.peek().is_some_and(|(_, n)| n.is_alphabetic());
        let word = c.is_alphanumeric() || (is_apostrophe(c) && prev_alnum && next_alpha);
        match cur {
            Some(prev) if prev == word => {}
            Some(prev) => {
                toks.push(Tok {
                    text: &text[start..i],
                    word: prev,
                });
                start = i;
            }
            None => start = i,
        }
        cur = Some(word);
        prev_alnum = c.is_alphanumeric();
    }
    if let Some(prev) = cur {
        toks.push(Tok {
            text: &text[start..],
            word: prev,
        });
    }
    toks
}

/// Lowercase into a stack buffer. `None` for anything that cannot be a number
/// word anyway (too long, or not ASCII), which is most of a real transcript.
fn lower<'a>(t: &str, buf: &'a mut [u8; 16]) -> Option<&'a str> {
    let b = t.as_bytes();
    if b.is_empty() || b.len() > buf.len() {
        return None;
    }
    for (i, c) in b.iter().enumerate() {
        if !c.is_ascii() {
            return None;
        }
        buf[i] = c.to_ascii_lowercase();
    }
    std::str::from_utf8(&buf[..b.len()]).ok()
}

fn word_lower(toks: &[Tok<'_>], i: usize) -> Option<String> {
    let t = toks.get(i)?;
    if !t.word {
        return None;
    }
    let mut buf = [0u8; 16];
    lower(t.text, &mut buf).map(str::to_string)
}

fn word_vocab(toks: &[Tok<'_>], i: usize) -> Option<W> {
    let t = toks.get(i)?;
    if !t.word {
        return None;
    }
    let mut buf = [0u8; 16];
    vocab(lower(t.text, &mut buf)?)
}

fn word_is(toks: &[Tok<'_>], i: usize, want: &str) -> bool {
    toks.get(i)
        .is_some_and(|t| t.word && t.text.eq_ignore_ascii_case(want))
}

fn is_digits(t: &Tok<'_>) -> bool {
    t.word && !t.text.is_empty() && t.text.bytes().all(|b| b.is_ascii_digit())
}

fn starts_upper(t: &Tok<'_>) -> bool {
    t.text.chars().next().is_some_and(char::is_uppercase)
}

/// The next word token, if it is glued to `after` by a single space. Anything
/// else — a newline, two spaces, a comma, a full stop — ends the phrase.
fn next_word(toks: &[Tok<'_>], after: usize) -> Option<usize> {
    let sep = toks.get(after + 1)?;
    if sep.word || sep.text != " " {
        return None;
    }
    let w = toks.get(after + 2)?;
    if !w.word {
        return None;
    }
    Some(after + 2)
}

/// The word token before `before`, glued to it by a single space.
fn prev_word(toks: &[Tok<'_>], before: usize) -> Option<usize> {
    let sep = toks.get(before.checked_sub(1)?)?;
    if sep.word || sep.text != " " {
        return None;
    }
    let w = toks.get(before.checked_sub(2)?)?;
    if !w.word {
        return None;
    }
    Some(before - 2)
}

// ---------------------------------------------------------------------------
// runs
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug)]
struct Item {
    tok: usize,
    w: W,
}

/// A maximal run of number words, or a single run of digits already in the
/// text. Rules only ever fire on a *whole* run (or a prefix ending at a bridge
/// word), never on part of one.
#[derive(Debug)]
struct Run<'a> {
    start: usize,
    /// exclusive token index
    end: usize,
    items: Vec<Item>,
    digits: Option<Cow<'a, str>>,
    /// a capital letter inside the run: "Twenty One Pilots", not 21
    proper: bool,
    /// longer than any real number; declined rather than half-converted
    oversized: bool,
}

/// No spoken number needs anywhere near this many words. A longer run is
/// someone counting out loud, and gets left alone as a whole.
const MAX_RUN_WORDS: usize = 64;

impl Run<'_> {
    /// Item counts to try, longest first: the whole run, then each prefix that
    /// stops before an "and". "twenty five and ten" fails as one number but
    /// "twenty five" does not, and "and" is a legitimate place to cut because
    /// it is an ordinary conjunction as well as a numeral joiner.
    ///
    /// "point" deliberately is *not* a cut point. Cutting there would turn a
    /// declined "thirty point one four" into "30 point one four", which is
    /// worse than leaving it alone. And there is no cut at all between two
    /// plain number words — that is rule 1 in the module docs.
    fn candidates(&self) -> Vec<usize> {
        let mut out = vec![self.items.len()];
        for (i, it) in self.items.iter().enumerate().rev() {
            if it.w == W::And && i > 0 {
                out.push(i);
            }
        }
        out
    }
}

fn run_at<'a>(toks: &[Tok<'a>], i: usize) -> Option<Run<'a>> {
    let t = toks.get(i)?;
    if !t.word {
        return None;
    }
    if is_digits(t) {
        // A number the model already wrote may be a compound: "3.14", "1,000",
        // "3:30". Take the whole thing, never a piece of it. Cutting "3.14" at
        // the dot lets the "14" claim a following "dollars" and produce
        // "3.$14" — mangling, and not even idempotent, since the same input
        // reached that state only on the second pass.
        let mut end = i + 1;
        while toks
            .get(end)
            .is_some_and(|s| !s.word && matches!(s.text, "." | "," | ":"))
            && toks.get(end + 1).is_some_and(is_digits)
        {
            end += 2;
        }
        let digits: Cow<'a, str> = if end == i + 1 {
            Cow::Borrowed(t.text)
        } else {
            Cow::Owned(toks[i..end].iter().map(|t| t.text).collect())
        };
        return Some(Run {
            start: i,
            end,
            items: Vec::new(),
            digits: Some(digits),
            proper: false,
            oversized: false,
        });
    }

    // What may start a run: a real number word, or "a" in "a hundred".
    let first = match word_vocab(toks, i) {
        Some(w) if w.can_start_run() => w,
        _ => {
            if !word_is(toks, i, "a") {
                return None;
            }
            let nxt = next_word(toks, i)?;
            match word_vocab(toks, nxt) {
                Some(W::Hundred) | Some(W::Scale(_, _)) => W::A,
                _ => return None,
            }
        }
    };

    let mut items = vec![Item { tok: i, w: first }];
    let mut last = i;
    let mut proper = false;
    let mut oversized = false;
    while let Some((tok, w)) = joined_number_word(toks, last) {
        let (tok, w) = if w.is_bridge() {
            // A bridge is only part of the run when a real number word follows.
            match joined_number_word(toks, tok) {
                Some((t2, w2)) if !w2.is_bridge() => {
                    if !oversized {
                        items.push(Item { tok, w });
                    }
                    (t2, w2)
                }
                _ => break,
            }
        } else {
            (tok, w)
        };
        if starts_upper(&toks[tok]) {
            proper = true;
        }
        if !oversized {
            items.push(Item { tok, w });
        }
        last = tok;
        if items.len() > MAX_RUN_WORDS {
            // Keep walking to find the real end so the whole thing is skipped,
            // but stop growing the item list.
            oversized = true;
        }
    }

    Some(Run {
        start: i,
        end: last + 1,
        items,
        digits: None,
        proper,
        oversized,
    })
}

/// The next number word glued to `after` by a single space or a hyphen, so
/// "twenty-five" is one run and "twenty\nfive" is two.
fn joined_number_word(toks: &[Tok<'_>], after: usize) -> Option<(usize, W)> {
    let sep = toks.get(after + 1)?;
    if sep.word || (sep.text != " " && sep.text != "-") {
        return None;
    }
    let w = word_vocab(toks, after + 2)?;
    Some((after + 2, w))
}

// ---------------------------------------------------------------------------
// grammar
// ---------------------------------------------------------------------------

/// 0..99 — "seven", "seventeen", "seventy", "seventy seven".
fn parse_sub100(ws: &[W], i: usize) -> Option<(u128, usize)> {
    match ws.get(i)? {
        W::Digit(v) => Some((*v, i + 1)),
        W::Teen(v) => Some((*v, i + 1)),
        W::Tens(v) => match ws.get(i + 1) {
            // "twenty zero" is not a number; only 1..9 may follow a tens word.
            Some(W::Digit(d)) if *d > 0 => Some((v + d, i + 2)),
            _ => Some((*v, i + 1)),
        },
        _ => None,
    }
}

/// 0..9999 — the "nineteen hundred and five" shape included, because people
/// say years and prices that way.
fn parse_sub1000(ws: &[W], i: usize) -> Option<(u128, usize)> {
    let (mult, j) = match ws.get(i)? {
        W::A => (1, i + 1),
        _ => parse_sub100(ws, i)?,
    };
    if matches!(ws.get(j), Some(W::Hundred)) && (1..=99).contains(&mult) {
        let mut v = mult * 100;
        let mut k = j + 1;
        // British "and": "two hundred and five".
        let after_and = if matches!(ws.get(k), Some(W::And)) {
            k + 1
        } else {
            k
        };
        if let Some((rest, k2)) = parse_sub100(ws, after_and) {
            v += rest;
            k = k2;
        }
        return Some((v, k));
    }
    if matches!(ws[i], W::A) {
        // "a" is only a number in front of a scale word.
        if matches!(ws.get(j), Some(W::Scale(_, _))) {
            return Some((1, j));
        }
        return None;
    }
    Some((mult, j))
}

/// The whole slice as one number, or nothing. Returns the value and, for the
/// exact shape `<n> million|billion|…`, the scale word to keep: "$2 million"
/// beats "$2000000", and keeping the word is the only way to stay readable
/// without inventing thousands separators.
fn parse_cardinal(ws: &[W]) -> Option<(u128, Option<&'static str>)> {
    if ws.is_empty() {
        return None;
    }
    let mut i = 0usize;
    let mut total: u128 = 0;
    let mut last_scale = u128::MAX;
    let mut parts = 0usize;
    let mut keep: Option<(&'static str, u128)> = None;
    loop {
        let (part, j) = parse_sub1000(ws, i)?;
        i = j;
        match ws.get(i) {
            Some(W::Scale(mul, name)) => {
                // Scales must strictly decrease: "two thousand five hundred"
                // is a number, "two thousand three thousand" is not.
                if *mul >= last_scale {
                    return None;
                }
                total = total.checked_add(part.checked_mul(*mul)?)?;
                last_scale = *mul;
                parts += 1;
                i += 1;
                if parts == 1 && part < 1000 && *mul >= MILLION && i == ws.len() {
                    keep = Some((name, part));
                }
                if matches!(ws.get(i), Some(W::And)) {
                    i += 1;
                    if i >= ws.len() {
                        return None; // trailing "and"
                    }
                }
                if i >= ws.len() {
                    break;
                }
            }
            _ => {
                if parts > 0 && part >= last_scale {
                    return None;
                }
                total = total.checked_add(part)?;
                break;
            }
        }
    }
    if i != ws.len() {
        return None;
    }
    // When the scale word is kept, the value written in front of it is the
    // multiplier ("2 million"), not the total.
    match keep {
        Some((name, mult)) => Some((mult, Some(name))),
        None => Some((total, None)),
    }
}

/// The whole slice as one ordinal: "twenty fifth" → 25, "two hundredth" → 200.
fn parse_ordinal(ws: &[W]) -> Option<u128> {
    let (last, head) = ws.split_last()?;
    match last {
        W::Ord(v) => {
            if head.is_empty() {
                return Some(*v);
            }
            let mut rebuilt = head.to_vec();
            rebuilt.push(ord_as_cardinal(*v)?);
            let (val, keep) = parse_cardinal(&rebuilt)?;
            if keep.is_some() {
                return None;
            }
            Some(val)
        }
        W::OrdScale(s) => {
            if head.is_empty() {
                return Some(*s);
            }
            let (mult, keep) = parse_cardinal(head)?;
            if keep.is_some() {
                return None;
            }
            mult.checked_mul(*s)
        }
        _ => None,
    }
}

/// A number as it will be written: integer part, optional fraction, optional
/// scale word kept verbatim, or the original digits if the text already had
/// them.
#[derive(Debug)]
struct Num<'a> {
    value: u128,
    frac: Option<String>,
    scale: Option<&'static str>,
    literal: Option<&'a str>,
}

impl Num<'_> {
    fn plain_int(&self) -> Option<u128> {
        if self.frac.is_none() && self.scale.is_none() {
            Some(self.value)
        } else {
            None
        }
    }

    fn render(&self, neg: bool) -> String {
        let mut s = String::new();
        if neg {
            s.push('-');
        }
        if let Some(lit) = self.literal {
            // Digits the user already had: never re-spell them. "007" is not 7.
            s.push_str(lit);
            return s;
        }
        s.push_str(&grouped(self.value));
        if let Some(f) = &self.frac {
            s.push('.');
            s.push_str(f);
        }
        if let Some(sc) = self.scale {
            s.push(' ');
            s.push_str(sc);
        }
        s
    }
}

/// Thousands separators from five digits up, and never below.
///
/// This is the one place a separator is invented, and the cut-off is what
/// keeps "two thousand and twenty four" → "2024" reading as the year the user
/// probably meant *and* as the count they might have meant — the ambiguity the
/// issue calls out simply never has to be resolved, because both are written
/// the same way. Above four digits there is no year to confuse it with, and
/// "$25,000" beats "$25000" by a mile.
fn grouped(v: u128) -> String {
    let plain = v.to_string();
    if plain.len() <= 4 {
        return plain;
    }
    let mut out = String::with_capacity(plain.len() + plain.len() / 3);
    // The first group is whatever is left over above a whole number of threes.
    let head = match plain.len() % 3 {
        0 => 3,
        n => n,
    };
    let (first, rest) = plain.split_at(head);
    out.push_str(first);
    let mut i = 0;
    while i < rest.len() {
        out.push(',');
        out.push_str(&rest[i..i + 3]);
        i += 3;
    }
    out
}

/// `<cardinal>` or `<cardinal> point <digits> [scale]`, consuming the slice
/// whole.
fn parse_num(ws: &[W]) -> Option<Num<'static>> {
    let Some(p) = ws.iter().position(|w| *w == W::Point) else {
        let (value, scale) = parse_cardinal(ws)?;
        return Some(Num {
            value,
            frac: None,
            scale,
            literal: None,
        });
    };
    let (value, keep) = parse_cardinal(&ws[..p])?;
    if keep.is_some() {
        return None; // "two million point five" is not a thing anyone says
    }
    let mut frac = String::new();
    let mut k = p + 1;
    // Only single digits after the point: "three point twenty five" is
    // ambiguous between 3.25 and 3.2-then-5, so the whole run is declined.
    while let Some(w) = ws.get(k) {
        match w {
            W::Digit(d) => frac.push(char::from_digit(*d as u32, 10)?),
            W::Oh => frac.push('0'),
            _ => break,
        }
        k += 1;
    }
    if frac.is_empty() {
        return None;
    }
    let mut scale = None;
    if let Some(W::Scale(mul, name)) = ws.get(k) {
        if *mul >= MILLION {
            scale = Some(*name);
            k += 1;
        }
    }
    if k != ws.len() {
        return None;
    }
    Some(Num {
        value,
        frac: Some(frac),
        scale,
        literal: None,
    })
}

// ---------------------------------------------------------------------------
// the scan
// ---------------------------------------------------------------------------

impl Numbers {
    fn convert(&self, text: &str) -> String {
        let toks = tokenize(text);
        let mut out = String::with_capacity(text.len() + 16);
        let mut i = 0usize;
        while i < toks.len() {
            if !toks[i].word {
                out.push_str(toks[i].text);
                i += 1;
                continue;
            }
            match self.rule_at(&toks, i) {
                Some((end, replacement)) => {
                    out.push_str(&replacement);
                    i = end;
                }
                None => {
                    out.push_str(toks[i].text);
                    i += 1;
                }
            }
        }
        out
    }

    /// Everything that can fire at a word token, in priority order. Returns the
    /// token index to resume at and the text to emit for everything skipped.
    fn rule_at(&self, toks: &[Tok<'_>], i: usize) -> Option<(usize, String)> {
        self.rule_at_with(toks, i, true)
    }

    /// `series_ok` is false for the one-level-deep lookahead in
    /// `written_in_digits`, which is what bounds the recursion.
    fn rule_at_with(&self, toks: &[Tok<'_>], i: usize, series_ok: bool) -> Option<(usize, String)> {
        if self.cfg.phone_numbers {
            if let Some(r) = phone_at(toks, i) {
                return Some(r);
            }
        }
        if self.cfg.times {
            if let Some(r) = clock_fraction_at(toks, i) {
                return Some(r);
            }
        }
        if self.cfg.dates {
            if let Some(r) = self.month_then_ordinal_at(toks, i) {
                return Some(r);
            }
        }
        // "negative twenty five" → "-25". "minus" is deliberately absent: it is
        // a preposition at least as often as a sign ("everyone minus five
        // people"), and "minus 25" is right under both readings anyway.
        if word_is(toks, i, "negative") {
            if let Some(n) = next_word(toks, i) {
                if let Some(run) = run_at(toks, n) {
                    if let Some(r) = self.try_run(toks, &run, true, series_ok) {
                        return Some(r);
                    }
                }
            }
        }

        let run = run_at(toks, i)?;
        if let Some(r) = self.try_run(toks, &run, false, series_ok) {
            return Some(r);
        }
        // Nothing fired: emit the run verbatim and skip *all* of it, so no
        // fragment of an unparseable run gets converted on its own.
        Some((run.end, toks[i..run.end].iter().map(|t| t.text).collect()))
    }

    /// Try every rule against every candidate length of a run, longest first.
    ///
    /// `series_ok` is false only for the one-level-deep call `written_in_digits`
    /// makes to find out how a neighbouring number will be written; it is what
    /// stops the two sides of a series asking each other the same question for
    /// ever.
    fn try_run(
        &self,
        toks: &[Tok<'_>],
        run: &Run<'_>,
        neg: bool,
        series_ok: bool,
    ) -> Option<(usize, String)> {
        if run.proper || run.oversized {
            return None;
        }
        // "the nineteen sixties", "hundreds of thousands": the run is modifying
        // the plural, not standing alone.
        let plural_follows = next_word(toks, run.end - 1)
            .and_then(|n| word_lower(toks, n))
            .is_some_and(|w| is_plural_number_word(&w));
        if plural_follows {
            return None;
        }
        // "half a million dollars" is not "half $1 million". A fraction word in
        // front makes the number part of a quantity this pass cannot express,
        // so the whole phrase is left as spoken.
        let fraction_before = prev_word(toks, run.start)
            .and_then(|p| word_lower(toks, p))
            .is_some_and(|w| w == "half");
        if fraction_before {
            return None;
        }
        if let Some(lit) = run.digits.as_deref() {
            // A compound like "3.14" or "1,000" does not parse as one integer,
            // and that is the answer: it is left exactly as the model wrote it.
            let value = lit.parse::<u128>().ok()?;
            let num = Num {
                value,
                frac: None,
                scale: None,
                literal: Some(lit),
            };
            // A unit word may still put a symbol on digits the model wrote
            // ("25 percent" → "25%"), but nothing else touches them — not even
            // a spoken "negative".
            //
            // Folding "negative 2019" into "-2019" looks harmless and is not:
            // "negative twenty nineteen" polishes to "negative 2019", because
            // the year rule refuses to wear a sign. A second pass would then
            // find digits and produce "-2019", so the transform would not be
            // idempotent. Found by `fuzz_is_idempotent_and_never_panics`, which
            // is exactly the sort of thing a hand-written corpus never catches.
            return self.unit_rule(toks, run.end, &num, neg);
        }

        for k in run.candidates() {
            if k == 0 {
                continue;
            }
            let ws: Vec<W> = run.items[..k].iter().map(|it| it.w).collect();
            let end = run.items[k - 1].tok + 1;
            if let Some(r) = self.version_rule(toks, run.start, end, &ws, neg) {
                return Some(r);
            }
            if let Some(num) = parse_num(&ws) {
                if let Some(r) = self.unit_rule(toks, end, &num, neg) {
                    return Some(r);
                }
            }
            if let Some(r) = self.clock_hm_rule(toks, run.start, end, &ws, neg) {
                return Some(r);
            }
            if let Some(r) = self.ordinal_of_month_rule(toks, end, &ws, neg) {
                return Some(r);
            }
            if let Some(r) = self.year_rule(toks, end, &ws, neg) {
                return Some(r);
            }
            if let Some(r) = self.bare_ordinal_rule(toks, run.start, end, &ws, neg) {
                return Some(r);
            }
            if let Some(r) = self.bare_number_rule(toks, run.start, end, &ws, neg, series_ok) {
                return Some(r);
            }
        }
        None
    }

    /// "version two point one point three" → "version 2.1.3". The cue word is
    /// required: without it "two point one point three" is not a number at all
    /// and the run is declined.
    fn version_rule(
        &self,
        toks: &[Tok<'_>],
        start: usize,
        end: usize,
        ws: &[W],
        neg: bool,
    ) -> Option<(usize, String)> {
        if !self.cfg.version_strings || neg || unit_word_follows(toks, end) {
            return None;
        }
        let cue = prev_word(toks, start)?;
        if !word_is(toks, cue, "version") {
            return None;
        }
        let mut parts = Vec::new();
        for chunk in ws.split(|w| *w == W::Point) {
            // "version four point oh" — the spoken zero counts here.
            if chunk == [W::Oh] {
                parts.push("0".to_string());
                continue;
            }
            let (v, keep) = parse_cardinal(chunk)?;
            if keep.is_some() {
                return None;
            }
            parts.push(v.to_string());
        }
        Some((end, parts.join(".")))
    }

    /// A unit word right after the number is the licence to use digits at any
    /// size: percent, a currency, or o'clock.
    fn unit_rule(
        &self,
        toks: &[Tok<'_>],
        end: usize,
        num: &Num<'_>,
        neg: bool,
    ) -> Option<(usize, String)> {
        let (stop, replacement) = self.unit_rule_inner(toks, end, num, neg)?;
        // Two unit words in a row means the phrase is not one we understand:
        // "nineteen euros percent". Taking the first and leaving the second
        // stranded is also not idempotent, because the second one ends up next
        // to the digits and fires on the following pass — which is how
        // `fuzz_is_idempotent_and_never_panics` found this.
        if unit_phrase_follows(toks, stop - 1) {
            return None;
        }
        Some((stop, replacement))
    }

    fn unit_rule_inner(
        &self,
        toks: &[Tok<'_>],
        end: usize,
        num: &Num<'_>,
        neg: bool,
    ) -> Option<(usize, String)> {
        let unit = next_word(toks, end - 1)?;
        let lw = word_lower(toks, unit)?;

        if self.cfg.percentages {
            if lw == "percent" {
                return Some((unit + 1, format!("{}%", num.render(neg))));
            }
            // British "per cent"
            if lw == "per" {
                if let Some(cent) = next_word(toks, unit) {
                    let cw = word_lower(toks, cent)?;
                    if cw == "cent" || cw == "cents" {
                        return Some((cent + 1, format!("{}%", num.render(neg))));
                    }
                }
            }
        }

        if self.cfg.times && lw == "o'clock" {
            let h = num.plain_int()?;
            if (1..=12).contains(&h) {
                return Some((unit + 1, format!("{} {}", num.render(neg), toks[unit].text)));
            }
            return None;
        }

        if self.cfg.currency {
            if let Some(cur) = currency_word(&lw) {
                // "twenty five dollars fifty" is either a price or "$25, fifty
                // times". Both readings survive "25 dollars 50", so the symbol
                // is declined whenever a bare number follows the currency word.
                if number_follows(toks, unit) {
                    return None;
                }
                let sign = if neg { "-" } else { "" };
                return match cur {
                    // The sign goes outside the symbol: "-$25", never "$-25".
                    Cur::Symbol(sym) => {
                        if let Some((cents_end, cents)) = self.cents_tail(toks, unit) {
                            let int = num.plain_int()?;
                            return Some((cents_end, format!("{sign}{sym}{int}.{cents:02}")));
                        }
                        Some((unit + 1, format!("{sign}{sym}{}", num.render(false))))
                    }
                    Cur::KeepWord => {
                        Some((unit + 1, format!("{} {}", num.render(neg), toks[unit].text)))
                    }
                };
            }
        }
        None
    }

    /// "… and fifty cents" after a currency word, so "twenty five dollars and
    /// fifty cents" lands as "$25.50" instead of "$25 and 50 cents".
    fn cents_tail(&self, toks: &[Tok<'_>], currency_tok: usize) -> Option<(usize, u128)> {
        let and = next_word(toks, currency_tok)?;
        if !word_is(toks, and, "and") {
            return None;
        }
        let n = next_word(toks, and)?;
        let run = run_at(toks, n)?;
        if run.proper || run.oversized || run.digits.is_some() {
            return None;
        }
        let ws: Vec<W> = run.items.iter().map(|it| it.w).collect();
        let (cents, keep) = parse_cardinal(&ws)?;
        if keep.is_some() || cents >= 100 {
            return None;
        }
        let unit = next_word(toks, run.end - 1)?;
        let lw = word_lower(toks, unit)?;
        if lw != "cent" && lw != "cents" {
            return None;
        }
        Some((unit + 1, cents))
    }

    /// "at three thirty" → "at 3:30". Needs a cue in front ("at", "by", …) or a
    /// meridiem behind, because "three thirty" on its own is as likely to be
    /// two numbers as a time.
    fn clock_hm_rule(
        &self,
        toks: &[Tok<'_>],
        start: usize,
        end: usize,
        ws: &[W],
        neg: bool,
    ) -> Option<(usize, String)> {
        if !self.cfg.times || neg || ws.len() < 2 || unit_word_follows(toks, end) {
            return None;
        }
        let cued = prev_word(toks, start)
            .and_then(|p| word_lower(toks, p))
            .is_some_and(|w| is_time_cue(&w))
            || meridiem_follows(toks, end);
        if !cued {
            return None;
        }
        for k in 1..ws.len() {
            let Some((h, keep)) = parse_cardinal(&ws[..k]) else {
                continue;
            };
            if keep.is_some() || !(1..=12).contains(&h) {
                continue;
            }
            if let Some(m) = parse_minutes(&ws[k..]) {
                return Some((end, format!("{h}:{m:02}")));
            }
        }
        None
    }

    /// "the twenty fifth of May" → "the 25th of May". The month is the licence;
    /// "the first of many" has none and stays as it was.
    fn ordinal_of_month_rule(
        &self,
        toks: &[Tok<'_>],
        end: usize,
        ws: &[W],
        neg: bool,
    ) -> Option<(usize, String)> {
        if !self.cfg.dates || neg {
            return None;
        }
        let v = parse_ordinal(ws)?;
        if !(1..=31).contains(&v) {
            return None;
        }
        let of = next_word(toks, end - 1)?;
        if !word_is(toks, of, "of") {
            return None;
        }
        let month = next_word(toks, of)?;
        if !month_here(toks, month) {
            return None;
        }
        Some((end, format!("{v}{}", ord_suffix(v))))
    }

    /// "May twenty fifth" → "May 25". Fires at the month, not the number.
    fn month_then_ordinal_at(&self, toks: &[Tok<'_>], i: usize) -> Option<(usize, String)> {
        if !month_here(toks, i) {
            return None;
        }
        let n = next_word(toks, i)?;
        let run = run_at(toks, n)?;
        if run.proper || run.oversized || run.digits.is_some() {
            return None;
        }
        let ws: Vec<W> = run.items.iter().map(|it| it.w).collect();
        let v = parse_ordinal(&ws)?;
        if !(1..=31).contains(&v) || unit_word_follows(toks, run.end) {
            return None;
        }
        Some((run.end, format!("{} {v}", toks[i].text)))
    }

    /// "nineteen ninety nine" → "1999", "twenty twenty four" → "2024",
    /// "nineteen oh five" → "1905". Only the 18xx/19xx/20xx shapes, so "ten
    /// thirty" is never a year.
    fn year_rule(
        &self,
        toks: &[Tok<'_>],
        end: usize,
        ws: &[W],
        neg: bool,
    ) -> Option<(usize, String)> {
        if !self.cfg.dates || neg || unit_word_follows(toks, end) {
            return None;
        }
        let century = match ws.first()? {
            W::Teen(v) if *v == 18 || *v == 19 => *v,
            W::Tens(v) if *v == 20 => *v,
            _ => return None,
        };
        let rest = &ws[1..];
        let within = match rest {
            [W::Oh, W::Digit(d)] => *d,
            _ => {
                let (v, keep) = parse_cardinal(rest)?;
                if keep.is_some() || !(10..=99).contains(&v) {
                    return None;
                }
                v
            }
        };
        Some((end, format!("{}", century * 100 + within)))
    }

    fn bare_ordinal_rule(
        &self,
        toks: &[Tok<'_>],
        start: usize,
        end: usize,
        ws: &[W],
        neg: bool,
    ) -> Option<(usize, String)> {
        if !self.cfg.ordinals || neg {
            return None;
        }
        let v = parse_ordinal(ws)?;
        if v < u128::from(self.cfg.spell_out_below) {
            return None;
        }
        // "a hundredth of a second" is a fraction, not a rank — "a 100th of a
        // second" reads like the hundredth item in a list. "the twenty fifth of
        // May" keeps its article and stays a rank.
        let article = prev_word(toks, start)
            .and_then(|p| word_lower(toks, p))
            .is_some_and(|w| w == "a" || w == "an");
        let of = next_word(toks, end - 1).is_some_and(|n| word_is(toks, n, "of"));
        if article && of {
            return None;
        }
        Some((end, format!("{v}{}", ord_suffix(v))))
    }

    fn bare_number_rule(
        &self,
        toks: &[Tok<'_>],
        start: usize,
        end: usize,
        ws: &[W],
        neg: bool,
        series_ok: bool,
    ) -> Option<(usize, String)> {
        let num = parse_num(ws)?;
        let allowed = if num.frac.is_some() {
            self.cfg.decimals
        } else {
            self.cfg.cardinals
        };
        if !allowed {
            return None;
        }
        // The threshold governs bare whole numbers only: a decimal, a kept
        // scale word and a spoken sign are all witnesses in their own right.
        let below = !neg
            && num.frac.is_none()
            && num.scale.is_none()
            && num.value < u128::from(self.cfg.spell_out_below);
        if below && !(series_ok && self.in_a_series_with_digits(toks, start, end)) {
            return None;
        }
        Some((end, num.render(neg)))
    }

    /// True when this number sits either side of "to"/"or"/"and"/"through"
    /// from a number that *is* being written in digits.
    ///
    /// Without this, "five to ten people" comes out "five to 10 people", which
    /// reads like a bug even though each half followed the rule. The series
    /// only lifts the threshold — it never converts the neighbour itself, so
    /// "five to ten percent" still lets the percentage rule own its half and
    /// lands as "5 to 10%".
    fn in_a_series_with_digits(&self, toks: &[Tok<'_>], start: usize, end: usize) -> bool {
        let forward = next_word(toks, end - 1)
            .filter(|j| self.is_series_word(toks, *j))
            .and_then(|j| next_word(toks, j))
            .is_some_and(|n| self.written_in_digits(toks, n));
        if forward {
            return true;
        }
        prev_word(toks, start)
            .filter(|j| self.is_series_word(toks, *j))
            .and_then(|j| prev_word(toks, j))
            // "ten dollars or five": the unit word belongs to the neighbour and
            // vanishes into its symbol, so step over it or the two passes
            // disagree about where the neighbour starts.
            .map(|w| match word_lower(toks, w) {
                Some(lw) if is_unit_word(&lw) => prev_word(toks, w).unwrap_or(w),
                _ => w,
            })
            .map(|w| run_start_of(toks, w))
            .is_some_and(|s| self.written_in_digits(toks, s))
    }

    fn is_series_word(&self, toks: &[Tok<'_>], i: usize) -> bool {
        word_lower(toks, i).is_some_and(|w| matches!(w.as_str(), "to" | "or" | "and" | "through"))
    }

    /// Would the run starting here be written in digits on its own? A plain
    /// whole number at or above the threshold, or one wearing a unit word.
    /// Ask the scan itself, rather than re-deriving the answer.
    ///
    /// An earlier version parsed the neighbour's whole run directly, and got a
    /// different answer from the one the scan would reach through its candidate
    /// prefixes: for "nineteen and five" the direct parse fails, but the scan
    /// cuts at the "and" and writes "19". The neighbour therefore read as words
    /// on the first pass and as digits on the second, and the transform was not
    /// idempotent. Running the real rules — with series licensing switched off,
    /// which is what bounds the recursion at one level — is the only version of
    /// this predicate that agrees with itself across passes.
    fn written_in_digits(&self, toks: &[Tok<'_>], i: usize) -> bool {
        let rendered = match self.rule_at_with(toks, i, false) {
            Some((_, s)) => s,
            // Nothing fires: it reads however it already reads, which for
            // digits the model wrote is still digits.
            None => toks.get(i).map(|t| t.text.to_string()).unwrap_or_default(),
        };
        rendered.starts_with(|c: char| c.is_ascii_digit())
            || rendered.starts_with(['-', '$', '€', '¥', '₹'])
    }
}

/// Walk back to the first token of the run that `w` belongs to.
fn run_start_of(toks: &[Tok<'_>], w: usize) -> usize {
    let mut s = w;
    while let Some(p) = prev_number_word(toks, s) {
        s = p;
    }
    s
}

/// Deliberately stops at a bridge word. Walking back across "and" would find
/// the start of a *word* run on the first pass and a different one on the
/// second, once part of it had become digits — and the series predicate built
/// on top of it would flip. Runs always extend forwards over bridges anyway, so
/// stopping here still lands inside the same run.
fn prev_number_word(toks: &[Tok<'_>], before: usize) -> Option<usize> {
    let sep = toks.get(before.checked_sub(1)?)?;
    if sep.word || (sep.text != " " && sep.text != "-") {
        return None;
    }
    let i = before.checked_sub(2)?;
    word_vocab(toks, i).filter(|w| w.can_start_run()).map(|_| i)
}

/// 0..59 as a minute count. "oh five" is 05; a bare "five" is not, because
/// nobody says "at three five".
fn parse_minutes(ws: &[W]) -> Option<u128> {
    if let [W::Oh, W::Digit(d)] = ws {
        return Some(*d);
    }
    let (v, keep) = parse_cardinal(ws)?;
    if keep.is_some() || !(10..=59).contains(&v) {
        return None;
    }
    Some(v)
}

fn month_here(toks: &[Tok<'_>], i: usize) -> bool {
    // Capitalisation is required: it separates the month from "you may", "the
    // march" and "august company". Both shipped models capitalise proper nouns.
    toks.get(i).is_some_and(starts_upper) && word_lower(toks, i).is_some_and(|w| is_month(&w))
}

/// "am"/"pm"/"a.m."/"p.m." right after a time.
fn meridiem_follows(toks: &[Tok<'_>], end: usize) -> bool {
    let Some(w) = next_word(toks, end - 1) else {
        return false;
    };
    let Some(lw) = word_lower(toks, w) else {
        return false;
    };
    if lw == "am" || lw == "pm" {
        return true;
    }
    if lw != "a" && lw != "p" {
        return false;
    }
    // "a" "." "m"
    toks.get(w + 1).is_some_and(|t| !t.word && t.text == ".")
        && toks
            .get(w + 2)
            .is_some_and(|t| t.word && t.text.eq_ignore_ascii_case("m"))
}

/// "a quarter past three" → "3:15", "half past three" → "3:30", "quarter to
/// four" → "3:45". "a quarter of the budget" has no "past"/"to" and is left
/// alone; so is "five to ten", which is a range as often as it is a time.
fn clock_fraction_at(toks: &[Tok<'_>], i: usize) -> Option<(usize, String)> {
    let mut cur = i;
    if word_is(toks, cur, "a") {
        cur = next_word(toks, cur)?;
    }
    let kind = word_lower(toks, cur)?;
    let minutes = match kind.as_str() {
        "quarter" => 15u128,
        "half" => 30,
        _ => return None,
    };
    let dir_tok = next_word(toks, cur)?;
    let dir = word_lower(toks, dir_tok)?;
    let to = match dir.as_str() {
        "past" => false,
        "to" if minutes == 15 => true, // "half to" is not English
        _ => return None,
    };
    let hour_tok = next_word(toks, dir_tok)?;
    let run = run_at(toks, hour_tok)?;
    if run.proper || run.oversized {
        return None;
    }
    // The hour must be the whole number, not the front of one. A run of words
    // swallows "million" and fails to parse, but "half past 2 million" — which
    // is what the first pass writes — leaves the digit run standing alone with
    // the scale word beside it, and without this the second pass would read it
    // as half past two.
    let more_number_follows = next_word(toks, run.end - 1)
        .and_then(|w| word_vocab(toks, w))
        .is_some();
    if more_number_follows || unit_phrase_follows(toks, run.end - 1) {
        return None;
    }
    let h = match run.digits.as_deref() {
        Some(lit) => lit.parse::<u128>().ok()?,
        None => {
            let ws: Vec<W> = run.items.iter().map(|it| it.w).collect();
            let (v, keep) = parse_cardinal(&ws)?;
            if keep.is_some() {
                return None;
            }
            v
        }
    };
    if !(1..=12).contains(&h) {
        return None;
    }
    let (h, m) = if to {
        (if h == 1 { 12 } else { h - 1 }, 60 - minutes)
    } else {
        (h, minutes)
    };
    Some((run.end, format!("{h}:{m:02}")))
}

/// Seven or more spoken digits in a row is a phone number and nothing else.
/// The digits are concatenated and not grouped: "555-123-4567" is a North
/// American convention, and imposing it on a number spoken anywhere else is
/// exactly the kind of mangling the issue rules out.
fn phone_at(toks: &[Tok<'_>], i: usize) -> Option<(usize, String)> {
    let mut digits = String::new();
    let mut last = i;
    let mut tok = i;
    loop {
        let lw = word_lower(toks, tok)?;
        let Some(d) = spoken_digit(&lw) else { break };
        digits.push(d);
        last = tok;
        match next_word(toks, tok) {
            Some(n) => tok = n,
            None => break,
        }
    }
    if digits.len() < 7 {
        return None;
    }
    Some((last + 1, digits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{prefix_violation, torture_inputs, truncate};

    // -- harness ------------------------------------------------------------

    /// Everything on. The shipping default is off; these tables describe what
    /// a user who switched it on gets.
    fn on() -> Numbers {
        Numbers::new(NumbersConfig {
            enabled: true,
            ..Default::default()
        })
    }

    fn with(f: impl FnOnce(&mut NumbersConfig)) -> Numbers {
        let mut cfg = NumbersConfig {
            enabled: true,
            ..Default::default()
        };
        f(&mut cfg);
        Numbers::new(cfg)
    }

    /// Every table goes through here: the expectation, then idempotence on the
    /// same case, because "25 percent" → "25%" → "25%%" is the trap.
    #[track_caller]
    fn table(t: &Numbers, cases: &[(&str, &str)]) {
        for (input, want) in cases {
            let got = t.apply(input);
            assert_eq!(&got, want, "input {input:?}");
            assert_eq!(t.apply(&got), got, "not idempotent on {input:?} -> {got:?}");
        }
    }

    /// Cases that must come out byte-identical.
    #[track_caller]
    fn unchanged(t: &Numbers, cases: &[&str]) {
        for input in cases {
            let got = t.apply(input);
            assert_eq!(&got, input, "input {input:?} was rewritten");
        }
    }

    // -- category tables ----------------------------------------------------

    #[test]
    fn cardinals() {
        table(
            &on(),
            &[
                ("twenty five", "25"),
                ("twenty-five", "25"),
                ("I need twenty five of them", "I need 25 of them"),
                ("a hundred", "100"),
                ("one hundred", "100"),
                ("one hundred and five", "105"),
                ("two hundred and five", "205"),
                ("nineteen hundred", "1900"),
                ("twenty five hundred", "2500"),
                ("a thousand", "1000"),
                ("two thousand and twenty four", "2024"),
                ("one thousand one hundred", "1100"),
                ("fifteen", "15"),
                ("ninety nine", "99"),
                ("one hundred and twenty three thousand", "123,000"),
                // scale words survive rather than turning into a wall of zeros
                ("two million", "2 million"),
                ("a million", "1 million"),
                ("three billion", "3 billion"),
                ("two million five", "2,000,005"),
                // the headline case from SCOPE.md
                ("chapter twenty five", "chapter 25"),
                ("Twenty five people came.", "25 people came."),
                ("I have twenty five.", "I have 25."),
            ],
        );
    }

    #[test]
    fn ordinals() {
        table(
            &on(),
            &[
                ("the twenty fifth time", "the 25th time"),
                ("the twenty-fifth time", "the 25th time"),
                ("the tenth", "the 10th"),
                ("the eleventh", "the 11th"),
                ("the twelfth", "the 12th"),
                ("the thirteenth", "the 13th"),
                ("the twenty first", "the 21st"),
                ("the twenty second", "the 22nd"),
                ("the twenty third", "the 23rd"),
                ("the hundredth", "the 100th"),
                ("the one hundred and first", "the 101st"),
                ("the one hundred and eleventh", "the 111th"),
                ("the two hundredth", "the 200th"),
                ("the thousandth", "the 1000th"),
                ("the twentieth", "the 20th"),
            ],
        );
    }

    #[test]
    fn decimals() {
        table(
            &on(),
            &[
                ("three point one four", "3.14"),
                ("twenty five point five", "25.5"),
                ("zero point nine", "0.9"),
                ("one point oh five", "1.05"),
                ("two point five million", "2.5 million"),
                ("pi is three point one four one five nine", "pi is 3.14159"),
            ],
        );
    }

    #[test]
    fn percentages() {
        table(
            &on(),
            &[
                ("twenty five percent", "25%"),
                ("five percent", "5%"),
                ("one hundred percent", "100%"),
                ("twelve point five percent", "12.5%"),
                ("twenty five per cent", "25%"),
                ("up twenty five percent year on year", "up 25% year on year"),
                // digits the model already wrote still get the symbol
                ("25 percent", "25%"),
            ],
        );
    }

    #[test]
    fn currency() {
        table(
            &on(),
            &[
                ("twenty five dollars", "$25"),
                ("one dollar", "$1"),
                ("twenty five euros", "€25"),
                ("five hundred rupees", "₹500"),
                ("a thousand yen", "¥1000"),
                ("two million dollars", "$2 million"),
                ("two point five million dollars", "$2.5 million"),
                ("twenty five dollars and fifty cents", "$25.50"),
                ("twenty five dollars and five cents", "$25.05"),
                ("fifty cents", "50 cents"),
                ("five cents", "5 cents"),
                // pounds are a weight as often as a currency: digits, no symbol
                ("twenty five pounds", "25 pounds"),
                ("twenty five pounds of flour", "25 pounds of flour"),
                ("it costs twenty five dollars", "it costs $25"),
            ],
        );
    }

    #[test]
    fn times() {
        table(
            &on(),
            &[
                ("three o'clock", "3 o'clock"),
                ("at three thirty", "at 3:30"),
                ("at three oh five", "at 3:05"),
                ("at ten fifteen", "at 10:15"),
                ("at twelve forty five", "at 12:45"),
                ("three thirty pm", "3:30 pm"),
                ("three thirty p.m.", "3:30 p.m."),
                ("a quarter past three", "3:15"),
                ("quarter past three", "3:15"),
                ("half past three", "3:30"),
                ("a quarter to four", "3:45"),
                ("a quarter to one", "12:45"),
                (
                    "meet me at three thirty tomorrow",
                    "meet me at 3:30 tomorrow",
                ),
                ("it's three thirty", "it's 3:30"),
            ],
        );
    }

    #[test]
    fn dates() {
        table(
            &on(),
            &[
                ("May twenty fifth", "May 25"),
                ("March third", "March 3"),
                ("the first of May", "the 1st of May"),
                ("the twenty fifth of December", "the 25th of December"),
                ("nineteen ninety nine", "1999"),
                ("twenty twenty four", "2024"),
                ("twenty twenty five", "2025"),
                ("nineteen eighty four", "1984"),
                ("nineteen oh five", "1905"),
                ("twenty ten", "2010"),
                ("eighteen sixty five", "1865"),
                ("born in nineteen ninety nine", "born in 1999"),
                ("March fourth, twenty twenty five", "March 4, 2025"),
            ],
        );
    }

    #[test]
    fn phone_numbers() {
        table(
            &on(),
            &[
                ("five five five one two three four", "5551234"),
                (
                    "call five five five one two three four five six seven",
                    "call 5551234567",
                ),
                // "oh" is a zero here and only here
                ("oh seven nine one two three four five six", "079123456"),
                ("zero two oh eight nine nine nine nine", "02089999"),
            ],
        );
    }

    #[test]
    fn version_strings() {
        table(
            &on(),
            &[
                ("version two point one", "version 2.1"),
                ("version two point one point three", "version 2.1.3"),
                ("version three point twelve", "version 3.12"),
                ("version two", "version 2"),
                ("upgrade to version four point oh", "upgrade to version 4.0"),
            ],
        );
    }

    // -- the ambiguity table ------------------------------------------------

    /// The other half of the contract, and the more important one: every case
    /// here has a reading in which converting would be wrong, so nothing is
    /// converted. Passing through is an acceptable answer; mangling is not.
    #[test]
    fn ambiguous_cases_pass_through_untouched() {
        unchanged(
            &on(),
            &[
                // "one" is not a quantity
                "one of the things",
                "one of these days",
                "no one saw it",
                "at one point I agreed",
                "one another",
                // below the threshold, so prose survives
                "the first of many",
                "at first I said no",
                "the second time",
                "a third of the team",
                "give me five",
                "chapter one",
                // "a quarter" is not always a time
                "a quarter of the budget",
                "a quarter of a mile",
                // "point" is not always a decimal. It is a bridge word: part
                // of a run only with a real number on *both* sides of it, and
                // never able to start one.
                "the point is",
                "that is beside the point",
                "at this point in time",
                "at this point one thing matters",
                "decimal point",
                "the decimal point moved",
                "move the decimal point two places",
                "point taken",
                "at that point five people left",
                "match point two nil",
                // "oh" is not always zero. It may never start a run, so it is
                // a digit only when a real number is already in progress.
                "oh, I see",
                "oh no",
                "oh and one more thing",
                "oh dear, oh dear",
                "oh well",
                "oh, one more thing",
                "double oh seven",
                "room two oh five",
                // a run that is not one number is left alone entirely
                "three four twenty twenty five",
                "one two three",
                "he counted one two three four five six",
                "three thirty",
                // proper nouns, spotted by the capital inside the run
                "Twenty One Pilots played",
                "Route Sixty Six",
                "Catch Twenty Two",
                // not a scale word, not a number
                "millions of dollars",
                "a hundredth of a second is fast",
                // words that only look like numbers
                "and then",
                "second to none",
            ],
        );
    }

    /// The pairs the issue names, asserted in **both** directions side by side.
    ///
    /// This is the most important test in the file. A pass-through table on its
    /// own proves nothing: a guard that declined everything would pass it, and
    /// so would a transform that was accidentally a no-op. Each row here is the
    /// same trigger word in two contexts, and exactly one side is allowed to
    /// move. Loosen a guard to fix one half and the other half fails.
    #[test]
    fn the_contrast_pairs_that_decide_it() {
        let t = on();
        // (stays as words, and what it stays as) | (converts, and what to)
        let pairs: &[(&str, &str, &str, &str)] = &[
            // "one" the pronoun vs "one" the count
            (
                "one of the things",
                "one of the things",
                "one hundred of the things",
                "100 of the things",
            ),
            // "a quarter" the fraction vs the clock
            (
                "a quarter of the budget",
                "a quarter of the budget",
                "a quarter past three",
                "3:15",
            ),
            // "the first of" a crowd vs a month
            (
                "the first of many",
                "the first of many",
                "the first of May",
                "the 1st of May",
            ),
            // "point" the noun vs the decimal separator
            (
                "the point is",
                "the point is",
                "three point one four",
                "3.14",
            ),
            (
                "move the decimal point two places",
                "move the decimal point two places",
                "zero point two five",
                "0.25",
            ),
            // "oh" the interjection vs the spoken zero
            ("oh, I see", "oh, I see", "nineteen oh five", "1905"),
            (
                "oh and one more thing",
                "oh and one more thing",
                "at three oh five",
                "at 3:05",
            ),
            // a bare number vs one wearing a unit. Both halves convert here,
            // and that is the point: "chapter 25" is correct, and the user who
            // wants "chapter twenty five" turns `cardinals` off — there is no
            // signal in the text that separates it from "twenty five widgets",
            // so it is a setting, never a guess.
            (
                "chapter twenty five",
                "chapter 25",
                "twenty five dollars",
                "$25",
            ),
            // "three thirty" is two numbers until something says it is a clock
            ("three thirty", "three thirty", "at three thirty", "at 3:30"),
            // a run that parses as one number vs one that does not
            (
                "three four twenty twenty five",
                "three four twenty twenty five",
                "in twenty twenty five",
                "in 2025",
            ),
            // "second" the rank vs "seconds" the unit
            (
                "the second time",
                "the second time",
                "the twenty second time",
                "the 22nd time",
            ),
            // "pounds" the weight keeps its word; "dollars" gets a symbol
            (
                "twenty five pounds of flour",
                "25 pounds of flour",
                "twenty five dollars of flour",
                "$25 of flour",
            ),
        ];
        for (stays, stays_want, moves, moves_want) in pairs {
            assert_eq!(&t.apply(stays), stays_want, "left half of {stays:?}");
            assert_eq!(&t.apply(moves), moves_want, "right half of {moves:?}");
            // Neither half may drift on a second pass.
            assert_eq!(t.apply(stays_want), *stays_want);
            assert_eq!(t.apply(moves_want), *moves_want);
        }
    }

    /// Two shapes collide with something rarer than a year, and the year wins.
    /// Recorded here rather than hidden: someone dictating the pun gets to see
    /// exactly what they will get, and a future change to the year rule has to
    /// walk past this test.
    #[test]
    fn year_shapes_beat_their_rarer_homographs() {
        let t = on();
        // "twenty twenty vision" is 20/20, not the year — but "in twenty
        // twenty" is far commoner in dictation than the pun, so the year rule
        // keeps it.
        assert_eq!(t.apply("twenty twenty vision"), "2020 vision");
        assert_eq!(t.apply("in twenty twenty"), "in 2020");
    }

    /// The pass-throughs above are not accidents of a disabled category: they
    /// hold with every category on, and they hold for the same reason each
    /// time. Spot-check the reason, not just the result.
    #[test]
    fn threshold_is_what_saves_small_words() {
        let t = with(|c| c.spell_out_below = 0);
        // With the guard removed the same inputs *do* convert, which is what
        // proves the guard is doing the work.
        assert_eq!(t.apply("one of the things"), "1 of the things");
        assert_eq!(t.apply("the first of many"), "the 1st of many");
        assert_eq!(t.apply("give me five"), "give me 5");
        // and the default puts them back
        unchanged(&on(), &["one of the things", "the first of many"]);
    }

    // -- boundaries ---------------------------------------------------------

    #[test]
    fn boundary_values() {
        table(
            &on(),
            &[
                // zero is below the default threshold, so it stays a word
                ("zero", "zero"),
                ("zero dollars", "$0"),
                ("zero percent", "0%"),
                // negatives: "negative" is a sign, "minus" is a preposition
                ("negative five", "-5"),
                ("negative twenty five", "-25"),
                ("negative twenty five percent", "-25%"),
                ("negative twenty five dollars", "-$25"),
                ("minus twenty five", "minus 25"),
                ("everyone minus five people", "everyone minus five people"),
                // "a hundred" and "one hundred" agree
                ("a hundred and one", "101"),
                ("one hundred and one", "101"),
                // hyphenated and British forms
                ("ninety-nine", "99"),
                ("two hundred and five", "205"),
                ("nine hundred and ninety nine thousand", "999,000"),
                // past u64: the parser is u128 and the grammar cannot reach
                // even that
                ("nineteen quintillion and one", "19,000,000,000,000,000,001"),
                ("nine hundred quintillion", "900 quintillion"),
            ],
        );
    }

    #[test]
    fn u64_boundary_is_not_a_boundary_here() {
        let t = on();
        // u64::MAX + 1, spoken out loud the long way. A u64 parser would wrap
        // or panic here; this one is u128 and the grammar tops out three orders
        // of magnitude below that, with checked arithmetic on the way.
        let spoken = "eighteen quintillion four hundred forty six quadrillion seven hundred \
                      forty four trillion seventy three billion seven hundred nine million five \
                      hundred fifty one thousand six hundred sixteen";
        assert_eq!(t.apply(spoken), "18,446,744,073,709,551,616");
        assert_eq!(grouped(u64::MAX as u128 + 1), "18,446,744,073,709,551,616");
    }

    #[test]
    fn nonsense_compositions_are_declined_whole() {
        unchanged(
            &on(),
            &[
                "twenty zero",
                "five twenty",
                "two thousand three thousand",
                "hundred",
                "thousand",
                "million people",
                "point five",
            ],
        );
    }

    // -- switches -----------------------------------------------------------

    #[test]
    fn disabled_is_byte_identical() {
        let off = Numbers::new(NumbersConfig::default());
        for input in torture_inputs() {
            assert_eq!(
                off.apply(&input),
                input,
                "disabled transform changed {:?}",
                truncate(&input)
            );
        }
        unchanged(
            &off,
            &[
                "twenty five percent",
                "twenty five dollars",
                "at three thirty",
                "version two point one",
                "five five five one two three four",
            ],
        );
    }

    #[test]
    fn every_category_off_is_byte_identical() {
        let t = with(|c| {
            c.cardinals = false;
            c.ordinals = false;
            c.decimals = false;
            c.percentages = false;
            c.currency = false;
            c.times = false;
            c.dates = false;
            c.phone_numbers = false;
            c.version_strings = false;
        });
        for input in torture_inputs() {
            assert_eq!(t.apply(&input), input);
        }
    }

    /// Turning one category off must move exactly one line of the table.
    #[test]
    fn categories_switch_independently() {
        /// input, what it becomes with everything on, and the switch that owns it
        type Probe = (&'static str, &'static str, fn(&mut NumbersConfig));
        let probes: &[Probe] = &[
            ("twenty five", "25", |c| c.cardinals = false),
            ("the twenty fifth time", "the 25th time", |c| {
                c.ordinals = false
            }),
            ("three point one four", "3.14", |c| c.decimals = false),
            ("twenty five percent", "25%", |c| c.percentages = false),
            ("twenty five dollars", "$25", |c| c.currency = false),
            ("at three thirty", "at 3:30", |c| c.times = false),
            ("nineteen ninety nine", "1999", |c| c.dates = false),
            ("five five five one two three four", "5551234", |c| {
                c.phone_numbers = false
            }),
            ("version two point one point three", "version 2.1.3", |c| {
                c.version_strings = false
            }),
        ];
        for (input, want_on, switch) in probes {
            assert_eq!(&on().apply(input), want_on, "with everything on: {input:?}");
            let off = with(switch);
            let got = off.apply(input);
            assert_ne!(
                &got, want_on,
                "switching the category off did nothing for {input:?}"
            );
            // Every *other* probe keeps working with that one category off.
            for (other, other_want, _) in probes {
                if other == input {
                    continue;
                }
                let got_other = off.apply(other);
                if got_other != *other_want {
                    // The only permitted interaction: a category that owns part
                    // of another line. Assert it is at least not mangled — the
                    // input comes back untouched.
                    assert_eq!(
                        &got_other, other,
                        "{input:?}'s switch mangled the unrelated case {other:?}"
                    );
                }
            }
        }
    }

    /// Currency and percentage do not need cardinals: a unit word is its own
    /// licence, which is the whole point of splitting the categories.
    #[test]
    fn unit_rules_survive_cardinals_being_off() {
        let t = with(|c| c.cardinals = false);
        assert_eq!(t.apply("twenty five percent"), "25%");
        assert_eq!(t.apply("twenty five dollars"), "$25");
        assert_eq!(t.apply("chapter twenty five"), "chapter twenty five");
    }

    /// Turning decimals off leaves "three point one four" fully spelled out
    /// rather than converting the halves either side of the "point": the run is
    /// one unit and declines as one.
    #[test]
    fn a_declined_decimal_does_not_leak_into_cardinals() {
        let t = with(|c| c.decimals = false);
        assert_eq!(t.apply("thirty point one four"), "thirty point one four");
        assert_eq!(
            t.apply("three point twenty five"),
            "three point twenty five"
        );
    }

    #[test]
    fn threshold_is_configurable() {
        let t = with(|c| c.spell_out_below = 100);
        assert_eq!(t.apply("twenty five"), "twenty five");
        assert_eq!(t.apply("a hundred and one"), "101");
        // a unit word still licenses digits below the threshold
        assert_eq!(t.apply("twenty five percent"), "25%");
        assert_eq!(t.apply("twenty five dollars"), "$25");
    }

    // -- adversarial --------------------------------------------------------

    #[test]
    fn adversarial_inputs() {
        table(
            &on(),
            &[
                // a number spanning a sentence boundary is two numbers
                (
                    "I have twenty. Twenty five people came.",
                    "I have 20. 25 people came.",
                ),
                (
                    "I have twenty five. Twenty five more.",
                    "I have 25. 25 more.",
                ),
                // terminal punctuation, immediately after the number
                ("give me twenty five!", "give me 25!"),
                ("twenty five?", "25?"),
                ("(twenty five)", "(25)"),
                ("twenty five...", "25..."),
                // a line break or a double space ends the number, so each
                // half is then a number of its own
                ("twenty\nfive", "20\nfive"),
                ("twenty  five", "20  five"),
                // digits and words already mixed
                ("25 twenty five", "25 25"),
                ("25 percent of twenty five", "25% of 25"),
                // a currency symbol the user spoke aloud as a symbol: #45 turns
                // "dollar sign" into "$", and this pass must not double it
                ("$25", "$25"),
                ("$twenty five", "$25"),
                ("25%", "25%"),
                ("$25.50", "$25.50"),
                ("3:15", "3:15"),
                ("007", "007"),
                ("the 25th", "the 25th"),
                // locale-ambiguous spoken date: nobody can tell 3 April from 4
                // March, so nothing moves
                (
                    "three four twenty twenty five",
                    "three four twenty twenty five",
                ),
                // quoted numbers still parse
                ("he said 'twenty five' out loud", "he said '25' out loud"),
                // A number the model wrote with a comma is one number, and it
                // is left alone: cutting it at the comma would let the "000"
                // claim the "percent" and produce "1,000%" via a route that
                // would also produce "3.$14" from "3.14 dollars".
                ("1,000 percent", "1,000 percent"),
                ("3.14 dollars", "3.14 dollars"),
                ("it costs 25.50 dollars", "it costs 25.50 dollars"),
            ],
        );
    }

    #[test]
    fn torture_corpus_is_survivable_and_idempotent() {
        let t = on();
        for input in torture_inputs() {
            let once = t.apply(&input);
            assert_eq!(
                t.apply(&once),
                once,
                "not idempotent on {:?}",
                truncate(&input)
            );
        }
        // the one torture line with numbers in it
        assert_eq!(
            t.apply("twenty five percent of one thousand"),
            "25% of 1000"
        );
        // and the ones without must come back byte-identical
        for input in [
            "naïve café résumé",
            "日本語のテキストです。",
            "Правда — это не то, что кажется",
            "👩‍💻 shipped it 🚀 👍🏽",
            "e\u{0301}gal",
            "\u{200b}zero\u{200b}width\u{200b}",
            "   leading and trailing   ",
            "trailing newline\n",
        ] {
            assert_eq!(t.apply(input), input, "changed {input:?}");
        }
    }

    /// A run longer than any real number is someone counting, and is left
    /// alone as a whole rather than converted in pieces.
    #[test]
    fn absurdly_long_runs_are_declined() {
        let t = on();
        let counting = "one two three four five six seven eight nine ten "
            .repeat(20)
            .trim_end()
            .to_string();
        // the phone rule owns runs of spoken digits; switch it off to see the
        // run-length guard on its own
        let t_nophone = with(|c| c.phone_numbers = false);
        assert_eq!(t_nophone.apply(&counting), counting);
        // with the phone rule on it becomes one long digit string, which is
        // the documented behaviour for seven or more spoken digits
        assert!(
            t.apply(&counting).starts_with("123456789 ten "),
            "{}",
            t.apply(&counting)
        );
    }

    /// Every idempotence bug the fuzz has found, pinned as a named case.
    ///
    /// All six are the same mistake: a rule decided what to do by looking at
    /// how a *neighbouring* number happened to be written, and the neighbour
    /// was written differently after the first pass had run. They are here in
    /// plain sight so the next person to touch a guard sees the shape of the
    /// trap rather than a fuzz seed.
    #[test]
    fn idempotence_regressions_found_by_the_fuzz() {
        let t = on();
        for (input, want) in [
            // the year rule refuses a sign, so "negative" must not merge into
            // one on the second pass either
            ("negative twenty nineteen", "negative 2019"),
            ("negative 2019", "negative 2019"),
            // a stranded unit word gets picked up by the next pass
            ("nineteen euros percent", "19 euros percent"),
            ("twenty thirty dollars", "twenty thirty dollars"),
            ("twelve euros per cent", "12 euros per cent"),
            // "3.14" is one number; cutting it at the dot produces "3.$14"
            ("3.14 dollars", "3.14 dollars"),
            ("1,000 percent", "1,000 percent"),
            ("3:30 percent", "3:30 percent"),
            // the currency decline has to see both spellings of "a number
            // follows": the words on pass one, the symbol on pass two
            ("one dollars ten dollars", "one dollars $10"),
            ("one cent twentieth", "one cent 20th"),
            // an hour is the whole number or nothing
            ("half past two million", "half past 2 million"),
            // the series predicate must agree with the scan's candidate cuts
            ("nineteen and five", "19 and 5"),
            ("ten dollars or five", "$10 or 5"),
        ] {
            let once = t.apply(input);
            assert_eq!(once, want, "input {input:?}");
            assert_eq!(t.apply(&once), once, "second pass moved {once:?}");
        }
    }

    // -- fuzz ---------------------------------------------------------------

    /// xorshift64*. Four lines beats a dependency, and a fixed seed means a
    /// failure here reproduces exactly rather than "sometimes on CI".
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
            &xs[(self.next() % xs.len() as u64) as usize]
        }
    }

    /// Deliberately weighted towards the parts that interact: bridges, unit
    /// words, the words that can start a run and the words that cannot.
    const FUZZ_WORDS: &[&str] = &[
        "one",
        "two",
        "five",
        "nine",
        "ten",
        "twelve",
        "nineteen",
        "twenty",
        "thirty",
        "ninety",
        "hundred",
        "thousand",
        "million",
        "quintillion",
        "and",
        "point",
        "oh",
        "a",
        "an",
        "first",
        "second",
        "third",
        "fifth",
        "twentieth",
        "hundredth",
        "percent",
        "per",
        "cent",
        "cents",
        "dollars",
        "pounds",
        "euros",
        "o'clock",
        "past",
        "to",
        "quarter",
        "half",
        "at",
        "it's",
        "am",
        "pm",
        "of",
        "the",
        "May",
        "March",
        "version",
        "negative",
        "minus",
        "sixties",
        "millions",
        "25",
        "007",
        "0",
        "1999",
        "%",
        "$",
        "the",
        "thing",
        "café",
        "日本",
        "👩‍💻",
    ];

    const FUZZ_SEPS: &[&str] = &[" ", " ", " ", "-", ", ", ". ", "\n", "  ", "! ", " — ", "'"];

    fn fuzz_input(rng: &mut Rng) -> String {
        let n = 1 + (rng.next() % 14) as usize;
        let mut s = String::new();
        for i in 0..n {
            if i > 0 {
                s.push_str(rng.pick(FUZZ_SEPS));
            }
            s.push_str(rng.pick(FUZZ_WORDS));
        }
        s
    }

    /// Idempotence on a corpus someone chose is a claim about that corpus. This
    /// is the same claim over 20k inputs nobody chose, built out of exactly the
    /// words the rules fight over.
    #[test]
    fn fuzz_is_idempotent_and_never_panics() {
        let t = on();
        let off = Numbers::new(NumbersConfig::default());
        let mut rng = Rng(0x5EED_1234_5678_9ABC);
        for _ in 0..20_000 {
            let input = fuzz_input(&mut rng);
            let once = t.apply(&input);
            let twice = t.apply(&once);
            assert_eq!(
                once, twice,
                "not idempotent: {input:?} -> {once:?} -> {twice:?}"
            );
            // and the disabled transform is still the identity on all of it
            assert_eq!(off.apply(&input), input, "disabled changed {input:?}");
        }
    }

    /// The same property at volume and across many seeds. Ignored so CI stays
    /// fast; run it before touching a rule:
    /// `cargo test -p wc-text --release -- --ignored deep_fuzz`.
    ///
    /// Every idempotence bug this module has had was found here first, and each
    /// one was a rule whose decision depended on how a *neighbouring* number
    /// happened to be written at the time it looked.
    #[test]
    #[ignore = "slow: run before changing the rules"]
    fn deep_fuzz() {
        let t = on();
        for seed in 1..=40u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            for _ in 0..50_000 {
                let input = fuzz_input(&mut rng);
                let once = t.apply(&input);
                assert_eq!(
                    t.apply(&once),
                    once,
                    "seed {seed}: not idempotent: {input:?} -> {once:?}"
                );
            }
        }
    }

    /// Every category on its own, over the same soup: a rule that only behaves
    /// because another rule got there first would show up here.
    #[test]
    fn fuzz_each_category_alone_is_idempotent() {
        let switches: &[fn(&mut NumbersConfig)] = &[
            |c| c.cardinals = true,
            |c| c.ordinals = true,
            |c| c.decimals = true,
            |c| c.percentages = true,
            |c| c.currency = true,
            |c| c.times = true,
            |c| c.dates = true,
            |c| c.phone_numbers = true,
            |c| c.version_strings = true,
        ];
        for switch in switches {
            let mut cfg = NumbersConfig {
                enabled: true,
                cardinals: false,
                ordinals: false,
                decimals: false,
                percentages: false,
                currency: false,
                times: false,
                dates: false,
                phone_numbers: false,
                version_strings: false,
                ..Default::default()
            };
            switch(&mut cfg);
            let t = Numbers::new(cfg);
            let mut rng = Rng(0xC0FF_EE00_1234_5678);
            for _ in 0..4_000 {
                let input = fuzz_input(&mut rng);
                let once = t.apply(&input);
                assert_eq!(
                    t.apply(&once),
                    once,
                    "not idempotent: {input:?} -> {once:?}"
                );
            }
        }
    }

    /// Every prefix of every fuzz input, cut at every character boundary. This
    /// is the shape `prefix_violation` uses, and it is where a byte-index slice
    /// that lands mid-character would panic.
    #[test]
    fn fuzz_every_prefix_is_safe() {
        let t = on();
        let mut rng = Rng(0xDEAD_BEEF_0BAD_F00D);
        for _ in 0..800 {
            let input = fuzz_input(&mut rng);
            for (i, _) in input.char_indices() {
                let out = t.apply(&input[..i]);
                assert_eq!(t.apply(&out), out, "prefix {:?} of {input:?}", &input[..i]);
            }
        }
    }

    // -- cost ---------------------------------------------------------------

    /// The torture corpus is 2 MB of *prose* — it never reaches the rules at
    /// all, so it would hide anything quadratic in the number machinery. These
    /// inputs are the slow paths: text that is almost entirely numbers, and a
    /// run made of nothing but bridge words, which is where the candidate loop
    /// does the most work per token.
    ///
    /// The bound is deliberately loose. It is not a latency budget — a real
    /// utterance is a few hundred bytes and takes microseconds — it is a
    /// tripwire for growth that is worse than linear, which would need minutes
    /// here rather than the tens of milliseconds it costs today.
    #[test]
    fn dense_number_input_does_not_go_quadratic() {
        use std::time::Instant;
        let t = on();

        let dense = "we spent twenty five thousand dollars on the twenty fifth of May at three \
                     thirty, up ninety nine point five percent from nineteen ninety nine. "
            .repeat(12_000); // ~1.5 MB, ~250k words, nearly all of them numbers
        let started = Instant::now();
        let out = t.apply(&dense);
        let dense_cost = started.elapsed();
        assert!(out.contains("$25,000"));
        assert!(
            dense_cost.as_secs() < 30,
            "1.5 MB of dense numbers took {dense_cost:?}"
        );

        // Every word a bridge: the worst case for the candidate loop, since
        // each run offers the most cut points a run is allowed to have.
        let bridges = "one and one and ".repeat(60_000);
        let started = Instant::now();
        let _ = t.apply(&bridges);
        let bridge_cost = started.elapsed();
        assert!(
            bridge_cost.as_secs() < 30,
            "a 1 MB chain of bridge words took {bridge_cost:?}"
        );

        // And the growth itself: sixteen times the input must not cost
        // anything like sixteen-squared the time.
        let small = "twenty five percent of one thousand and one. ".repeat(2_000);
        let big = "twenty five percent of one thousand and one. ".repeat(32_000);
        let started = Instant::now();
        let _ = t.apply(&small);
        let small_cost = started.elapsed().as_secs_f64();
        let started = Instant::now();
        let _ = t.apply(&big);
        let big_cost = started.elapsed().as_secs_f64();
        // 16x the input, allowed up to 64x the time before we call it a
        // regression. Quadratic would be 256x.
        assert!(
            big_cost < small_cost * 64.0 + 0.5,
            "16x the input cost {:.1}x the time ({small_cost:?} -> {big_cost:?})",
            big_cost / small_cost.max(f64::EPSILON)
        );
    }

    // -- streaming ----------------------------------------------------------

    /// `prefix_stable()` is a promise to the streaming loop (#50), and this
    /// transform cannot make it. Run the executable definition against the real
    /// implementation and pin the counterexample it finds.
    #[test]
    fn prefix_violation_is_real() {
        let t = on();
        let (prefix, polished_prefix, polished_whole) = prefix_violation(&t, "twenty five")
            .expect("a compound numeral must break the property");
        // The harness cuts at every character boundary, so the first prefix it
        // finds that breaks is one letter long: the streaming pass has typed
        // "t", and the finished utterance starts "2".
        assert_eq!(
            (
                prefix.as_str(),
                polished_prefix.as_str(),
                polished_whole.as_str()
            ),
            ("t", "t", "25"),
            "the streaming loop would have to retract a character it already typed"
        );
        // The instructive cut is at the word boundary, and it fails just as
        // hard: a finished "25" cannot start with an already-typed "20".
        assert_eq!(t.apply("twenty"), "20");
        assert_eq!(t.apply("twenty five"), "25");
        assert!(!"25".starts_with(&t.apply("twenty")));

        // Not a one-off: every shape this transform handles breaks it, because
        // the trigger for each one straddles the prefix boundary partway
        // through.
        for whole in [
            "twenty five",
            "twenty five percent",
            "twenty five dollars",
            "at three thirty",
            "nineteen ninety nine",
            "five five five one two three four",
        ] {
            assert!(
                prefix_violation(&t, whole).is_some(),
                "{whole:?} unexpectedly held the prefix property"
            );
        }
        assert!(!t.prefix_stable());
    }

    #[test]
    fn is_not_prefix_stable() {
        assert!(!Numbers::new(NumbersConfig::default()).prefix_stable());
    }

    // -- config -------------------------------------------------------------

    #[test]
    fn disabled_by_default() {
        assert!(!NumbersConfig::default().enabled);
    }

    /// The categories ship on, so switching the transform on does something
    /// useful without a second visit to the config file.
    #[test]
    fn categories_ship_on() {
        let d = NumbersConfig::default();
        assert!(d.any_category());
        assert_eq!(d.spell_out_below, 10);
    }

    #[test]
    fn default_config_has_nothing_to_report() {
        assert!(Numbers::new(NumbersConfig::default()).validate().is_empty());
        assert!(Numbers::new(NumbersConfig {
            enabled: true,
            ..Default::default()
        })
        .validate()
        .is_empty());
    }

    #[test]
    fn validate_flags_config_that_cannot_do_anything() {
        let all_off = NumbersConfig {
            cardinals: false,
            ordinals: false,
            decimals: false,
            percentages: false,
            currency: false,
            times: false,
            dates: false,
            phone_numbers: false,
            version_strings: false,
            ..Default::default()
        };
        let msgs = Numbers::new(all_off).validate();
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("every category is off"), "{msgs:?}");

        let silly = NumbersConfig {
            spell_out_below: 5000,
            ..Default::default()
        };
        let msgs = Numbers::new(silly).validate();
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("spell_out_below"), "{msgs:?}");
    }

    #[test]
    fn config_round_trips_through_toml() {
        let cfg = NumbersConfig {
            enabled: true,
            cardinals: false,
            spell_out_below: 21,
            ..Default::default()
        };
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: NumbersConfig = toml::from_str(&text).unwrap();
        assert_eq!(back, cfg);
    }

    /// A config written before this issue landed has none of the category keys,
    /// and must still load — with the categories at their defaults.
    #[test]
    fn older_config_still_loads() {
        let cfg: NumbersConfig = toml::from_str("enabled = true\n").unwrap();
        assert_eq!(
            cfg,
            NumbersConfig {
                enabled: true,
                ..Default::default()
            }
        );
    }

    /// Seventy-odd sentences of the kind people actually dictate, each one
    /// reviewed by hand. Half of them are here because the answer is "leave it
    /// alone" — this is the table that would catch a clever new rule quietly
    /// wrecking ordinary prose, and every line also runs the idempotence check.
    #[test]
    fn realistic_dictation() {
        table(
            &on(),
            &[
                ("half a million dollars", "half a million dollars"),
                ("one and a half", "one and a half"),
                ("three quarters of an hour", "three quarters of an hour"),
                ("a couple of hundred", "a couple of hundred"),
                ("he turned twenty one", "he turned 21"),
                ("nine eleven changed everything", "nine eleven changed everything"),
                ("it's open twenty four seven", "it's open twenty four seven"),
                ("flight two forty seven to Boston", "flight two forty seven to Boston"),
                ("room two oh five", "room two oh five"),
                ("on a scale of one to ten", "on a scale of 1 to 10"),
                ("one thousand and one nights", "1001 nights"),
                ("one hundred and one dalmatians", "101 dalmatians"),
                ("buy one get one free", "buy one get one free"),
                ("part one of three", "part one of three"),
                ("one on one", "one on one"),
                ("the big three", "the big three"),
                ("top ten", "top 10"),
                ("I need it by three", "I need it by three"),
                ("aged five to ten", "aged 5 to 10"),
                ("twenty five to thirty percent", "25 to 30%"),
                ("a hundred and ten percent", "110%"),
                ("October the twenty first", "October the 21st"),
                ("the fifth of November nineteen oh five", "the 5th of November 1905"),
                ("eight thirty in the morning", "eight thirty in the morning"),
                ("at ten to six", "at 10 to 6"),
                ("five dollars each", "$5 each"),
                ("twenty euros fifty", "20 euros 50"),
                ("we did it in two thousand and one", "we did it in 2001"),
                ("for the third time in five years", "for the third time in five years"),
                ("she said no one hundred times", "she said no 100 times"),
                ("on one hand it works", "on one hand it works"),
                ("point taken", "point taken"),
                ("two point oh", "2.0"),
                ("five hundred thousand", "500,000"),
                ("twenty twenty one was worse", "2021 was worse"),
                ("let's meet at four thirty on the twenty second", "let's meet at 4:30 on the 22nd"),
                ("the invoice came to three hundred and forty seven dollars and eighty two cents", "the invoice came to $347.82"),
                ("I ran five kilometres in twenty three minutes", "I ran five kilometres in 23 minutes"),
                ("she is twenty five years old and lives at number seventeen", "she is 25 years old and lives at number 17"),
                ("we shipped version one point two point three on May third", "we shipped version 1.2.3 on May 3"),
                ("call me on five five five two three four five six seven", "call me on 555234567"),
                ("about ninety percent of the time it just works", "about 90% of the time it just works"),
                ("there were one or two problems", "there were one or two problems"),
                ("I'll take the second one", "I'll take the second one"),
                ("it was one of those days", "it was one of those days"),
                ("give me one second", "give me one second"),
                ("the first thing to do is nothing", "the first thing to do is nothing"),
                ("a third of the budget went on rent", "a third of the budget went on rent"),
                ("he came in twenty third out of a hundred and four", "he came in 23rd out of 104"),
                ("the meeting is at nine", "the meeting is at nine"),
                ("the meeting is at nine thirty am", "the meeting is at 9:30 am"),
                ("it's half past eleven already", "it's 11:30 already"),
                ("temperatures hit minus twenty degrees", "temperatures hit minus 20 degrees"),
                ("temperatures hit negative twenty degrees", "temperatures hit -20 degrees"),
                ("we need twenty five thousand dollars by Friday", "we need $25,000 by Friday"),
                ("growth of two point five percent year on year", "growth of 2.5% year on year"),
                ("chapter twenty five, page three hundred and one", "chapter 25, page 301"),
                ("one two three testing", "one two three testing"),
                ("the nineteen sixties were different", "the nineteen sixties were different"),
                ("in two thousand and eight everything changed", "in 2008 everything changed"),
                ("she scored ninety nine out of a hundred", "she scored 99 out of 100"),
                ("press one for sales, two for support", "press one for sales, two for support"),
                ("the top three are all within one point five percent", "the top three are all within 1.5%"),
                ("twenty twenty was a long year", "2020 was a long year"),
                ("on the fourth of July", "on the 4th of July"),
                ("a quarter to midnight", "a quarter to midnight"),
                ("eight hundred and eighty eight", "888"),
                ("double oh seven", "double oh seven"),
                ("I said no a hundred times", "I said no 100 times"),
                ("two thirds of them agreed", "two thirds of them agreed"),
                ("we have three to five candidates", "we have three to five candidates"),
                ("between fifteen and twenty people", "between 15 and 20 people"),
                ("the score was nine to five", "the score was nine to five"),
            ],
        );
    }

    #[test]
    fn empty_string_and_whitespace() {
        let t = on();
        for input in ["", " ", "\n", "\t\n  \r\n"] {
            assert_eq!(t.apply(input), input);
        }
    }
}
