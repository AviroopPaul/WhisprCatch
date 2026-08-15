//! Custom dictionary — the user's names, jargon and acronyms, spelled the way
//! they spell them. The day-one papercut: the app gets your own name wrong.
//!
//! # Where the rules live
//!
//! Not in `config.toml`. A dictionary is something you share with a team or
//! check into a dotfiles repo, so it gets its own file, in the plainest format
//! that every spreadsheet and every text editor already understands:
//!
//! ```text
//! <config dir>/whisper-catch/dictionary.csv
//! ```
//!
//! which is `~/.config/whisper-catch/dictionary.csv` on Linux and
//! `~/Library/Application Support/whisper-catch/dictionary.csv` on macOS —
//! right next to the `config.toml` written by `apps/cli/src/config.rs`.
//! `[polish.dictionary] path = "..."` overrides it, so a team can point every
//! machine at one checked-out file. **A missing file is not an error**; it
//! means no rules.
//!
//! Choosing CSV as *the* format rather than adding a CSV importer next to a
//! native one is the whole point: bulk import is `cp`, and export is the file
//! itself.
//!
//! # Format
//!
//! ```csv
//! # lines starting with '#' are comments, blank lines are ignored
//! pattern,replacement          <- optional header, skipped if present
//! aviroop,Aviroop
//! wisper catch,WhisprCatch
//! get hub,GitHub
//! "hello, world","Hello, World"  <- quote a field to keep a comma in it
//! api,API,exact                <- optional third column: smart (default) or exact
//! ```
//!
//! Unquoted fields are trimmed. Quoted fields follow RFC 4180: `""` is a
//! literal quote and a newline inside quotes is part of the field.
//!
//! # Matching
//!
//! * **Word-boundary anchored.** A rule for `sam` rewrites `sam` and `Sam` but
//!   never `same`, `sames` or `flotsam`. A "word character" is anything
//!   `char::is_alphanumeric` accepts, plus `_`, plus combining marks — so the
//!   `gal` in `e` + U+0301 + `gal` is *not* a word start, because the word is
//!   `égal`.
//! * **Scripts without spaces are exempt.** A pattern that starts or ends in
//!   Han, Kana or Thai does not demand a boundary on that side, because there
//!   are no boundaries to demand: `日本語` matches inside `日本語のテキスト`.
//! * **Multi-word patterns** match across any run of whitespace, so
//!   `get hub` matches `get   hub` and `get\nhub`.
//! * **Nothing is a metacharacter.** `c++`, `.net` and `f#` are matched
//!   literally; there is no regex engine here and there never should be.
//! * **Leftmost, then longest, then first in the file.** One left-to-right
//!   pass; a replacement is never rescanned, so `a -> b` and `b -> c` do not
//!   chain.
//!
//! # Case
//!
//! Matching ignores case; the *output* case follows what the user actually
//! said, so one lowercase rule covers a whole sentence:
//!
//! | rule | input | output | why |
//! |---|---|---|---|
//! | `github -> GitHub` | `github` | `GitHub` | replacement verbatim |
//! | `wisper -> whispr` | `Wisper is fast` | `Whispr is fast` | sentence start keeps its capital |
//! | `sam -> Sam` | `SAM` | `SAM` | shouted text stays shouted — an intentional acronym survives |
//! | `wisper -> whispr` | `WISPER` | `WHISPR` | …and a real misspelling is still fixed |
//!
//! The acronym rule falls out of "all-caps in, all-caps out": a rule that only
//! fixes case reproduces its own input when uppercased. `exact` mode turns all
//! of this off and matches byte for byte.
//!
//! # Idempotence
//!
//! `apply(apply(x)) == apply(x)` holds for every dictionary [`validate`] calls
//! clean, and that is the contract. It cannot hold unconditionally: a
//! dictionary containing both `a -> b` and `b -> c` either chains (banned by
//! the issue) or is not idempotent, and there is no third answer. So the
//! chain is *reported*, not silently repaired — a dictionary is the user's
//! data, and quietly disabling one of their rules to satisfy a mathematical
//! property is worse than telling them about it.
//!
//! [`validate`]: Transform::validate

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Transform;

/// Directory under the platform config dir, shared with `config.toml`.
const APP_DIR: &str = "whisper-catch";
/// File name under [`APP_DIR`].
const FILE_NAME: &str = "dictionary.csv";

/// Where the dictionary comes from and whether it is switched on.
///
/// The rules themselves are deliberately *not* here: see the module docs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DictionaryConfig {
    /// Ships off. Product defaults for the cleanup tier are still an open
    /// decision, and every transform in this crate starts disabled so that
    /// output stays byte-identical to v0.4.0 until a user opts in.
    pub enabled: bool,
    /// Override for [`Dictionary::default_path`]. Point it at a file in a
    /// dotfiles repo to share one dictionary across machines.
    pub path: Option<PathBuf>,
}

/// How a rule's pattern is compared against the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MatchMode {
    /// Case-insensitive match, case-aware replacement. The default.
    Smart,
    /// Byte-for-byte match, replacement emitted verbatim. The escape hatch for
    /// a rule that must not touch an acronym.
    Exact,
}

/// One replacement rule, pre-chewed into the shape the scanner wants.
#[derive(Debug, Clone)]
struct Rule {
    /// Pattern with runs of whitespace normalised to one space. Used in
    /// messages and for duplicate detection, never for matching.
    pattern: String,
    /// `pattern` split on whitespace. Never empty.
    tokens: Vec<String>,
    replacement: String,
    mode: MatchMode,
    /// Line in the source file, for validation messages.
    line: usize,
    /// The match must not start in the middle of a word.
    needs_start_boundary: bool,
    /// The match must not end in the middle of a word.
    needs_end_boundary: bool,
}

/// Replaces what the model heard with what the user actually writes.
pub struct Dictionary {
    enabled: bool,
    rules: Vec<Rule>,
    /// Rules that must begin at a word start, keyed by the lowercased first
    /// word of the pattern. One hash lookup per word of the transcript is what
    /// keeps a 500-entry dictionary off the critical path.
    anchored: HashMap<String, Vec<usize>>,
    /// Longest key in `anchored`, in bytes. A word longer than this cannot
    /// start a match, which is what stops a 100k-character token from being
    /// lowercased into a key nobody will ever look up.
    max_key_len: usize,
    /// Rules that may begin anywhere: patterns starting with punctuation, and
    /// patterns touching a script that has no word boundaries. Keyed by the
    /// lowercased first character. Almost always empty, and skipped entirely
    /// when it is.
    floating: HashMap<char, Vec<usize>>,
    /// Problems found while reading and parsing, in file order.
    load_errors: Vec<String>,
    /// What to print in a validation message: a path, or `dictionary.csv`.
    source: String,
}

impl Dictionary {
    /// Reads the dictionary named by `cfg` (or [`default_path`] when it names
    /// nothing) and compiles it.
    ///
    /// This is the one place in `wc-text` that touches the disk, and it is
    /// deliberate: [`crate::Polish::from_config`] is the only constructor the
    /// pipeline calls, so loading here is what lets the rules live in their own
    /// file without a second wiring path through `apps/cli`. Everything after
    /// construction — [`Transform::apply`] above all — stays pure.
    ///
    /// A missing file means no rules. A file that exists but cannot be read is
    /// reported by [`Transform::validate`], not swallowed: silence there would
    /// look exactly like "my dictionary stopped working".
    ///
    /// [`default_path`]: Dictionary::default_path
    pub fn new(cfg: DictionaryConfig) -> Self {
        let Some(path) = cfg.path.clone().or_else(Self::default_path) else {
            // No HOME, no config dir, no dictionary. Not worth an error: this
            // is a daemon that has to keep dictating.
            return Self::empty(cfg.enabled, FILE_NAME.to_string());
        };
        let source = path.display().to_string();
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let mut d = Self::compile(&text, source);
                d.enabled = cfg.enabled;
                d
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::empty(cfg.enabled, source),
            Err(e) => {
                let mut d = Self::empty(cfg.enabled, source.clone());
                d.load_errors.push(format!("{source}: {e}"));
                d
            }
        }
    }

    /// Compiles a dictionary straight from CSV text, with no disk anywhere.
    ///
    /// The pure constructor: every test in this module uses it, and the
    /// Settings preview (#49) wants it to show the effect of an edit before it
    /// is saved.
    pub fn from_csv(csv: &str) -> Self {
        Self::compile(csv, FILE_NAME.to_string())
    }

    /// `<config dir>/whisper-catch/dictionary.csv`, or `None` when the platform
    /// will not say where its config lives.
    ///
    /// Mirrors `dirs::config_dir()` — which `apps/cli/src/config.rs` uses for
    /// `config.toml` — for the platforms WhisprCatch ships on, without adding a
    /// dependency to this crate: `$XDG_CONFIG_HOME` (absolute paths only) then
    /// `$HOME/.config` on Linux, `$HOME/Library/Application Support` on macOS,
    /// `%APPDATA%` on Windows. If those two ever disagree the dictionary lands
    /// somewhere the user's `config.toml` is not, so keep them in step.
    pub fn default_path() -> Option<PathBuf> {
        default_path_impl()
    }

    /// Number of active rules. For logs and for Settings (#49); rules dropped
    /// as malformed are not counted.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    fn empty(enabled: bool, source: String) -> Self {
        Self {
            enabled,
            rules: Vec::new(),
            anchored: HashMap::new(),
            max_key_len: 0,
            floating: HashMap::new(),
            load_errors: Vec::new(),
            source,
        }
    }

    fn compile(csv: &str, source: String) -> Self {
        let mut d = Self::empty(true, source);
        // Excel writes a byte-order mark, and "export from a spreadsheet" is
        // the whole point of choosing CSV. Without this the first pattern in
        // the file silently never matches.
        let records = parse_csv(csv.strip_prefix('\u{feff}').unwrap_or(csv));
        // (mode, comparison key) -> line that claimed it first
        let mut seen: HashMap<(MatchMode, String), usize> = HashMap::new();
        let mut first_record = true;

        for rec in records {
            let where_ = format!("{}:{}", d.source, rec.line);
            if rec.unterminated {
                d.load_errors.push(format!(
                    "{where_}: unterminated quote, rest of file ignored"
                ));
            }
            if first_record && is_header(&rec.fields) {
                first_record = false;
                continue;
            }
            first_record = false;

            if rec.fields.len() < 2 {
                d.load_errors.push(format!(
                    "{where_}: expected pattern,replacement but found {} column(s)",
                    rec.fields.len()
                ));
                continue;
            }
            if rec.fields.len() > 3 {
                d.load_errors.push(format!(
                    "{where_}: expected at most 3 columns (pattern,replacement,mode) but found {}; \
                     quote a field that contains a comma",
                    rec.fields.len()
                ));
                continue;
            }

            let raw_pattern = &rec.fields[0];
            let replacement = rec.fields[1].clone();
            let mode = match rec.fields.get(2).map(String::as_str).unwrap_or("") {
                "" | "smart" => MatchMode::Smart,
                "exact" => MatchMode::Exact,
                other if other.eq_ignore_ascii_case("smart") => MatchMode::Smart,
                other if other.eq_ignore_ascii_case("exact") => MatchMode::Exact,
                other => {
                    d.load_errors.push(format!(
                        "{where_}: unknown match mode {other:?}, expected \"smart\" or \"exact\""
                    ));
                    continue;
                }
            };

            let tokens: Vec<String> = raw_pattern.split_whitespace().map(str::to_string).collect();
            if tokens.is_empty() {
                d.load_errors.push(if raw_pattern.is_empty() {
                    format!("{where_}: empty pattern")
                } else {
                    format!("{where_}: pattern is only whitespace")
                });
                continue;
            }
            let pattern = tokens.join(" ");

            if pattern == replacement {
                d.load_errors.push(format!(
                    "{where_}: pattern and replacement are both {pattern:?}, so the rule has no \
                     effect and was dropped"
                ));
                continue;
            }

            let key = match mode {
                MatchMode::Smart => pattern.to_lowercase(),
                MatchMode::Exact => pattern.clone(),
            };
            if let Some(first) = seen.get(&(mode, key.clone())) {
                d.load_errors.push(format!(
                    "{where_}: duplicate pattern {pattern:?}, already defined on line {first}; \
                     the first rule wins and this one was dropped"
                ));
                continue;
            }
            seen.insert((mode, key), rec.line);

            let first_char = pattern.chars().next().expect("tokens are non-empty");
            let last_char = pattern.chars().next_back().expect("tokens are non-empty");
            d.rules.push(Rule {
                needs_start_boundary: is_word_char(first_char) && !is_boundaryless(first_char),
                needs_end_boundary: is_word_char(last_char) && !is_boundaryless(last_char),
                tokens,
                pattern,
                replacement,
                mode,
                line: rec.line,
            });
        }

        d.index();
        d
    }

    /// Builds the two lookup tables. Called once, at construction.
    fn index(&mut self) {
        for (i, rule) in self.rules.iter().enumerate() {
            let first = rule.pattern.chars().next().expect("tokens are non-empty");
            // A pattern touching a boundaryless script cannot be found by
            // whole-word lookup, because the "word" around it runs on for the
            // rest of the sentence. Those go in the scan-every-character
            // bucket along with patterns that open on punctuation.
            let anchored = rule.needs_start_boundary && !rule.pattern.chars().any(is_boundaryless);
            if anchored {
                let key: String = rule
                    .pattern
                    .chars()
                    .take_while(|c| is_word_char(*c))
                    .flat_map(char::to_lowercase)
                    .collect();
                self.max_key_len = self.max_key_len.max(key.len());
                self.anchored.entry(key).or_default().push(i);
            } else {
                self.floating.entry(lc_first(first)).or_default().push(i);
            }
        }
    }

    /// One left-to-right pass. Pure, allocation-free until something matches.
    fn run(&self, text: &str) -> String {
        if self.rules.is_empty() || text.is_empty() {
            return text.to_string();
        }
        let mut out = String::new();
        let mut key_buf = String::new();
        let mut copied = 0usize;
        let mut pos = 0usize;
        let mut prev_word = false;
        let mut hit = false;

        let bytes = text.as_bytes();
        while pos < text.len() {
            // ASCII is one byte and needs no decoding, which matters because
            // this loop visits every character of the transcript.
            let c = match bytes[pos] {
                b if b < 0x80 => b as char,
                _ => text[pos..].chars().next().expect("pos is a char boundary"),
            };
            let cw = is_word_char(c);
            let mut best: Option<(usize, usize)> = None;

            if cw && !prev_word && !self.anchored.is_empty() {
                if let Some(end) = word_run_end(text, pos, self.max_key_len) {
                    let word = &text[pos..end];
                    let key: &str =
                        if word.is_ascii() && !word.bytes().any(|b| b.is_ascii_uppercase()) {
                            word
                        } else {
                            key_buf.clear();
                            key_buf.extend(word.chars().flat_map(char::to_lowercase));
                            key_buf.as_str()
                        };
                    if let Some(ids) = self.anchored.get(key) {
                        best = self.best_match(text, pos, ids, prev_word);
                    }
                }
            }
            if best.is_none() && !self.floating.is_empty() {
                if let Some(ids) = self.floating.get(&lc_first(c)) {
                    best = self.best_match(text, pos, ids, prev_word);
                }
            }

            match best {
                Some((len, idx)) => {
                    let rule = &self.rules[idx];
                    if !hit {
                        out.reserve(text.len() + rule.replacement.len());
                        hit = true;
                    }
                    out.push_str(&text[copied..pos]);
                    push_replacement(&mut out, &text[pos..pos + len], rule);
                    copied = pos + len;
                    pos = copied;
                    prev_word = text[..copied].chars().next_back().is_some_and(is_word_char);
                }
                None => {
                    pos += c.len_utf8();
                    prev_word = cw;
                }
            }
        }

        if !hit {
            return text.to_string();
        }
        out.push_str(&text[copied..]);
        out
    }

    /// Longest match at `pos` among `ids`. Two rules of equal length are
    /// decided by mode and then by position in the file: an `exact` rule beats
    /// a `smart` one, because someone who spelled out the casing meant it, and
    /// otherwise the earlier line wins.
    fn best_match(
        &self,
        text: &str,
        pos: usize,
        ids: &[usize],
        prev_word: bool,
    ) -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize)> = None;
        for &i in ids {
            let Some(len) = match_len(&self.rules[i], text, pos, prev_word) else {
                continue;
            };
            let better = match best {
                None => true,
                Some((b, j)) => {
                    len > b
                        || (len == b
                            && self.rules[i].mode == MatchMode::Exact
                            && self.rules[j].mode != MatchMode::Exact)
                }
            };
            if better {
                best = Some((len, i));
            }
        }
        best
    }
}

impl Transform for Dictionary {
    fn name(&self) -> &'static str {
        "dictionary"
    }

    /// Deterministic and side-effect free: the file was read once, in
    /// [`Dictionary::new`], and nothing here touches the world.
    fn apply(&self, text: &str) -> String {
        if !self.enabled {
            return text.to_string();
        }
        self.run(text)
    }

    /// Not prefix-stable, and this is the measured answer rather than the
    /// cautious one: `prefix_violation_finds_a_real_counterexample` runs
    /// `testing::prefix_violation` against the real implementation.
    ///
    /// With the entry "push to get" -> "push to GitHub" and the utterance
    /// "push to get now", the first prefix that breaks is `"push to g"`. A
    /// streaming pass that has heard that much types a lower-case `g`, and the
    /// finished utterance polishes to `"push to GitHub now"`, which does not
    /// start with it. The seam's original example — the whole trigger, `"push
    /// to get"` — is in fact *not* a violation: `apply` of it is `"push to
    /// GitHub"`, which the finished text does start with. The property breaks
    /// while a trigger is still being spoken, not once it is complete, and it
    /// breaks a full word earlier than "the trigger straddles the boundary"
    /// suggests. `sam -> Sam` is enough on its own: `apply("s")` is `"s"` and
    /// `apply("sam")` is `"Sam"`.
    fn prefix_stable(&self) -> bool {
        false
    }

    /// Everything wrong with the user's file, in file order, and then the two
    /// hazards that only exist between rules.
    ///
    /// Deliberately independent of `enabled`: someone editing a dictionary they
    /// have not switched on still deserves to be told line 12 is broken.
    ///
    /// The between-rules half is quadratic and costs about 4 ms on a 500-entry
    /// dictionary. That is a Settings-save budget, not a keystroke budget —
    /// #49 should call this on save or on a debounce, never per character.
    fn validate(&self) -> Vec<String> {
        let mut msgs = self.load_errors.clone();
        self.chain_hazards(&mut msgs);
        msgs.dedup();
        msgs
    }
}

impl Dictionary {
    /// The two ways a rule set stops being idempotent, both checked by effect
    /// rather than by shape so that the overwhelmingly common `github ->
    /// GitHub` next to `get hub -> GitHub` is not flagged for containing its
    /// own pattern.
    fn chain_hazards(&self, msgs: &mut Vec<String>) {
        // 1. A replacement that some rule rewrites: a -> b next to b -> c.
        for rule in &self.rules {
            if rule.replacement.is_empty() {
                continue;
            }
            let again = self.run(&rule.replacement);
            if again != rule.replacement {
                msgs.push(format!(
                    "{}:{}: the replacement {:?} is itself rewritten (to {:?}) by another rule, so \
                     applying the dictionary twice would not give the same text; drop one of the \
                     two rules",
                    self.source, rule.line, rule.replacement, again
                ));
            }
        }

        // 2. A multi-word pattern that only appears once a replacement is
        //    already in place: "foo" -> "bar baz" next to "baz qux" -> "Z".
        //    Split once per rule rather than once per pair: the inner loop runs
        //    a hundred thousand times on a full dictionary.
        let replacement_words: Vec<Vec<&str>> = self
            .rules
            .iter()
            .map(|r| r.replacement.split_whitespace().collect())
            .collect();
        for rule in &self.rules {
            if rule.tokens.len() < 2 {
                continue;
            }
            for (other, words) in self.rules.iter().zip(&replacement_words) {
                if words.is_empty() {
                    continue;
                }
                let n = rule.tokens.len();
                for k in 1..n {
                    if k > words.len() {
                        break;
                    }
                    let head_meets_tail = eq_words(&rule.tokens[..k], &words[words.len() - k..]);
                    let tail_meets_head = eq_words(&rule.tokens[n - k..], &words[..k]);
                    let probe = if head_meets_tail {
                        format!("{} {}", other.replacement, rule.tokens[k..].join(" "))
                    } else if tail_meets_head {
                        format!("{} {}", rule.tokens[..n - k].join(" "), other.replacement)
                    } else {
                        continue;
                    };
                    if self.run(&probe) != probe {
                        msgs.push(format!(
                            "{}:{}: pattern {:?} can be formed by the rule on line {} ({:?} -> \
                             {:?}), so applying the dictionary twice would not give the same text",
                            self.source,
                            rule.line,
                            rule.pattern,
                            other.line,
                            other.pattern,
                            other.replacement
                        ));
                    }
                }
            }
        }
    }
}

// ---- matching ------------------------------------------------------------

/// Byte length of `rule`'s match in `text` at `at`, or `None`.
fn match_len(rule: &Rule, text: &str, at: usize, prev_word: bool) -> Option<usize> {
    if rule.needs_start_boundary && prev_word {
        return None;
    }
    let mut pos = at;
    for (i, token) in rule.tokens.iter().enumerate() {
        if i > 0 {
            // At least one whitespace character between words, however many
            // the speaker's pauses turned into.
            let gap: usize = text[pos..]
                .chars()
                .take_while(|c| c.is_whitespace())
                .map(char::len_utf8)
                .sum();
            if gap == 0 {
                return None;
            }
            pos += gap;
        }
        let len = match rule.mode {
            MatchMode::Exact => {
                if text[pos..].starts_with(token.as_str()) {
                    token.len()
                } else {
                    return None;
                }
            }
            MatchMode::Smart => ci_prefix_len(&text[pos..], token)?,
        };
        pos += len;
    }
    if rule.needs_end_boundary && text[pos..].chars().next().is_some_and(is_word_char) {
        return None;
    }
    Some(pos - at)
}

/// Byte length of the prefix of `hay` whose simple lowercase mapping equals
/// `needle`'s, or `None` if there is no such prefix.
///
/// Compares the two lowercase *streams*, so a character that lowercases to
/// more than one (`İ` -> `i` + U+0307) is handled, and a match that would end
/// halfway through such an expansion is rejected rather than reported as a
/// match of the wrong length. Allocation-free.
fn ci_prefix_len(hay: &str, needle: &str) -> Option<usize> {
    let mut want = needle.chars().flat_map(char::to_lowercase);
    let mut next = want.next();
    for (i, c) in hay.char_indices() {
        if next.is_none() {
            return Some(i);
        }
        for lc in c.to_lowercase() {
            match next {
                Some(w) if w == lc => next = want.next(),
                _ => return None,
            }
        }
    }
    if next.is_none() {
        Some(hay.len())
    } else {
        None
    }
}

/// End of the run of word characters starting at `at`, or `None` when that run
/// is longer than `cap` bytes and therefore longer than any pattern's first
/// word. The cap is what keeps a single 100k-character token cheap.
fn word_run_end(text: &str, at: usize, cap: usize) -> Option<usize> {
    let mut end = 0usize;
    for (i, c) in text[at..].char_indices() {
        if !is_word_char(c) {
            return Some(at + i);
        }
        end = i + c.len_utf8();
        if end > cap {
            return None;
        }
    }
    Some(at + end)
}

// ---- case ----------------------------------------------------------------

/// Appends `rule`'s replacement, cased to match what the speaker actually said.
fn push_replacement(out: &mut String, matched: &str, rule: &Rule) {
    if rule.mode == MatchMode::Exact {
        out.push_str(&rule.replacement);
        return;
    }
    let mut upper = 0usize;
    let mut lower = 0usize;
    let mut first_cased_is_upper = None;
    for c in matched.chars() {
        if c.is_uppercase() {
            upper += 1;
            first_cased_is_upper.get_or_insert(true);
        } else if c.is_lowercase() {
            lower += 1;
            first_cased_is_upper.get_or_insert(false);
        }
    }

    if upper >= 2 && lower == 0 {
        // Shouted. Uppercasing the replacement leaves a case-only rule's own
        // input untouched, which is exactly what saves an intentional acronym:
        // `sam -> Sam` applied to `SAM` gives `SAM` back.
        out.push_str(&rule.replacement.to_uppercase());
        return;
    }
    if first_cased_is_upper == Some(true) && upper == 1 {
        // Capitalised, most often because it opened a sentence. Carry that on
        // to a replacement that does not already start with a capital.
        let mut chars = rule.replacement.chars();
        if let Some(first) = chars.next() {
            if first.is_lowercase() {
                out.extend(first.to_uppercase());
                out.push_str(chars.as_str());
                return;
            }
        }
    }
    out.push_str(&rule.replacement);
}

// ---- character classes ---------------------------------------------------

/// Word characters, for the boundary that stops `sam` rewriting `same`.
///
/// Combining marks count, or the `gal` in a decomposed `égal` would look like
/// the start of a word. The ASCII arm is not premature: this runs once per
/// character of every transcript, and English text never leaves it.
fn is_word_char(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric() || c == '_';
    }
    c.is_alphanumeric() || is_combining_mark(c)
}

fn is_combining_mark(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F   // combining diacritical marks
        | 0x0483..=0x0489 // Cyrillic
        | 0x0591..=0x05BD // Hebrew points
        | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 // Arabic
        | 0x0900..=0x0903 | 0x093A..=0x094F // Devanagari
        | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E // Thai
        | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF // extensions
        | 0x20D0..=0x20F0 // combining marks for symbols
        | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F) // variation selectors, half marks
}

/// Scripts written without spaces, where demanding a word boundary would mean
/// a rule could never fire.
fn is_boundaryless(c: char) -> bool {
    matches!(c as u32,
        0x0E00..=0x0E7F      // Thai
        | 0x3040..=0x30FF    // Hiragana, Katakana
        | 0x3400..=0x4DBF    // CJK unified ideographs extension A
        | 0x4E00..=0x9FFF    // CJK unified ideographs
        | 0xF900..=0xFAFF    // CJK compatibility ideographs
        | 0x20000..=0x2FA1F) // CJK extensions B and later
}

/// First character of `c`'s simple lowercase mapping, without allocating.
fn lc_first(c: char) -> char {
    if c.is_ascii() {
        c.to_ascii_lowercase()
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

/// Case-insensitive equality of two word lists. Allocation-free, because
/// `chain_hazards` calls it once per (multi-word rule, replacement) pair and a
/// 500-entry dictionary makes that six figures.
fn eq_words(a: &[String], b: &[&str]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| ci_prefix_len(y, x) == Some(y.len()))
}

// ---- CSV -----------------------------------------------------------------

struct CsvRecord {
    /// Physical line the record started on, 1-based.
    line: usize,
    fields: Vec<String>,
    unterminated: bool,
}

/// True when a record looks like the header row rather than a rule.
fn is_header(fields: &[String]) -> bool {
    fields.len() >= 2
        && fields[0].eq_ignore_ascii_case("pattern")
        && fields[1].eq_ignore_ascii_case("replacement")
}

/// RFC 4180 with two house rules: a record whose first non-blank character is
/// `#` is a comment, and unquoted fields are trimmed.
fn parse_csv(text: &str) -> Vec<CsvRecord> {
    let mut records = Vec::new();
    let mut chars = text.chars().peekable();
    let mut line = 1usize;

    'records: loop {
        // Blank lines, indentation and comments between records.
        loop {
            match chars.peek().copied() {
                Some('\n') => {
                    chars.next();
                    line += 1;
                }
                Some('\r') | Some(' ') | Some('\t') => {
                    chars.next();
                }
                Some('#') => {
                    for c in chars.by_ref() {
                        if c == '\n' {
                            line += 1;
                            break;
                        }
                    }
                }
                Some(_) => break,
                None => break 'records,
            }
        }

        let start_line = line;
        let mut fields: Vec<String> = Vec::new();
        let mut field = String::new();
        let mut quoted = false;
        let mut was_quoted = false;
        let mut unterminated = false;

        loop {
            let Some(c) = chars.next() else {
                unterminated = quoted;
                fields.push(finish_field(field, was_quoted));
                break;
            };
            if quoted {
                match c {
                    '"' if chars.peek() == Some(&'"') => {
                        chars.next();
                        field.push('"');
                    }
                    '"' => quoted = false,
                    '\n' => {
                        line += 1;
                        field.push(c);
                    }
                    _ => field.push(c),
                }
                continue;
            }
            match c {
                '"' if !was_quoted && field.chars().all(char::is_whitespace) => {
                    quoted = true;
                    was_quoted = true;
                    field.clear();
                }
                ',' => {
                    fields.push(finish_field(std::mem::take(&mut field), was_quoted));
                    was_quoted = false;
                }
                '\r' => {}
                '\n' => {
                    line += 1;
                    fields.push(finish_field(field, was_quoted));
                    break;
                }
                // Whitespace after a closing quote is padding, not content.
                _ if was_quoted && c.is_whitespace() => {}
                _ => field.push(c),
            }
        }
        records.push(CsvRecord {
            line: start_line,
            fields,
            unterminated,
        });
        if unterminated {
            break;
        }
    }
    records
}

fn finish_field(field: String, was_quoted: bool) -> String {
    if was_quoted {
        field
    } else {
        field.trim().to_string()
    }
}

// ---- where the file lives ------------------------------------------------

/// The crate's own tests must never read a contributor's real dictionary:
/// `PolishConfig::validate` in `lib.rs` builds a `Dictionary` from a default
/// config, so a stray `~/.config/whisper-catch/dictionary.csv` would otherwise
/// decide whether tests in a file this issue is not allowed to touch pass.
/// Tests that need a file on disk pass an explicit `path`.
#[cfg(test)]
fn default_path_impl() -> Option<PathBuf> {
    None
}

#[cfg(not(test))]
fn default_path_impl() -> Option<PathBuf> {
    config_dir(|k| std::env::var_os(k)).map(|d| d.join(APP_DIR).join(FILE_NAME))
}

/// `dirs::config_dir()` for the platforms WhisprCatch ships on, reading the
/// environment through `vars` so the rules can be tested without touching the
/// process environment.
fn config_dir(vars: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let home = || {
        vars("HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
    };
    if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library").join("Application Support"))
    } else if cfg!(target_os = "windows") {
        vars("APPDATA")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
    } else {
        vars("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| Path::new(p).is_absolute())
            .or_else(|| home().map(|h| h.join(".config")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{prefix_violation, torture_inputs, truncate};

    /// Shorthand: compile a dictionary from inline CSV.
    fn dict(csv: &str) -> Dictionary {
        Dictionary::from_csv(csv)
    }

    /// Shorthand: run one dictionary over one string.
    fn go(csv: &str, input: &str) -> String {
        dict(csv).apply(input)
    }

    /// The torture corpus minus its three multi-hundred-kilobyte entries.
    ///
    /// The sweeps below run a dozen dictionaries over the whole corpus, and an
    /// unoptimised build spends about a second on each pass over the 2 MB
    /// input. Splitting the giants out keeps every one of those cases and
    /// takes about half a minute off `cargo test`;
    /// `the_giant_torture_inputs_are_cheap_and_idempotent` walks them once,
    /// with the largest dictionary of the lot.
    fn small_torture_inputs() -> Vec<String> {
        torture_inputs()
            .into_iter()
            .filter(|s| s.len() < 4096)
            .collect()
    }

    fn giant_torture_inputs() -> Vec<String> {
        let giants: Vec<String> = torture_inputs()
            .into_iter()
            .filter(|s| s.len() >= 4096)
            .collect();
        assert_eq!(giants.len(), 3, "the corpus grew a large input");
        giants
    }

    /// A scratch directory that cleans itself up. Enough for the handful of
    /// tests that need a real file, and cheaper than a dev-dependency.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let mut p = std::env::temp_dir();
            let n = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            p.push(format!("wc-text-dict-{tag}-{n}"));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn file(&self, name: &str, body: &str) -> PathBuf {
            let p = self.0.join(name);
            std::fs::write(&p, body).unwrap();
            p
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // ---- the headline bug -------------------------------------------------

    /// The single most likely bug in the whole transform, named in the issue.
    #[test]
    fn a_rule_for_sam_does_not_rewrite_same() {
        let csv = "sam,Sam\n";
        assert_eq!(go(csv, "sam"), "Sam");
        assert_eq!(go(csv, "same"), "same");
        assert_eq!(go(csv, "sames"), "sames");
        assert_eq!(go(csv, "flotsam"), "flotsam");
        assert_eq!(go(csv, "sam's laptop"), "Sam's laptop");
        assert_eq!(go(csv, "ask sam, then go"), "ask Sam, then go");
        assert_eq!(go(csv, "sam-adjacent"), "Sam-adjacent");
        assert_eq!(go(csv, "samsam"), "samsam");
        assert_eq!(go(csv, "sam sam"), "Sam Sam");
        assert_eq!(go(csv, "_sam"), "_sam", "underscore is a word character");
        assert_eq!(go(csv, "sam2"), "sam2", "digits are word characters");
    }

    #[test]
    fn boundaries_hold_at_the_ends_of_the_input() {
        let csv = "sam,Sam\n";
        assert_eq!(go(csv, "sam"), "Sam");
        assert_eq!(go(csv, " sam"), " Sam");
        assert_eq!(go(csv, "sam "), "Sam ");
        assert_eq!(go(csv, "\nsam\n"), "\nSam\n");
        assert_eq!(go(csv, "(sam)"), "(Sam)");
    }

    /// Combining marks are word characters, or the `gal` in a decomposed
    /// `égal` looks like the start of a word.
    #[test]
    fn combining_marks_do_not_open_a_word() {
        assert_eq!(go("gal,GAL\n", "e\u{0301}gal"), "e\u{0301}gal");
        assert_eq!(go("gal,GAL\n", "égal"), "égal", "precomposed too");
        assert_eq!(go("gal,GAL\n", "gal"), "GAL");
        // ...and a mark at the end of the word blocks the closing boundary
        assert_eq!(go("cafe,café\n", "cafe\u{0301}"), "cafe\u{0301}");
    }

    #[test]
    fn zero_width_space_separates_words() {
        // U+200B is not alphanumeric, so it is a boundary the same as a space
        assert_eq!(
            go("zero,ZERO\n", "\u{200b}zero\u{200b}width\u{200b}"),
            "\u{200b}ZERO\u{200b}width\u{200b}"
        );
        assert_eq!(
            go("zerowidth,X\n", "\u{200b}zero\u{200b}width\u{200b}"),
            "\u{200b}zero\u{200b}width\u{200b}"
        );
    }

    #[test]
    fn emoji_are_left_alone() {
        let csv = "shipped,shipped it twice\n";
        assert_eq!(go(csv, "👩‍💻 shipped 🚀 👍🏽"), "👩‍💻 shipped it twice 🚀 👍🏽");
        assert_eq!(go("🚀,rocket\n", "👩‍💻 shipped 🚀"), "👩‍💻 shipped rocket");
    }

    // ---- word boundaries in scripts that have none ------------------------

    #[test]
    fn cjk_patterns_match_without_word_boundaries() {
        assert_eq!(
            go("日本語,ニホンゴ\n", "日本語のテキストです。"),
            "ニホンゴのテキストです。"
        );
        assert_eq!(
            go("テキスト,text\n", "日本語のテキストです。"),
            "日本語のtextです。"
        );
        // and a Latin pattern still does not match inside a CJK run
        assert_eq!(go("のテ,X\n", "日本語のテキスト"), "日本語Xキスト");
    }

    /// A pattern that mixes scripts wants a boundary on the Latin side and
    /// none on the CJK side, which is the one case that needs both code paths
    /// at once.
    #[test]
    fn a_mixed_script_pattern_asks_for_a_boundary_only_where_one_exists() {
        let d = dict("sam日,SamJP\n");
        assert_eq!(d.apply("sam日"), "SamJP");
        assert_eq!(d.apply("sam日本語"), "SamJP本語", "no boundary after 日");
        assert_eq!(d.apply("flotsam日"), "flotsam日", "boundary before sam");
        let d = dict("日text,JP\n");
        assert_eq!(d.apply("本日text"), "本JP", "no boundary before 日");
        assert_eq!(d.apply("日texture"), "日texture", "boundary after text");
    }

    #[test]
    fn cyrillic_words_respect_boundaries() {
        let csv = "правда,Правда\n";
        assert_eq!(
            go(csv, "Правда — это не то, что кажется"),
            "Правда — это не то, что кажется"
        );
        assert_eq!(go(csv, "правда важна"), "Правда важна");
        assert_eq!(go(csv, "неправда"), "неправда");
    }

    // ---- multi-word patterns ---------------------------------------------

    #[test]
    fn multi_word_patterns_span_any_whitespace() {
        let csv = "get hub,GitHub\n";
        assert_eq!(go(csv, "push to get hub now"), "push to GitHub now");
        assert_eq!(go(csv, "get  hub"), "GitHub");
        assert_eq!(go(csv, "get\thub"), "GitHub");
        assert_eq!(go(csv, "get\nhub"), "GitHub");
        assert_eq!(go(csv, "gethub"), "gethub", "a gap is required");
        assert_eq!(go(csv, "forget hub"), "forget hub", "boundary at the start");
        assert_eq!(go(csv, "get hubs"), "get hubs", "boundary at the end");
    }

    #[test]
    fn a_pattern_longer_than_the_input_matches_nothing() {
        assert_eq!(go("a very long phrase indeed,X\n", "a very"), "a very");
        assert_eq!(go("aviroop,Aviroop\n", "av"), "av");
        assert_eq!(go("aviroop,Aviroop\n", ""), "");
    }

    // ---- adversarial rule sets -------------------------------------------

    #[test]
    fn nothing_in_a_pattern_is_a_metacharacter() {
        assert_eq!(go("c++,C++\n", "i write c++ daily"), "i write C++ daily");
        assert_eq!(go(".net,.NET\n", "on .net today"), "on .NET today");
        assert_eq!(go("a.b,X\n", "a.b"), "X");
        assert_eq!(go("a.b,X\n", "aXb"), "aXb", ". is not any character");
        assert_eq!(go("a*b,X\n", "a*b"), "X");
        assert_eq!(go("a*b,X\n", "aab"), "aab", "* is not a repeat");
        assert_eq!(go("f#,F#\n", "f# is fine"), "F# is fine");
        assert_eq!(go("\\d,digit\n", "1"), "1", "\\d is not a digit class");
        assert_eq!(go("\\d,digit\n", "\\d"), "digit");
        assert_eq!(go("^start$,X\n", "^start$"), "X");
        assert_eq!(go("(a|b),X\n", "a"), "a");
        assert_eq!(go("(a|b),X\n", "(a|b)"), "X");
    }

    #[test]
    fn the_longest_match_wins_then_the_earlier_line() {
        let csv = "new,NEW\nnew york,NYC\nnew york city,NY\n";
        assert_eq!(go(csv, "new york city"), "NY");
        assert_eq!(go(csv, "new york"), "NYC");
        assert_eq!(go(csv, "new jersey"), "NEW jersey");
        // same length, two rules: the first line in the file wins
        let tie = "aa bb,FIRST\naa BB,SECOND\n";
        assert_eq!(go(tie, "aa bb"), "FIRST");
    }

    #[test]
    fn overlapping_matches_are_consumed_once() {
        // "b c" cannot fire, because "a b" already consumed the b
        let csv = "a b,X\nb c,Y\n";
        assert_eq!(go(csv, "a b c"), "X c");
        assert_eq!(go(csv, "b c"), "Y");
        // and a replacement is never rescanned inside the same pass
        assert_eq!(go("aa,aa aa\n", "aa"), "aa aa");
    }

    #[test]
    fn a_replacement_containing_another_pattern_does_not_chain() {
        // the case the issue calls out: a -> b next to b -> c
        let csv = "alpha,beta\nbeta,gamma\n";
        assert_eq!(go(csv, "alpha"), "beta", "one pass only");
        assert_eq!(go(csv, "beta"), "gamma");
        assert_eq!(go(csv, "alpha beta"), "beta gamma");
    }

    /// The documented cost of "no chaining": such a dictionary is not
    /// idempotent, and `validate` says so rather than the code silently
    /// deleting one of the user's rules.
    #[test]
    fn a_chaining_dictionary_is_reported_not_repaired() {
        let d = dict("alpha,beta\nbeta,gamma\n");
        let once = d.apply("alpha");
        assert_eq!(once, "beta");
        assert_eq!(d.apply(&once), "gamma", "the documented exception");

        let msgs = d.validate();
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("dictionary.csv:1"), "{msgs:?}");
        assert!(msgs[0].contains("\"beta\""), "{msgs:?}");
        assert!(msgs[0].contains("twice"), "{msgs:?}");
    }

    /// The other, subtler shape: the pattern only exists once a replacement is
    /// in place, so no replacement literally contains it.
    #[test]
    fn a_pattern_formed_across_a_replacement_boundary_is_reported() {
        let d = dict("foo,bar baz\nbaz qux,Z\n");
        assert_eq!(d.apply("foo qux"), "bar baz qux");
        assert_eq!(
            d.apply("bar baz qux"),
            "bar Z",
            "not idempotent, hence the report"
        );
        let msgs = d.validate();
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("baz qux"), "{msgs:?}");
        assert!(msgs[0].contains("line 1"), "{msgs:?}");
    }

    /// The common shape that must *not* be flagged: two spellings of the same
    /// correct output, where one pattern does occur inside the other's
    /// replacement but rewrites it to itself.
    #[test]
    fn a_case_only_rule_inside_a_replacement_is_not_a_hazard() {
        let d = dict("get hub,GitHub\ngithub,GitHub\n");
        assert_eq!(d.validate(), Vec::<String>::new());
        assert_eq!(d.apply("push to get hub"), "push to GitHub");
        assert_eq!(d.apply("push to github"), "push to GitHub");
        assert_eq!(d.apply("push to GitHub"), "push to GitHub");
    }

    #[test]
    fn patterns_that_differ_only_by_case_are_duplicates_in_smart_mode() {
        let d = dict("api,API\nAPI,Application Programming Interface\n");
        assert_eq!(d.rule_count(), 1);
        let msgs = d.validate();
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("duplicate"), "{msgs:?}");
        assert_eq!(d.apply("the api"), "the API");
    }

    #[test]
    fn exact_mode_is_case_sensitive_and_coexists_with_a_smart_rule() {
        let d = dict("dr,doctor\nDR,Disaster Recovery,exact\n");
        assert_eq!(d.rule_count(), 2);
        assert_eq!(d.validate(), Vec::<String>::new());
        assert_eq!(d.apply("DR plan"), "Disaster Recovery plan");
        assert_eq!(d.apply("dr smith"), "doctor smith");
        // exact never fires on a different casing
        assert_eq!(dict("DR,X,exact\n").apply("dr"), "dr");
        assert_eq!(dict("DR,X,exact\n").apply("Dr"), "Dr");
    }

    #[test]
    fn exact_mode_emits_the_replacement_verbatim() {
        // no case carrying: exact is the escape hatch for "leave my acronym"
        assert_eq!(dict("it,IT,exact\n").apply("IT"), "IT");
        assert_eq!(dict("It,it,exact\n").apply("It was"), "it was");
    }

    // ---- case handling ----------------------------------------------------

    #[test]
    fn a_lowercase_rule_fixes_the_word_at_a_sentence_start() {
        let csv = "wisper,whispr\n";
        assert_eq!(go(csv, "wisper is fast"), "whispr is fast");
        assert_eq!(go(csv, "Wisper is fast"), "Whispr is fast");
        assert_eq!(go(csv, "I like wisper"), "I like whispr");
        assert_eq!(go(csv, "Yes. Wisper."), "Yes. Whispr.");
    }

    #[test]
    fn an_uppercase_replacement_is_used_as_written() {
        let csv = "github,GitHub\n";
        assert_eq!(go(csv, "github"), "GitHub");
        assert_eq!(go(csv, "Github"), "GitHub");
        assert_eq!(go(csv, "gitHub"), "GitHub");
    }

    /// The requirement in the issue: fix the sentence start without destroying
    /// an intentional acronym.
    #[test]
    fn shouted_text_stays_shouted() {
        // a case-only rule reproduces its own input, so the acronym survives
        assert_eq!(go("sam,Sam\n", "SAM is an acronym"), "SAM is an acronym");
        assert_eq!(go("github,GitHub\n", "GITHUB"), "GITHUB");
        assert_eq!(go("iphone,iPhone\n", "IPHONE"), "IPHONE");
        // ...but a genuine misspelling is still corrected, in the same voice
        assert_eq!(go("wisper,whispr\n", "WISPER"), "WHISPR");
        // one capital is a sentence start, not a shout
        assert_eq!(go("sam,Sam\n", "Sam"), "Sam");
        assert_eq!(go("i,I\n", "i think"), "I think");
    }

    #[test]
    fn mixed_case_input_takes_the_replacement_verbatim() {
        assert_eq!(go("mcdonald,McDonald\n", "mcDonald"), "McDonald");
        assert_eq!(go("sam,Sam\n", "sAm"), "Sam");
        // multi-word matches: two capitals is not "capitalised"
        assert_eq!(go("get hub,github\n", "Get Hub"), "github");
        assert_eq!(go("get hub,github\n", "Get hub"), "Github");
        assert_eq!(go("get hub,github\n", "GET HUB"), "GITHUB");
    }

    #[test]
    fn a_replacement_with_no_letters_survives_case_carrying() {
        assert_eq!(go("c plus plus,C++\n", "C PLUS PLUS"), "C++");
        assert_eq!(go("smiley,:-)\n", "Smiley"), ":-)");
    }

    // ---- disabled / empty -------------------------------------------------

    #[test]
    fn a_disabled_dictionary_is_byte_identical_on_everything() {
        let t = TempDir::new("disabled");
        let path = t.file(
            "dictionary.csv",
            "hello,HELLO\nworld,WORLD\nthe,THE\num,UM\n",
        );
        let d = Dictionary::new(DictionaryConfig {
            enabled: false,
            path: Some(path),
        });
        assert_eq!(d.rule_count(), 4, "the rules loaded, they are just off");
        for input in torture_inputs() {
            assert_eq!(
                d.apply(&input),
                input,
                "disabled dictionary changed {:?}",
                truncate(&input)
            );
        }
    }

    #[test]
    fn an_empty_dictionary_is_the_identity_function() {
        for csv in ["", "\n\n\n", "# only a comment\n", "pattern,replacement\n"] {
            let d = dict(csv);
            assert_eq!(d.rule_count(), 0, "{csv:?}");
            assert_eq!(d.validate(), Vec::<String>::new(), "{csv:?}");
            for input in torture_inputs() {
                assert_eq!(
                    d.apply(&input),
                    input,
                    "{csv:?} changed {:?}",
                    truncate(&input)
                );
            }
        }
    }

    #[test]
    fn a_dictionary_that_matches_nothing_leaves_every_torture_input_alone() {
        let d = dict("zzqqxx,Q\nzzqqxx yyww,R\n");
        for input in small_torture_inputs() {
            assert_eq!(d.apply(&input), input, "changed {:?}", truncate(&input));
        }
    }

    // ---- idempotence ------------------------------------------------------

    /// The property the chain rules exist to protect. Every dictionary here
    /// validates clean, which is exactly the condition under which the module
    /// promises idempotence.
    #[test]
    fn a_clean_dictionary_is_idempotent_on_every_torture_input() {
        let dicts = [
            "sam,Sam\n",
            "hello,Hi there\nworld,Earth\n",
            "the quick,THE QUICK\n",
            "a,A\n",
            "日本語,ニホンゴ\nテキスト,text\n",
            "e\u{0301}gal,equal\n",
            "правда,Правда\n",
            "um,\n",
            "very,extremely\n",
            "quick brown,swift auburn\nlazy dog,idle hound\n",
            "shipped,launched\n",
        ];
        for csv in dicts {
            let d = dict(csv);
            assert_eq!(d.validate(), Vec::<String>::new(), "{csv:?} is not clean");
            for input in small_torture_inputs() {
                let once = d.apply(&input);
                assert_eq!(
                    d.apply(&once),
                    once,
                    "{csv:?} is not idempotent on {:?}",
                    truncate(&input)
                );
            }
        }
    }

    /// Rules that insert characters designed to break the next pass: a
    /// combining mark, a zero-width space, a character whose case mapping
    /// changes its length. Nothing here may panic, and everything must survive
    /// a second pass unchanged.
    #[test]
    fn adversarial_replacements_still_round_trip() {
        let d = dict("sam,Sam\naaa,ä\nzzz,\u{200b}zzz2\nyyy,e\u{0301}\nqqq,日本語\n");
        assert_eq!(d.validate(), Vec::<String>::new());
        for input in small_torture_inputs() {
            let once = d.apply(&input);
            assert_eq!(
                d.apply(&once),
                once,
                "not idempotent on {:?}",
                truncate(&input)
            );
        }
        assert_eq!(d.apply("aaa"), "ä");
        assert_eq!(d.apply("say zzz twice"), "say \u{200b}zzz2 twice");
        assert_eq!(d.apply("yyy"), "e\u{0301}");
        assert_eq!(d.apply("qqq"), "日本語");
    }

    /// The three giants in the corpus, once, against the largest dictionary in
    /// this file. A naive implementation — 500 passes of `str::replace` — would
    /// allocate 500 copies of 2 MB and blow this budget by orders of magnitude.
    /// Measured on an M1 over the 2 MB entry: 47 ms release, 230 ms
    /// unoptimised, and 23 ns/byte from 0.5 MB to 4 MB, which is the shape of a
    /// linear pass rather than a quadratic one.
    #[test]
    fn the_giant_torture_inputs_are_cheap_and_idempotent() {
        let d = dict(&five_hundred());
        for input in giant_torture_inputs() {
            let t0 = std::time::Instant::now();
            let once = d.apply(&input);
            let elapsed = t0.elapsed();
            println!("500 entries over {} bytes: {elapsed:?}", input.len());
            assert_eq!(once, input, "nothing in the corpus matches these rules");
            assert_eq!(d.apply(&once), once);
            assert!(
                elapsed < std::time::Duration::from_secs(5),
                "{} bytes took {elapsed:?}",
                input.len()
            );
        }
    }

    // ---- prefix stability -------------------------------------------------

    /// #43: keep this false unless you can prove the prefix property, and add
    /// a `prefix_violation` case here showing why the proof holds.
    #[test]
    fn is_not_prefix_stable() {
        assert!(!Dictionary::new(DictionaryConfig::default()).prefix_stable());
        assert!(!dict("sam,Sam\n").prefix_stable());
    }

    /// The measured counterexample, not an assumed one: `prefix_violation`
    /// against the real implementation, with the entry from the doc comment.
    #[test]
    fn prefix_violation_finds_a_real_counterexample() {
        let d = dict("push to get,push to GitHub\n");
        let (prefix, polished_prefix, polished_whole) = prefix_violation(&d, "push to get now")
            .expect("a dictionary rewrites its own prefixes");
        // The *first* prefix that breaks is one character earlier than the doc
        // comment's example: streaming has typed "push to g", the finished
        // utterance polishes to "push to GitHub now", and the lower-case "g"
        // on screen is already wrong.
        assert_eq!(prefix, "push to g");
        assert_eq!(polished_prefix, "push to g");
        assert_eq!(polished_whole, "push to GitHub now");
        assert!(!polished_whole.starts_with(&polished_prefix));

        // Worth pinning, because it is the case the seam's doc comment
        // originally reached for and it is *not* a violation: once the trigger
        // is complete, "push to GitHub" is a prefix of "push to GitHub now".
        // The property breaks while the trigger is still being spoken, not
        // after it finishes.
        assert_eq!(d.apply("push to get"), "push to GitHub");
        assert!(d
            .apply("push to get now")
            .starts_with(&d.apply("push to get")));

        // the simplest possible entry breaks it too: a single-word rule whose
        // replacement only differs in case
        let d = dict("sam,Sam\n");
        let (prefix, polished_prefix, whole) = prefix_violation(&d, "sam").unwrap();
        assert_eq!(
            (prefix.as_str(), polished_prefix.as_str(), whole.as_str()),
            ("s", "s", "Sam")
        );
    }

    // ---- CSV --------------------------------------------------------------

    #[test]
    fn comments_blank_lines_and_indentation_are_ignored() {
        let d = dict(
            "# my dictionary\n\
             \n\
             sam,Sam\n\
             \n\
               # indented comment\n\
               aviroop,Aviroop\n\
             \r\n\
             ",
        );
        assert_eq!(d.rule_count(), 2);
        assert_eq!(d.validate(), Vec::<String>::new());
        assert_eq!(d.apply("sam and aviroop"), "Sam and Aviroop");
    }

    #[test]
    fn the_header_row_is_optional_and_skipped() {
        let with = dict("pattern,replacement\nsam,Sam\n");
        let without = dict("sam,Sam\n");
        assert_eq!(with.rule_count(), without.rule_count());
        assert_eq!(with.validate(), Vec::<String>::new());
        assert_eq!(dict("Pattern,Replacement\nsam,Sam\n").rule_count(), 1);
        // only on the first record
        let d = dict("sam,Sam\npattern,replacement\n");
        assert_eq!(d.rule_count(), 2);
        assert_eq!(d.apply("pattern"), "replacement");
    }

    #[test]
    fn quoted_fields_carry_commas_quotes_and_padding() {
        let d =
            dict("\"hello, world\",\"Hello, World\"\nsay,\"he said \"\"hi\"\"\"\npad,\"  x  \"\n");
        assert_eq!(d.apply("hello, world"), "Hello, World");
        assert_eq!(d.apply("say"), "he said \"hi\"");
        assert_eq!(d.apply("pad"), "  x  ");
    }

    #[test]
    fn unquoted_fields_are_trimmed() {
        let d = dict("  sam  ,  Sam  \n");
        assert_eq!(d.apply("sam"), "Sam");
        // whitespace inside a pattern is normalised, so it still matches
        assert_eq!(dict("get   hub,GitHub\n").apply("get hub"), "GitHub");
    }

    #[test]
    fn a_quoted_field_may_contain_a_newline() {
        let d = dict("sig,\"line one\nline two\"\nsam,Sam\n");
        assert_eq!(d.apply("sig"), "line one\nline two");
        assert_eq!(d.apply("sam"), "Sam", "the line counter kept up");
        assert_eq!(d.validate(), Vec::<String>::new());
    }

    #[test]
    fn crlf_files_load() {
        let d = dict("sam,Sam\r\naviroop,Aviroop\r\n");
        assert_eq!(d.rule_count(), 2);
        assert_eq!(d.apply("sam aviroop"), "Sam Aviroop");
    }

    #[test]
    fn a_file_with_no_trailing_newline_loads() {
        assert_eq!(dict("sam,Sam").rule_count(), 1);
        assert_eq!(dict("sam,Sam").apply("sam"), "Sam");
    }

    /// "Export to CSV" from a spreadsheet is the import path this format
    /// exists for, and Excel puts a byte-order mark on the front.
    #[test]
    fn a_spreadsheet_export_with_a_byte_order_mark_loads() {
        let d = dict("\u{feff}pattern,replacement\r\nsam,Sam\r\n");
        assert_eq!(d.rule_count(), 1);
        assert_eq!(d.validate(), Vec::<String>::new());
        assert_eq!(d.apply("sam"), "Sam");
        // and with no header row in front of it
        assert_eq!(dict("\u{feff}sam,Sam\n").apply("sam"), "Sam");
    }

    #[test]
    fn an_empty_replacement_deletes_the_word() {
        let d = dict("basically,\n");
        assert_eq!(d.validate(), Vec::<String>::new());
        assert_eq!(d.apply("it is basically fine"), "it is  fine");
        assert_eq!(d.apply("Basically"), "");
    }

    // ---- validation -------------------------------------------------------

    #[test]
    fn an_empty_pattern_is_reported_and_dropped() {
        let d = dict("sam,Sam\n,Nobody\n");
        assert_eq!(d.rule_count(), 1);
        let msgs = d.validate();
        assert_eq!(msgs, vec!["dictionary.csv:2: empty pattern".to_string()]);
    }

    #[test]
    fn a_whitespace_only_pattern_is_reported_and_dropped() {
        let d = dict("\"   \",Nobody\n\"\t\",Nobody\n");
        assert_eq!(d.rule_count(), 0);
        let msgs = d.validate();
        assert_eq!(msgs.len(), 2, "{msgs:?}");
        assert!(msgs[0].contains("only whitespace"), "{msgs:?}");
        assert!(msgs[1].starts_with("dictionary.csv:2:"), "{msgs:?}");
    }

    #[test]
    fn a_rule_that_does_nothing_is_reported_and_dropped() {
        let d = dict("sam,sam\n");
        assert_eq!(d.rule_count(), 0);
        let msgs = d.validate();
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("no effect"), "{msgs:?}");
    }

    #[test]
    fn a_rule_that_feeds_itself_is_reported() {
        let d = dict("sam,sam smith\n");
        assert_eq!(d.apply("sam"), "sam smith", "still one pass only");
        let msgs = d.validate();
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("sam smith smith"), "{msgs:?}");
    }

    #[test]
    fn duplicate_patterns_are_reported_and_the_first_wins() {
        let d = dict("sam,Sam\nsam,Samuel\nSAM,Sammy\n");
        assert_eq!(d.rule_count(), 1);
        assert_eq!(d.apply("sam"), "Sam");
        let msgs = d.validate();
        assert_eq!(msgs.len(), 2, "{msgs:?}");
        assert!(msgs[0].contains("duplicate pattern \"sam\""), "{msgs:?}");
        assert!(msgs[0].contains("line 1"), "{msgs:?}");
        assert!(msgs[1].starts_with("dictionary.csv:3:"), "{msgs:?}");
    }

    #[test]
    fn a_short_row_is_reported() {
        let d = dict("sam\nsam,Sam\n");
        assert_eq!(d.rule_count(), 1);
        let msgs = d.validate();
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("expected pattern,replacement"), "{msgs:?}");
    }

    #[test]
    fn an_unquoted_comma_in_a_replacement_is_reported_with_a_hint() {
        let d = dict("sam,Sam, the founder, really\n");
        assert_eq!(d.rule_count(), 0);
        let msgs = d.validate();
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("quote a field"), "{msgs:?}");
    }

    #[test]
    fn an_unknown_mode_is_reported_with_the_options() {
        let d = dict("sam,Sam,fuzzy\n");
        assert_eq!(d.rule_count(), 0);
        let msgs = d.validate();
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("\"smart\""), "{msgs:?}");
        assert!(msgs[0].contains("\"exact\""), "{msgs:?}");
    }

    #[test]
    fn an_unterminated_quote_is_reported() {
        let d = dict("sam,Sam\nbad,\"never closed\n");
        let msgs = d.validate();
        assert!(
            msgs.iter().any(|m| m.contains("unterminated quote")),
            "{msgs:?}"
        );
        assert_eq!(d.apply("sam"), "Sam", "the good rule still loaded");
    }

    #[test]
    fn validation_does_not_depend_on_enabled() {
        let t = TempDir::new("validate-off");
        let path = t.file("dictionary.csv", ",Nobody\n");
        for enabled in [false, true] {
            let d = Dictionary::new(DictionaryConfig {
                enabled,
                path: Some(path.clone()),
            });
            assert_eq!(d.validate().len(), 1, "enabled = {enabled}");
        }
    }

    #[test]
    fn a_realistic_dictionary_validates_clean() {
        let d = dict(
            "# team dictionary\n\
             pattern,replacement\n\
             aviroop,Aviroop\n\
             wisper catch,WhisprCatch\n\
             get hub,GitHub\n\
             para keet,Parakeet\n\
             moon shine,Moonshine\n\
             oh nix,ONNX\n\
             api,API,exact\n",
        );
        assert_eq!(d.rule_count(), 7);
        assert_eq!(d.validate(), Vec::<String>::new());
        assert_eq!(
            d.apply("hey it's aviroop, wisper catch runs para keet through oh nix"),
            "hey it's Aviroop, WhisprCatch runs Parakeet through ONNX"
        );
    }

    // ---- files on disk ----------------------------------------------------

    #[test]
    fn a_missing_file_is_not_an_error() {
        let d = Dictionary::new(DictionaryConfig {
            enabled: true,
            path: Some(PathBuf::from("/nonexistent/whisper-catch/dictionary.csv")),
        });
        assert_eq!(d.rule_count(), 0);
        assert_eq!(d.validate(), Vec::<String>::new());
        assert_eq!(d.apply("anything at all"), "anything at all");
    }

    #[test]
    fn no_configured_path_and_no_default_means_no_rules() {
        // `default_path` is `None` under cfg(test) so that the crate's tests
        // never read a contributor's real dictionary
        assert_eq!(Dictionary::default_path(), None);
        let d = Dictionary::new(DictionaryConfig::default());
        assert_eq!(d.rule_count(), 0);
        assert_eq!(d.validate(), Vec::<String>::new());
    }

    #[test]
    fn a_file_that_cannot_be_read_is_reported() {
        let t = TempDir::new("unreadable");
        // a directory where a file should be: readable path, unreadable content
        let d = Dictionary::new(DictionaryConfig {
            enabled: true,
            path: Some(t.0.clone()),
        });
        assert_eq!(d.rule_count(), 0);
        assert_eq!(d.validate().len(), 1, "{:?}", d.validate());
        assert_eq!(d.apply("still dictating"), "still dictating");
    }

    #[test]
    fn the_configured_path_is_used_and_messages_name_it() {
        let t = TempDir::new("override");
        let path = t.file("team.csv", "sam,Sam\n,oops\n");
        let d = Dictionary::new(DictionaryConfig {
            enabled: true,
            path: Some(path.clone()),
        });
        assert_eq!(d.apply("sam"), "Sam");
        let msgs = d.validate();
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(
            msgs[0].starts_with(&format!("{}:2:", path.display())),
            "{msgs:?}"
        );
    }

    #[test]
    fn the_default_path_sits_beside_config_toml() {
        let get = |home: Option<&str>, xdg: Option<&str>, appdata: Option<&str>| {
            config_dir(move |k| match k {
                "HOME" => home.map(OsString::from),
                "XDG_CONFIG_HOME" => xdg.map(OsString::from),
                "APPDATA" => appdata.map(OsString::from),
                _ => None,
            })
        };
        if cfg!(target_os = "macos") {
            assert_eq!(
                get(Some("/Users/a"), None, None),
                Some(PathBuf::from("/Users/a/Library/Application Support"))
            );
        } else if cfg!(target_os = "windows") {
            assert_eq!(
                get(None, None, Some("C:\\Users\\a\\AppData\\Roaming")),
                Some(PathBuf::from("C:\\Users\\a\\AppData\\Roaming"))
            );
        } else {
            assert_eq!(
                get(Some("/home/a"), None, None),
                Some(PathBuf::from("/home/a/.config"))
            );
            assert_eq!(
                get(Some("/home/a"), Some("/cfg"), None),
                Some(PathBuf::from("/cfg"))
            );
            // dirs ignores a relative XDG_CONFIG_HOME, and so must we
            assert_eq!(
                get(Some("/home/a"), Some("relative"), None),
                Some(PathBuf::from("/home/a/.config"))
            );
        }
        assert_eq!(get(None, None, None), None);
        assert_eq!(
            APP_DIR, "whisper-catch",
            "must match config_path in apps/cli"
        );
        assert_eq!(FILE_NAME, "dictionary.csv");
    }

    // ---- performance ------------------------------------------------------

    /// A realistic dictation, roughly one minute of speech.
    fn transcript() -> String {
        let sentence = "So the plan for this week is to land the streaming reconciliation work, \
                        then look at the injector replace path, and finally get the dictionary \
                        shipped so that nobody has to spell their own name twice. ";
        sentence.repeat(6)
    }

    /// 500 entries, the size the issue budgets for.
    fn five_hundred() -> String {
        let mut csv = String::new();
        for i in 0..250 {
            csv.push_str(&format!("term{i},Term{i}\n"));
            csv.push_str(&format!("phrase {i} here,Phrase{i}\n"));
        }
        // a handful that actually fire, so the measurement is not of a miss
        csv.push_str("dictionary,Dictionary\ninjector,Injector\nstreaming,Streaming\n");
        csv
    }

    #[test]
    fn five_hundred_entries_cost_under_a_millisecond() {
        let d = dict(&five_hundred());
        assert_eq!(d.rule_count(), 503);
        let text = transcript();
        assert!(text.len() > 900, "{} bytes", text.len());

        // warm up, then take the best of many: the minimum is the measurement
        // least polluted by a busy CI runner.
        for _ in 0..20 {
            std::hint::black_box(d.apply(&text));
        }
        let mut best = std::time::Duration::MAX;
        for _ in 0..200 {
            let t0 = std::time::Instant::now();
            std::hint::black_box(d.apply(&text));
            best = best.min(t0.elapsed());
        }
        // The issue budgets 1 ms. Debug builds are 10-30x slower than the
        // release binary users run, so the assertion scales and the number is
        // always printed.
        let budget = if cfg!(debug_assertions) {
            std::time::Duration::from_millis(20)
        } else {
            std::time::Duration::from_millis(1)
        };
        println!(
            "500-entry dictionary over {} bytes: {:?} (budget {:?})",
            text.len(),
            best,
            budget
        );
        assert!(
            best < budget,
            "500 entries took {best:?}, budget {budget:?}"
        );
    }

    /// `validate` is O(rules^2) in the worst case and Settings (#49) calls it
    /// on every keystroke-ish edit, so a full dictionary must not stall the UI.
    #[test]
    fn validating_five_hundred_entries_is_not_pathological() {
        let d = dict(&five_hundred());
        let t0 = std::time::Instant::now();
        let msgs = d.validate();
        let elapsed = t0.elapsed();
        println!("validate() over {} rules: {elapsed:?}", d.rule_count());
        assert_eq!(msgs, Vec::<String>::new());
        let budget = if cfg!(debug_assertions) { 2000 } else { 100 };
        assert!(
            elapsed < std::time::Duration::from_millis(budget),
            "validate took {elapsed:?}"
        );
    }

    /// The `max_key_len` cap, which is what stops one enormous token being
    /// lowercased into a key no rule could ever have.
    #[test]
    fn a_single_enormous_token_is_cheap() {
        let d = dict(&five_hundred());
        let big = "a".repeat(500_000);
        let t0 = std::time::Instant::now();
        assert_eq!(d.apply(&big), big);
        assert!(
            t0.elapsed() < std::time::Duration::from_secs(2),
            "one long token took {:?}",
            t0.elapsed()
        );
    }

    // ---- internals worth pinning -----------------------------------------

    #[test]
    fn case_insensitive_prefix_matching_handles_expanding_lowercase() {
        assert_eq!(ci_prefix_len("SAMe", "sam"), Some(3));
        assert_eq!(ci_prefix_len("sam", "SAM"), Some(3));
        assert_eq!(ci_prefix_len("sa", "sam"), None);
        assert_eq!(ci_prefix_len("ÉCOLE", "école"), Some("ÉCOLE".len()));
        // U+0130 lowercases to two characters; a needle that covers only the
        // first of them is not a match of length 2
        assert_eq!(ci_prefix_len("\u{0130}", "i"), None);
        assert_eq!(ci_prefix_len("\u{0130}", "i\u{0307}"), Some(2));
    }

    #[test]
    fn word_runs_stop_at_the_cap() {
        assert_eq!(word_run_end("sam ", 0, 8), Some(3));
        assert_eq!(word_run_end("sam", 0, 8), Some(3));
        assert_eq!(word_run_end("sam", 0, 2), None);
        assert_eq!(word_run_end(",sam", 0, 8), Some(0));
    }

    #[test]
    fn the_transform_name_is_stable() {
        assert_eq!(dict("").name(), "dictionary");
    }

    #[test]
    fn disabled_by_default() {
        let cfg = DictionaryConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.path, None);
    }

    #[test]
    fn config_round_trips_through_toml() {
        let cfg = DictionaryConfig {
            enabled: true,
            path: Some(PathBuf::from("/team/dictionary.csv")),
        };
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: DictionaryConfig = toml::from_str(&text).unwrap();
        assert!(back.enabled);
        assert_eq!(back.path, Some(PathBuf::from("/team/dictionary.csv")));
        // and the shape older configs have, with no `path` key at all
        let old: DictionaryConfig = toml::from_str("enabled = true\n").unwrap();
        assert!(old.enabled);
        assert_eq!(old.path, None);
        // ...and with no keys at all
        let empty: DictionaryConfig = toml::from_str("").unwrap();
        assert!(!empty.enabled);
    }
}
