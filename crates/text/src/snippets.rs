//! Snippets — a spoken trigger expands to canned text ("my address" → the
//! actual address). Dictation's text expander.
//!
//! # Where snippets live
//!
//! In their own file, `snippets.txt`, next to `config.toml` in the app's config
//! directory (`~/.config/whisper-catch/` on Linux, `~/Library/Application
//! Support/whisper-catch/` on macOS). **Not** in `config.toml`: that file
//! round-trips through a typed struct and drops every key the running build
//! does not know about on the next Settings save, which would silently destroy
//! a user's snippets the first time they downgrade. A plain text file is also
//! something you can put in a dotfiles repo or share with a team.
//!
//! A missing file is not an error. It means no snippets.
//!
//! # File format
//!
//! ```text
//! # Lines starting with '#' are comments.
//! # A line of the form [trigger] starts an entry; every line until the next
//! # [trigger] is that entry's text, line breaks and all.
//!
//! [insert my email]
//! ada@example.com
//!
//! [sign off]
//! Best,
//! Ada
//! Founder, WhisprCatch
//! ```
//!
//! - Blank lines directly above and below a body are separators and are
//!   dropped; blank lines *inside* a body are kept.
//! - A body line that has to start with `[` or `#` is escaped with a
//!   backslash: `\[draft]`, `\# heading`.
//! - `\r\n` files are read as if they were `\n` files, so a snippets file
//!   synced from another machine still works.
//!
//! # Matching: whole phrase, never mid-sentence
//!
//! This is the one thing that separates snippets from the custom dictionary
//! (#43). The dictionary rewrites *words* wherever they appear. A snippet
//! fires only when its trigger is an **entire sentence** of the utterance —
//! the text between one sentence boundary (`. ! ? ; :` a newline, or their CJK
//! equivalents) and the next.
//!
//! ```text
//! "Sign off."                        -> the signature block
//! "Please sign off on this document" -> unchanged
//! ```
//!
//! That is what "triggers do not fire mid-sentence when the phrase appears
//! incidentally" costs: `"email me at insert my email please"` does *not*
//! expand either. Deliberate. A snippet pastes an address, a signature or a
//! meeting link into whatever app has focus; a false positive there is far
//! more expensive than one missed expansion, which the user fixes by saying
//! the trigger on its own.
//!
//! Comparison ignores case and collapses runs of whitespace, so "Sign  off"
//! and "SIGN OFF" both fire. A comma does **not** end a sentence, so
//! "sign off, and let me know" keeps its verb.
//!
//! When a trigger fires at the very end of the utterance, the full stop the
//! model added after it goes away with it ("Insert my email." →
//! `ada@example.com`, not `ada@example.com.`, which would break the address in
//! most apps). Mid-utterance the stop is kept, because it still separates two
//! sentences.
//!
//! # Composition with the custom dictionary (#43)
//!
//! [`crate::Polish`] runs `dictionary` **before** `snippets`, so a snippet
//! always sees text the dictionary has already rewritten. That is a feature —
//! a dictionary rule can repair a trigger the model misheard:
//!
//! ```text
//! dictionary: "sign of"  -> "sign off"
//! snippet:    "sign off" -> "Best,\nAda"
//! "Sign of." -> dictionary -> "Sign off." -> snippets -> "Best,\nAda"
//! ```
//!
//! and it also means a dictionary rule can *destroy* a trigger without saying
//! so. A user who adds `"email" -> "e-mail"` because that is how they write it
//! turns `"Insert my email."` into `"Insert my e-mail."` before snippets get a
//! look, and the trigger silently never fires again. (Case is not a failure
//! mode — matching folds case, so `"email" -> "EMAIL"` still expands.) One
//! order had to lose; this one loses in the direction the user can see and
//! fix, because their dictionary is a file they wrote. The reverse order would
//! feed snippet *bodies* to the dictionary and rewrite text the user typed by
//! hand, which is worse. `a_dictionary_rule_reaches_inside_a_trigger` is the
//! executable version of all three cases.
//!
//! # Idempotence
//!
//! `apply(apply(x)) == apply(x)`, and the proof is structural rather than a
//! loop with a counter: an entry whose body contains any trigger as a whole
//! sentence is reported by [`Transform::validate`] and **disabled**, so no
//! expansion can ever produce text that a later pass would expand again.
//! Snippets do not nest.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Transform;

/// Name of the snippets file inside the config directory.
pub const SNIPPETS_FILE: &str = "snippets.txt";

/// Configuration for [`Snippets`].
///
/// The snippets themselves are *not* here — see the module docs for why. All
/// that lives in `config.toml` is the on/off switch and an optional path for
/// people who keep the file somewhere else (a dotfiles checkout, a shared
/// team folder).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SnippetsConfig {
    /// Off by default, like every transform in this crate.
    pub enabled: bool,

    /// Where to read the snippets file from. `None` means the default
    /// location, [`default_path`]. Skipped on serialize so an untouched
    /// `config.toml` stays exactly as small as it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

/// One parsed entry: the trigger as the user wrote it, its body, and the line
/// its `[trigger]` header was on so problems can point at it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub trigger: String,
    pub body: String,
    pub line: usize,
}

/// Expands spoken triggers into their stored text.
pub struct Snippets {
    enabled: bool,
    snippets: Vec<Snippet>,
    /// Normalized trigger → index into `snippets`. Only usable entries are in
    /// here; a malformed one is reported by `validate` and never expands.
    by_trigger: HashMap<String, usize>,
    /// Longest usable trigger, in non-whitespace characters. Lets `lookup`
    /// reject a long sentence without normalizing it.
    max_trigger_chars: usize,
    problems: Vec<String>,
}

impl Snippets {
    /// Loads the snippets file named by `cfg` (or the default location).
    ///
    /// This is the only place in the crate that touches the filesystem, and it
    /// happens once, when the chain is built. [`Transform::apply`] stays a
    /// pure function of its input.
    pub fn new(cfg: SnippetsConfig) -> Self {
        let path = cfg.path.clone().or_else(default_path_for_load);
        let mut loaded = match path {
            Some(p) => Self::from_file(&p),
            // No home directory to read from. Not worth a complaint in
            // Settings: there is nowhere to put the file either.
            None => Self::from_source(""),
        };
        loaded.enabled = cfg.enabled;
        loaded
    }

    /// Reads and parses a snippets file. A missing file is not an error and
    /// not a problem to report — it is the normal state of a fresh install.
    /// Anything else (no permission, not valid UTF-8, a directory) is
    /// reported, because it means snippets the user believes in are not
    /// loading.
    pub fn from_file(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_source(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::from_source(""),
            Err(e) => {
                let mut s = Self::from_source("");
                s.problems
                    .push(format!("snippets: cannot read {}: {e}", path.display()));
                s
            }
        }
    }

    /// Parses snippets from the text of a snippets file. Pure — this is the
    /// seam the Settings preview (#49) and every test in this module use.
    ///
    /// The returned transform is enabled; [`Snippets::new`] is the path that
    /// respects the config flag.
    ///
    /// ```
    /// use wc_text::{Snippets, Transform};
    ///
    /// let s = Snippets::from_source("[sign off]\nBest,\nAda\n");
    /// assert_eq!(s.apply("Sign off."), "Best,\nAda");
    /// assert_eq!(
    ///     s.apply("Please sign off on this document."),
    ///     "Please sign off on this document."
    /// );
    /// ```
    pub fn from_source(source: &str) -> Self {
        let (snippets, mut problems) = parse(source);
        let (by_trigger, max_trigger_chars, more) = compile(&snippets);
        problems.extend(more);
        Self {
            enabled: true,
            snippets,
            by_trigger,
            max_trigger_chars,
            problems,
        }
    }

    /// Every entry the file defined, in file order — including the ones
    /// `validate` rejected, so Settings (#49) can show them struck through
    /// next to their problem.
    pub fn snippets(&self) -> &[Snippet] {
        &self.snippets
    }

    /// True when nothing will ever expand: no usable entry.
    pub fn is_empty(&self) -> bool {
        self.by_trigger.is_empty()
    }

    /// The body to expand `segment` into, if it is exactly a trigger.
    fn lookup(&self, segment: &str) -> Option<&str> {
        lookup_key(segment, &self.by_trigger, self.max_trigger_chars)
            .map(|(_, i)| self.snippets[i].body.as_str())
    }

    fn expand(&self, text: &str) -> String {
        // Precomputed once: where the utterance's trailing whitespace starts.
        // `expand` needs to know whether anything follows a match, and asking
        // per match would make a text with many matches quadratic.
        let trailing_ws_start = text.trim_end().len();

        let mut out = String::with_capacity(text.len());
        for seg in Segments::new(text) {
            match self.lookup(seg.text) {
                Some(body) => {
                    // Keep the whitespace that framed the trigger ("Hi. Sign
                    // off." must not lose the space after "Hi.").
                    let after_lead = seg.text.trim_start();
                    let lead = &seg.text[..seg.text.len() - after_lead.len()];
                    let core = after_lead.trim_end();
                    let trail = &after_lead[core.len()..];
                    out.push_str(lead);
                    out.push_str(body);
                    out.push_str(trail);
                    if let Some(d) = seg.delim {
                        // Drop the full stop the model put after a trigger
                        // that ended the utterance; keep it when it still
                        // separates this sentence from the next one.
                        let ends_utterance = seg.delim_end >= trailing_ws_start;
                        if !(is_sentence_end(d) && ends_utterance) {
                            out.push(d);
                        }
                    }
                }
                None => {
                    out.push_str(seg.text);
                    if let Some(d) = seg.delim {
                        out.push(d);
                    }
                }
            }
        }
        out
    }
}

impl Transform for Snippets {
    fn name(&self) -> &'static str {
        "snippets"
    }

    fn apply(&self, text: &str) -> String {
        // The `enabled` check is belt and braces: `Polish::from_config` never
        // puts a disabled transform in the chain. It is here so that a
        // hand-built chain (#49's preview) cannot paste someone's home address
        // into a document from a feature they switched off.
        if !self.enabled || self.by_trigger.is_empty() {
            return text.to_string();
        }
        self.expand(text)
    }

    /// Not prefix-stable. With the snippet "sign off" -> "Best,\nAda", a
    /// streaming pass that has heard `"Hello. Sign o"` has already typed those
    /// 13 characters, and the finished `"Hello. Sign off."` polishes to
    /// `"Hello. Best,\nAda"` — which does not start with what is on screen, so
    /// the last six characters would have to be retracted. `prefix_violation`
    /// finds exactly that pair; see `prefix_violation_finds_a_straddling_trigger`.
    fn prefix_stable(&self) -> bool {
        false
    }

    /// Malformed entries, in file order. Each names the line so Settings can
    /// jump to it. Every problem reported here also means the entry does not
    /// expand: a snippet the app cannot make sense of must not fire.
    fn validate(&self) -> Vec<String> {
        self.problems.clone()
    }
}

// ---- parsing --------------------------------------------------------------

/// Splits a snippets file into entries. The second half of the pair is the
/// problems the *parse* can see; [`compile`] finds the rest.
fn parse(source: &str) -> (Vec<Snippet>, Vec<String>) {
    let mut snippets: Vec<Snippet> = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    // (trigger, header line number, body lines so far)
    let mut pending: Option<(String, usize, Vec<String>)> = None;
    let mut warned_about_preamble = false;

    for (i, line) in source.lines().enumerate() {
        let line_no = i + 1;
        if let Some(trigger) = header_trigger(line) {
            if let Some((t, n, body)) = pending.take() {
                snippets.push(finish_snippet(t, n, body));
            }
            pending = Some((trigger.to_string(), line_no, Vec::new()));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        match pending.as_mut() {
            Some((_, _, body)) => body.push(unescape(line).to_string()),
            None => {
                if !line.trim().is_empty() && !warned_about_preamble {
                    warned_about_preamble = true;
                    problems.push(format!(
                        "snippets line {line_no}: text before the first [trigger] line is ignored"
                    ));
                }
            }
        }
    }
    if let Some((t, n, body)) = pending.take() {
        snippets.push(finish_snippet(t, n, body));
    }
    (snippets, problems)
}

/// `[insert my email]` → `insert my email`. The bracket must be the first
/// character of the line, which is what lets an indented body line contain one.
fn header_trigger(line: &str) -> Option<&str> {
    line.trim_end().strip_prefix('[')?.strip_suffix(']')
}

/// Un-escapes a body line that had to start with `[` or `#`.
fn unescape(line: &str) -> &str {
    match line.strip_prefix('\\') {
        Some(rest) if rest.starts_with('[') || rest.starts_with('#') => rest,
        _ => line,
    }
}

fn finish_snippet(trigger: String, line: usize, mut body: Vec<String>) -> Snippet {
    // Blank lines hugging a body are file layout, not content. Blank lines
    // inside one are content — a signature block with a gap in it survives.
    let blank = |l: &String| l.trim().is_empty();
    while body.last().is_some_and(blank) {
        body.pop();
    }
    let first = body.iter().position(|l| !blank(l)).unwrap_or(body.len());
    Snippet {
        trigger,
        body: body[first..].join("\n"),
        line,
    }
}

// ---- compiling ------------------------------------------------------------

/// Turns parsed entries into the lookup map, rejecting the ones that cannot
/// work. Returns `(map, longest trigger in non-whitespace chars, problems)`.
fn compile(snippets: &[Snippet]) -> (HashMap<String, usize>, usize, Vec<String>) {
    let mut by_trigger: HashMap<String, usize> = HashMap::new();
    let mut problems: Vec<String> = Vec::new();

    for (idx, s) in snippets.iter().enumerate() {
        if s.trigger.is_empty() {
            problems.push(format!("snippets line {}: empty trigger, `[]`", s.line));
            continue;
        }
        let key = normalize_key(&s.trigger);
        if key.is_empty() {
            problems.push(format!(
                "snippets line {}: trigger is only whitespace",
                s.line
            ));
            continue;
        }
        if let Some(bad) = s.trigger.chars().find(|c| is_delimiter(*c)) {
            problems.push(format!(
                "snippets line {}: trigger {:?} contains {bad:?}, which ends a sentence, \
                 so the trigger can never match a whole one — remove it",
                s.line, s.trigger
            ));
            continue;
        }
        match by_trigger.entry(key) {
            Entry::Vacant(v) => {
                v.insert(idx);
            }
            Entry::Occupied(o) => problems.push(format!(
                "snippets line {}: duplicate trigger {:?}, already defined on line {} — \
                 only the first one is used",
                s.line,
                s.trigger,
                snippets[*o.get()].line
            )),
        }
    }

    // Second pass: an entry whose body contains a trigger would expand again
    // on a second `apply`. Report it and disable it — this is what makes
    // `apply` idempotent by construction rather than by iteration limit.
    //
    // Deliberately walks `snippets` in file order rather than the map, so the
    // problems come out in the same order on every run.
    let max = max_trigger_chars(&by_trigger);
    let mut nested: Vec<String> = Vec::new();
    for (idx, s) in snippets.iter().enumerate() {
        let key = normalize_key(&s.trigger);
        if by_trigger.get(&key) != Some(&idx) {
            continue; // already rejected above, or shadowed by a duplicate
        }
        if let Some(found) = first_trigger_in(&s.body, &by_trigger, max) {
            problems.push(format!(
                "snippets line {}: the body of {:?} contains the trigger {found:?} as a whole \
                 sentence — snippets do not nest, so this entry is disabled",
                s.line, s.trigger
            ));
            nested.push(key);
        }
    }
    for key in nested {
        by_trigger.remove(&key);
    }

    let max = max_trigger_chars(&by_trigger);
    (by_trigger, max, problems)
}

fn max_trigger_chars(by_trigger: &HashMap<String, usize>) -> usize {
    by_trigger
        .keys()
        .map(|k| k.chars().filter(|c| !c.is_whitespace()).count())
        .max()
        .unwrap_or(0)
}

/// The first trigger `text` contains as a whole sentence, if any.
fn first_trigger_in<'a>(
    text: &str,
    by_trigger: &'a HashMap<String, usize>,
    max_trigger_chars: usize,
) -> Option<&'a str> {
    Segments::new(text)
        .find_map(|seg| lookup_key(seg.text, by_trigger, max_trigger_chars).map(|(k, _)| k))
}

// ---- matching -------------------------------------------------------------

/// Looks one sentence up in the trigger map, ignoring the whitespace around it,
/// the case it was spoken in, and how many spaces the model put between words.
fn lookup_key<'a>(
    segment: &str,
    by_trigger: &'a HashMap<String, usize>,
    max_trigger_chars: usize,
) -> Option<(&'a str, usize)> {
    if by_trigger.is_empty() {
        return None;
    }
    let core = segment.trim();
    if core.is_empty() || longer_than(core, max_trigger_chars) {
        return None;
    }
    by_trigger
        .get_key_value(&normalize_key(core))
        .map(|(k, i)| (k.as_str(), *i))
}

/// True when `s` has more non-whitespace characters than `max`.
///
/// A cheap, allocation-free rejection for the sentences that make up almost
/// all of an utterance. Sound because normalizing only ever collapses
/// whitespace and lowercases, and `char::to_lowercase` never produces *fewer*
/// characters than it consumes: if a sentence already has more non-whitespace
/// characters than the longest trigger, no normalization of it can equal one.
fn longer_than(s: &str, max: usize) -> bool {
    let mut n = 0usize;
    for c in s.chars() {
        if !c.is_whitespace() {
            n += 1;
            if n > max {
                return true;
            }
        }
    }
    false
}

/// Lowercased, with runs of whitespace collapsed to one space and the ends
/// trimmed. The one canonical form both sides of a comparison go through.
fn normalize_key(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, word) in s.split_whitespace().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.extend(word.chars().flat_map(char::to_lowercase));
    }
    out
}

/// Ends a sentence. `;` and `:` are included because "Address: my address"
/// should expand; `,` is not, because "sign off, and let me know" should not.
fn is_delimiter(c: char) -> bool {
    matches!(
        c,
        '.' | '!' | '?' | ';' | ':' | '\n' | '。' | '！' | '？' | '；' | '：' | '…'
    )
}

/// The subset of [`is_delimiter`] that is punctuation a model added rather
/// than structure the user dictated. Only these are dropped when a trigger
/// ends the utterance — a newline is never eaten.
fn is_sentence_end(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '。' | '！' | '？' | '…')
}

/// One sentence of the utterance: the text up to a delimiter, plus the
/// delimiter itself so `expand` can put it back (or not).
struct Segment<'a> {
    text: &'a str,
    delim: Option<char>,
    /// Byte offset just past `delim`, i.e. where the next segment starts.
    delim_end: usize,
}

/// Splits text into [`Segment`]s. Linear in the length of the text: every byte
/// is looked at once, whatever the number of segments.
struct Segments<'a> {
    rest: &'a str,
    offset: usize,
    done: bool,
}

impl<'a> Segments<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            rest: text,
            offset: 0,
            done: false,
        }
    }
}

impl<'a> Iterator for Segments<'a> {
    type Item = Segment<'a>;

    fn next(&mut self) -> Option<Segment<'a>> {
        if self.done {
            return None;
        }
        match self.rest.char_indices().find(|(_, c)| is_delimiter(*c)) {
            Some((i, c)) => {
                let text = &self.rest[..i];
                self.rest = &self.rest[i + c.len_utf8()..];
                self.offset += i + c.len_utf8();
                Some(Segment {
                    text,
                    delim: Some(c),
                    delim_end: self.offset,
                })
            }
            None => {
                self.done = true;
                let text = self.rest;
                self.offset += text.len();
                self.rest = "";
                Some(Segment {
                    text,
                    delim: None,
                    delim_end: self.offset,
                })
            }
        }
    }
}

// ---- where the file lives -------------------------------------------------

/// `<config dir>/whisper-catch/snippets.txt` — the same directory
/// `config.toml` is in. `None` when the platform has no home directory to
/// look in.
pub fn default_path() -> Option<PathBuf> {
    config_home().map(|d| d.join("whisper-catch").join(SNIPPETS_FILE))
}

/// [`default_path`], except inside this crate's own unit tests, where it is
/// `None`.
///
/// `wc-text` has no business reading a contributor's real snippets file while
/// `cargo test` runs — `PolishConfig::validate()` builds all six transforms,
/// so without this a developer with one malformed entry at home would see
/// tests fail in `lib.rs`. Every test that exercises loading passes an
/// explicit path.
#[cfg(not(test))]
fn default_path_for_load() -> Option<PathBuf> {
    default_path()
}

#[cfg(test)]
fn default_path_for_load() -> Option<PathBuf> {
    None
}

/// Mirrors `dirs::config_dir()` for the two platforms WhisprCatch ships on.
///
/// `apps/cli` finds `config.toml` with the `dirs` crate; `wc-text` deliberately
/// has no dependencies beyond `serde`, and one `$HOME` lookup is not worth
/// growing the build for. Keep the two in step: `apps/cli/src/config.rs`.
#[cfg(target_os = "macos")]
fn config_home() -> Option<PathBuf> {
    home_dir().map(|h| h.join("Library").join("Application Support"))
}

#[cfg(not(target_os = "macos"))]
fn config_home() -> Option<PathBuf> {
    xdg_config_home(
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        home_dir(),
    )
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// The XDG rule `dirs` implements: `$XDG_CONFIG_HOME` when it is an absolute
/// path, `$HOME/.config` otherwise. Split out from the environment lookup so
/// it can be tested without mutating the process environment.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn xdg_config_home(xdg: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    match xdg {
        Some(x) if x.is_absolute() => Some(x),
        _ => home.map(|h| h.join(".config")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{prefix_violation, torture_inputs, truncate};
    use std::time::{Duration, Instant};

    // ---- fixtures ---------------------------------------------------------

    /// The three uses the issue names: an address, a signature block, a
    /// meeting link. Every trigger here is also a phrase that occurs in
    /// ordinary prose, which is the whole point of `trigger_vs_prose`.
    const REAL_FILE: &str = "\
# my snippets

[insert my email]
ada@example.com

[sign off]
Best,
Ada
Founder, WhisprCatch

[standup link]
https://meet.example.com/ada-standup
";

    fn real() -> Snippets {
        let s = Snippets::from_source(REAL_FILE);
        assert!(s.validate().is_empty(), "{:?}", s.validate());
        s
    }

    fn one(trigger: &str, body: &str) -> Snippets {
        Snippets::from_source(&format!("[{trigger}]\n{body}\n"))
    }

    // ---- format -----------------------------------------------------------

    #[test]
    fn parses_trigger_and_single_line_body() {
        let s = real();
        assert_eq!(s.snippets().len(), 3);
        assert_eq!(s.snippets()[0].trigger, "insert my email");
        assert_eq!(s.snippets()[0].body, "ada@example.com");
        assert_eq!(s.snippets()[0].line, 3);
    }

    #[test]
    fn a_multi_line_body_keeps_its_line_breaks() {
        let s = real();
        assert_eq!(s.snippets()[1].body, "Best,\nAda\nFounder, WhisprCatch");
        assert_eq!(s.apply("Sign off."), "Best,\nAda\nFounder, WhisprCatch");
    }

    /// The whole reason snippets are a file and not a config value: a
    /// signature block goes in with its shape intact, blank line and all.
    /// Trailing whitespace inside a line survives too — the standard email
    /// signature separator is literally `-- ` with a space on the end.
    #[test]
    fn multi_line_body_round_trips_through_a_real_file() {
        let body = "-- \nBest,\nAda\n\nWhisprCatch\n  indented line\n";
        let f = TempFile::new("multiline", &format!("[sign off]\n{body}\n[x]\ny\n"));
        let s = Snippets::from_file(f.path());
        assert!(s.validate().is_empty(), "{:?}", s.validate());
        assert_eq!(
            s.apply("Sign off."),
            "-- \nBest,\nAda\n\nWhisprCatch\n  indented line"
        );
        // interior blank line kept, trailing blank line dropped
        assert_eq!(s.apply("Sign off.").lines().count(), 6);
    }

    #[test]
    fn blank_lines_around_a_body_are_layout_not_content() {
        let s = Snippets::from_source("[a]\n\n\nbody\n\n\n[b]\nother\n");
        assert_eq!(s.snippets()[0].body, "body");
        assert_eq!(s.snippets()[1].body, "other");
    }

    #[test]
    fn comments_are_stripped_anywhere_and_escapable_in_a_body() {
        let s = Snippets::from_source("# top\n[a]\n\\# heading\n# dropped\n\\[link]\nend\n");
        assert_eq!(s.snippets()[0].body, "# heading\n[link]\nend");
    }

    #[test]
    fn a_crlf_file_reads_as_a_lf_file() {
        let s = Snippets::from_source("[sign off]\r\nBest,\r\nAda\r\n");
        assert_eq!(s.snippets()[0].body, "Best,\nAda");
        assert_eq!(s.apply("Sign off."), "Best,\nAda");
    }

    #[test]
    fn a_file_without_a_trailing_newline_parses() {
        let s = Snippets::from_source("[a]\nbody");
        assert_eq!(s.snippets()[0].body, "body");
    }

    #[test]
    fn an_indented_bracket_line_is_body_not_a_header() {
        let s = Snippets::from_source("[a]\n  [not a header]\n");
        assert_eq!(s.snippets().len(), 1);
        assert_eq!(s.snippets()[0].body, "  [not a header]");
    }

    #[test]
    fn a_trigger_may_contain_a_closing_bracket() {
        let s = Snippets::from_source("[a]b]\nbody\n");
        assert_eq!(s.snippets()[0].trigger, "a]b");
        assert_eq!(s.apply("A]b."), "body");
    }

    // ---- the corpus: trigger versus prose ---------------------------------

    /// The most important test in the issue. For every trigger form the
    /// transform supports, both directions: it fires when the user says it as
    /// a sentence, and it stays out of the way when the same words turn up
    /// inside one.
    #[test]
    fn trigger_vs_prose() {
        let s = real();

        // fires: the trigger is the whole utterance, or a whole sentence of it
        let fires = [
            ("Sign off.", "Best,\nAda\nFounder, WhisprCatch"),
            ("sign off", "Best,\nAda\nFounder, WhisprCatch"),
            ("Sign off!", "Best,\nAda\nFounder, WhisprCatch"),
            ("Sign off?", "Best,\nAda\nFounder, WhisprCatch"),
            ("Insert my email.", "ada@example.com"),
            ("insert my email", "ada@example.com"),
            ("Standup link.", "https://meet.example.com/ada-standup"),
            (
                "Here you go. Insert my email. Thanks.",
                "Here you go. ada@example.com. Thanks.",
            ),
            (
                "Talk soon. Sign off.",
                "Talk soon. Best,\nAda\nFounder, WhisprCatch",
            ),
            (
                "Sign off. Talk soon.",
                "Best,\nAda\nFounder, WhisprCatch. Talk soon.",
            ),
            ("Ping me: insert my email.", "Ping me: ada@example.com"),
            (
                "Two things; standup link; done.",
                "Two things; https://meet.example.com/ada-standup; done.",
            ),
        ];
        for (input, want) in fires {
            let out = s.apply(input);
            assert_eq!(out, want, "expected {input:?} to expand");
            assert_eq!(s.apply(&out), out, "{input:?} expanded twice");
        }

        // does not fire: the phrase is incidental, part of a longer sentence
        let quiet = [
            "Please sign off on this document.",
            "Please sign off on this document",
            "Can you sign off before Friday?",
            "I will sign off, and then let me know.",
            "sign off on it",
            "Do not sign off.",
            "Sign off the release notes tomorrow.",
            "insert my email address into the form",
            "Please insert my email in the CC line.",
            "The standup link is broken.",
            "Send the standup link to Bob.",
            "signing off",
            "sign offs",
            "sign",
            "off",
            "insert my",
            "my email",
        ];
        for input in quiet {
            assert_eq!(s.apply(input), input, "expected {input:?} to stay put");
        }
    }

    /// A comma does not end a sentence, so a trigger followed by a clause is
    /// still prose. This is the case that decides how safe the whole feature
    /// is: "sign off, and let me know" is a request, not a signature.
    #[test]
    fn a_comma_does_not_start_a_new_sentence() {
        let s = real();
        assert_eq!(
            s.apply("Sign off, and let me know."),
            "Sign off, and let me know."
        );
        assert_eq!(s.apply("Yes, sign off, please."), "Yes, sign off, please.");
    }

    #[test]
    fn matching_ignores_case_and_extra_whitespace() {
        let s = real();
        let want = "Best,\nAda\nFounder, WhisprCatch";
        for input in [
            "Sign off.",
            "SIGN OFF.",
            "sIgN OfF.",
            "sign  \t off.",
            " sign off ",
        ] {
            assert!(
                s.apply(input).contains(want),
                "{input:?} -> {:?}",
                s.apply(input)
            );
        }
    }

    #[test]
    fn a_newline_ends_a_sentence_and_survives_the_expansion() {
        let s = real();
        assert_eq!(
            s.apply("first line\nsign off\nlast line"),
            "first line\nBest,\nAda\nFounder, WhisprCatch\nlast line"
        );
    }

    // ---- punctuation around a match ---------------------------------------

    /// The stop the model added after a trigger that ended the utterance goes
    /// away with the trigger — an email address with a full stop welded to it
    /// is a broken link in most apps. Mid-utterance it stays, because it is
    /// still separating two sentences.
    #[test]
    fn the_final_full_stop_goes_with_the_trigger() {
        let s = real();
        assert_eq!(s.apply("Insert my email."), "ada@example.com");
        assert_eq!(s.apply("Insert my email.  "), "ada@example.com  ");
        assert_eq!(s.apply("Insert my email.\n"), "ada@example.com\n");
        assert_eq!(s.apply("Insert my email!"), "ada@example.com");
        assert_eq!(
            s.apply("Insert my email. Thanks."),
            "ada@example.com. Thanks."
        );
        // a newline is structure the user dictated, never punctuation to eat
        assert_eq!(s.apply("Insert my email\n"), "ada@example.com\n");
        // and so is a semicolon or a colon
        assert_eq!(s.apply("Insert my email;"), "ada@example.com;");
    }

    #[test]
    fn a_trigger_may_span_punctuation_that_does_not_end_a_sentence() {
        let s = one("hey, there", "Hi!");
        assert!(s.validate().is_empty());
        assert_eq!(s.apply("Hey, there."), "Hi!");
        assert_eq!(s.apply("Hey, there, you."), "Hey, there, you.");
    }

    #[test]
    fn a_trigger_fires_at_the_very_start_and_the_very_end() {
        let s = real();
        assert_eq!(
            s.apply("Sign off. Then send it. Standup link."),
            "Best,\nAda\nFounder, WhisprCatch. Then send it. https://meet.example.com/ada-standup"
        );
    }

    #[test]
    fn two_triggers_can_be_adjacent() {
        let s = real();
        assert_eq!(
            s.apply("Insert my email. Standup link."),
            "ada@example.com. https://meet.example.com/ada-standup"
        );
        assert_eq!(
            s.apply("Insert my email.Standup link."),
            "ada@example.com.https://meet.example.com/ada-standup"
        );
    }

    // ---- overlapping triggers ---------------------------------------------

    /// Whole-sentence matching makes "longest match wins" free rather than
    /// something to implement: a sentence is compared against a trigger whole,
    /// so "sign" cannot claim the "sign" in "sign off". Asserted anyway,
    /// because the day someone switches to substring matching this is the test
    /// that should stop them.
    #[test]
    fn the_longest_trigger_wins_when_one_is_a_prefix_of_another() {
        let s = Snippets::from_source(
            "[sign]\nSHORT\n[sign off]\nLONG\n[sign off now]\nLONGEST\n[my]\nM\n",
        );
        assert!(s.validate().is_empty(), "{:?}", s.validate());
        assert_eq!(s.apply("Sign off now."), "LONGEST");
        assert_eq!(s.apply("Sign off."), "LONG");
        assert_eq!(s.apply("Sign."), "SHORT");
        assert_eq!(s.apply("Sign off later."), "Sign off later.");
        // and the short one never eats a word out of the middle of the long one
        assert_eq!(s.apply("Sign off now please."), "Sign off now please.");
    }

    // ---- idempotence and termination --------------------------------------

    #[test]
    fn apply_is_idempotent_on_the_torture_corpus() {
        // triggers deliberately chosen to hit the corpus: "hello world" and
        // the Japanese and Cyrillic lines are whole sentences in it
        let s = Snippets::from_source(
            "[hello world]\nHELLO\n[日本語のテキストです]\nJA\n\
             [Правда — это не то, что кажется]\nRU\n[um, I mean, like, the thing]\nTHING\n",
        );
        assert!(s.validate().is_empty(), "{:?}", s.validate());
        for input in torture_inputs() {
            let once = s.apply(&input);
            assert_eq!(
                s.apply(&once),
                once,
                "not idempotent on {:?}",
                truncate(&input)
            );
        }
    }

    /// The trap the issue calls out. A body that contains its own trigger — or
    /// another entry's — would expand again on the second pass, so it is
    /// reported and disabled instead. That is what makes termination a
    /// property of the data rather than of a recursion limit.
    #[test]
    fn a_body_containing_a_trigger_is_reported_and_disabled() {
        // its own trigger, on a line of its own inside the body
        let s = Snippets::from_source("[loop]\nbefore\nloop\nafter\n");
        assert_eq!(s.validate().len(), 1);
        assert!(
            s.validate()[0].contains("do not nest"),
            "{:?}",
            s.validate()
        );
        assert_eq!(s.apply("Loop."), "Loop.");

        // a mutual cycle: both are disabled, neither expands
        let s = Snippets::from_source("[a]\nb\n[b]\na\n");
        assert_eq!(s.validate().len(), 2, "{:?}", s.validate());
        assert_eq!(s.apply("A. B."), "A. B.");

        // a one-way reference: only the referring entry is disabled
        let s = Snippets::from_source(
            "[sign off]\nBest,\ninsert my email\n[insert my email]\nada@example.com\n",
        );
        assert_eq!(s.validate().len(), 1, "{:?}", s.validate());
        assert_eq!(s.apply("Sign off."), "Sign off.");
        assert_eq!(s.apply("Insert my email."), "ada@example.com");
    }

    /// A body that merely *mentions* a trigger inside a longer sentence is
    /// fine: it can never match, so it can never re-expand.
    #[test]
    fn a_body_that_mentions_a_trigger_mid_sentence_is_not_nesting() {
        let s = Snippets::from_source("[a]\nplease sign off on this\n[sign off]\nBest,\nAda\n");
        assert!(s.validate().is_empty(), "{:?}", s.validate());
        assert_eq!(s.apply("A."), "please sign off on this");
        assert_eq!(s.apply(&s.apply("A.")), s.apply("A."));
    }

    /// Expansion must not create a trigger out of the text around it either.
    #[test]
    fn expansion_does_not_create_a_new_trigger_at_the_seam() {
        // "Ha!" introduces a sentence boundary the input did not have
        let s = Snippets::from_source("[a]\nHa!\n[ha]\nNO\n");
        // "Ha!" contains "Ha" as a whole sentence, so [a] is disabled
        assert_eq!(s.validate().len(), 1, "{:?}", s.validate());
        assert_eq!(s.apply("A. b."), "A. b.");

        // with the collision removed, the seam is stable
        let s = Snippets::from_source("[a]\nHa!\n");
        assert!(s.validate().is_empty());
        let once = s.apply("A. b.");
        assert_eq!(once, "Ha!. b.");
        assert_eq!(s.apply(&once), once);
    }

    // ---- disabled ---------------------------------------------------------

    #[test]
    fn disabled_is_byte_identical() {
        let f = TempFile::new("disabled", REAL_FILE);
        let s = Snippets::new(SnippetsConfig {
            enabled: false,
            path: Some(f.path().to_path_buf()),
        });
        assert_eq!(s.snippets().len(), 3, "the file still loads for Settings");
        for input in torture_inputs()
            .into_iter()
            .chain(["Sign off.".into(), "Insert my email.".into()])
        {
            assert_eq!(s.apply(&input), input, "changed {:?}", truncate(&input));
        }
    }

    #[test]
    fn disabled_by_default() {
        assert!(!SnippetsConfig::default().enabled);
        assert!(SnippetsConfig::default().path.is_none());
    }

    #[test]
    fn an_empty_file_changes_nothing() {
        for source in ["", "\n\n", "# only a comment\n"] {
            let s = Snippets::from_source(source);
            assert!(s.is_empty());
            assert!(s.validate().is_empty());
            for input in torture_inputs() {
                assert_eq!(s.apply(&input), input, "changed {:?}", truncate(&input));
            }
        }
    }

    // ---- adversarial bodies -----------------------------------------------

    #[test]
    fn an_empty_body_deletes_the_trigger() {
        let s = Snippets::from_source("[scratch that]\n\n[keep]\nkept\n");
        assert!(s.validate().is_empty(), "{:?}", s.validate());
        assert_eq!(s.snippets()[0].body, "");
        assert_eq!(s.apply("Scratch that."), "");
        assert_eq!(s.apply("Yes. Scratch that. No."), "Yes. . No.");
        // and it stays deleted
        assert_eq!(s.apply(&s.apply("Scratch that.")), s.apply("Scratch that."));
    }

    #[test]
    fn a_very_large_body_expands_once_and_stays_put() {
        let body = "x".repeat(1_000_000);
        let s = one("big one", &body);
        let out = s.apply("Big one. Done.");
        assert_eq!(out.len(), body.len() + ". Done.".len());
        assert!(out.starts_with("xxxx"));
        assert!(out.ends_with(". Done."));
        assert_eq!(s.apply(&out), out);
    }

    #[test]
    fn unicode_triggers_match_by_case_folded_content() {
        // CJK: the full stop is a delimiter and is dropped at the end
        let s = one("日本語のテキストです", "JA");
        assert_eq!(s.apply("日本語のテキストです。"), "JA");
        assert_eq!(
            s.apply("日本語のテキストですね。"),
            "日本語のテキストですね。"
        );

        // Cyrillic, case-insensitively
        let s = one("правда", "RU");
        assert_eq!(s.apply("Правда."), "RU");
        assert_eq!(s.apply("ПРАВДА"), "RU");

        // a character whose lowercase is a different length in bytes (KELVIN
        // SIGN) and one whose lowercase is two characters (LATIN CAPITAL I
        // WITH DOT ABOVE) — both must survive the length guard in `lookup`
        let s = one("k", "KELVIN");
        assert_eq!(s.apply("\u{212a}"), "KELVIN");
        let s = one("\u{130}", "DOTTED");
        assert_eq!(s.apply("\u{130}"), "DOTTED");
        assert_eq!(s.apply("i\u{307}"), "DOTTED");

        // emoji, including a ZWJ sequence and a skin-tone modifier
        let s = one("👩‍💻 shipped it 🚀 👍🏽", "SHIPPED");
        assert_eq!(s.apply("👩‍💻 shipped it 🚀 👍🏽"), "SHIPPED");

        // zero-width characters are content, not whitespace: they are not
        // stripped, so they do not silently make a trigger match
        let s = one("zerowidth", "ZW");
        assert_eq!(
            s.apply("\u{200b}zero\u{200b}width\u{200b}"),
            "\u{200b}zero\u{200b}width\u{200b}"
        );
    }

    #[test]
    fn combining_marks_are_compared_as_written() {
        // no NFC/NFD normalization: "é" precomposed is not "e" + combining
        // acute. Both directions asserted so the day someone adds
        // normalization, they see the decision they are changing.
        let s = one("e\u{301}gal", "COMBINING");
        assert_eq!(s.apply("e\u{301}gal"), "COMBINING");
        assert_eq!(s.apply("égal"), "égal");
    }

    // ---- validation -------------------------------------------------------

    #[test]
    fn validate_reports_empty_and_whitespace_only_triggers() {
        let s = Snippets::from_source("[]\nbody\n[   ]\nbody\n[\t]\nbody\n[ok]\nfine\n");
        let problems = s.validate();
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("line 1"), "{problems:?}");
        assert!(problems[0].contains("empty trigger"), "{problems:?}");
        assert!(problems[1].contains("only whitespace"), "{problems:?}");
        assert!(problems[2].contains("line 5"), "{problems:?}");
        // the sound entry still works
        assert_eq!(s.apply("Ok."), "fine");
    }

    #[test]
    fn validate_reports_duplicate_triggers_and_keeps_the_first() {
        let s = Snippets::from_source("[sign off]\nFIRST\n[Sign  Off]\nSECOND\n");
        let problems = s.validate();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("duplicate"), "{problems:?}");
        assert!(problems[0].contains("line 3"), "{problems:?}");
        assert!(problems[0].contains("line 1"), "{problems:?}");
        assert_eq!(s.apply("Sign off."), "FIRST");
    }

    /// A trigger written with a full stop in it can never match a whole
    /// sentence, because the full stop is where sentences are cut. Silently
    /// never firing is the worst possible outcome, so it is an error.
    #[test]
    fn validate_reports_a_trigger_that_could_never_match() {
        for bad in ["sign off.", "e.g", "one: two", "a?b", "x…y"] {
            let s = one(bad, "body");
            let problems = s.validate();
            assert_eq!(problems.len(), 1, "{bad:?} -> {problems:?}");
            assert!(problems[0].contains("never match"), "{problems:?}");
            assert!(s.is_empty());
        }
    }

    #[test]
    fn validate_reports_text_before_the_first_header() {
        let s = Snippets::from_source("ada@example.com\n[insert my email]\nada@example.com\n");
        let problems = s.validate();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("line 1"), "{problems:?}");
        assert!(problems[0].contains("ignored"), "{problems:?}");
        // reported once, however many stray lines there are
        let s = Snippets::from_source("one\ntwo\nthree\n[a]\nb\n");
        assert_eq!(s.validate().len(), 1, "{:?}", s.validate());
    }

    #[test]
    fn a_valid_file_reports_nothing() {
        assert!(Snippets::from_source(REAL_FILE).validate().is_empty());
        assert!(Snippets::from_source("").validate().is_empty());
    }

    /// Every problem `validate` reports must also mean the entry cannot fire.
    /// Reporting a malformed snippet and expanding it anyway would be the
    /// worst of both worlds.
    #[test]
    fn a_reported_entry_never_expands() {
        let s = Snippets::from_source(
            "[]\nX\n[  ]\nX\n[dup]\nONE\n[dup]\nTWO\n[bad.]\nX\n[nest]\ndup\n",
        );
        assert_eq!(s.validate().len(), 5, "{:?}", s.validate());
        assert_eq!(s.apply("Dup."), "ONE"); // the first of the duplicates
        for input in ["X.", "Bad.", "Nest.", "."] {
            assert_eq!(s.apply(input), input, "{input:?} expanded");
        }
    }

    // ---- loading from disk ------------------------------------------------

    /// A file that is not there is the normal state of a fresh install, not a
    /// problem to shout about in Settings.
    #[test]
    fn a_missing_file_means_no_snippets_and_no_complaint() {
        let mut path = std::env::temp_dir();
        path.push("wc-text-snippets-does-not-exist-4711.txt");
        let _ = std::fs::remove_file(&path);
        let s = Snippets::new(SnippetsConfig {
            enabled: true,
            path: Some(path),
        });
        assert!(s.is_empty());
        assert!(s.validate().is_empty());
        assert_eq!(s.apply("Sign off."), "Sign off.");
    }

    #[test]
    fn a_configured_path_is_read_and_used() {
        let f = TempFile::new("configured", REAL_FILE);
        let s = Snippets::new(SnippetsConfig {
            enabled: true,
            path: Some(f.path().to_path_buf()),
        });
        assert!(s.validate().is_empty(), "{:?}", s.validate());
        assert_eq!(s.apply("Insert my email."), "ada@example.com");
    }

    /// A file that exists but cannot be read is different from one that is not
    /// there: the user wrote snippets and they are not loading, so say so.
    #[test]
    fn an_unreadable_file_is_reported() {
        let f = TempFile::new_bytes("not-utf8", &[b'[', b'a', b']', b'\n', 0xff, 0xfe]);
        let s = Snippets::from_file(f.path());
        assert_eq!(s.validate().len(), 1, "{:?}", s.validate());
        assert!(
            s.validate()[0].contains("cannot read"),
            "{:?}",
            s.validate()
        );
        assert!(s.is_empty());
        assert_eq!(s.apply("A."), "A.");
    }

    /// The production wiring, end to end: `PolishConfig` → `Polish` →
    /// `Snippets::new` → the file on disk.
    #[test]
    fn the_chain_runs_snippets_from_a_configured_file() {
        let f = TempFile::new("chain", REAL_FILE);
        let cfg = crate::PolishConfig {
            snippets: SnippetsConfig {
                enabled: true,
                path: Some(f.path().to_path_buf()),
            },
            ..Default::default()
        };
        assert_eq!(cfg.validate(), Vec::<String>::new());

        let p = crate::Polish::from_config(&cfg);
        assert_eq!(p.names(), ["snippets"]);
        assert!(p.has_rewriting_transforms(), "streaming must be warned");
        assert_eq!(p.apply("Sign off."), "Best,\nAda\nFounder, WhisprCatch");
        // and the prefix-stable subset still runs nothing
        assert_eq!(p.apply_prefix_stable("Sign off."), "Sign off.");
    }

    #[test]
    fn the_default_path_sits_next_to_config_toml() {
        // resolved from the environment, so only its shape is assertable
        if let Some(p) = default_path() {
            assert!(p.is_absolute(), "{p:?}");
            assert!(p.ends_with("whisper-catch/snippets.txt"), "{p:?}");
        }
        // the XDG rule `dirs` implements, without touching the environment
        assert_eq!(
            xdg_config_home(Some("/xdg".into()), Some("/home/ada".into())),
            Some(PathBuf::from("/xdg"))
        );
        assert_eq!(
            xdg_config_home(Some("relative".into()), Some("/home/ada".into())),
            Some(PathBuf::from("/home/ada/.config"))
        );
        assert_eq!(
            xdg_config_home(None, Some("/home/ada".into())),
            Some(PathBuf::from("/home/ada/.config"))
        );
        assert_eq!(xdg_config_home(None, None), None);
    }

    // ---- prefix stability -------------------------------------------------

    #[test]
    fn is_not_prefix_stable() {
        assert!(!Snippets::from_source(REAL_FILE).prefix_stable());
        assert!(!Snippets::new(SnippetsConfig::default()).prefix_stable());
    }

    /// Run against the real implementation, not the stub: the counterexample
    /// `prefix_violation` finds is a trigger that straddles the streaming
    /// boundary. The streaming pass has already typed "Hello. Sign o"; the
    /// finished utterance polishes to "Hello. Best,\nAda", which does not
    /// start with it, so six characters would have to be retracted — which the
    /// injector cannot do until #41 lands.
    #[test]
    fn prefix_violation_finds_a_straddling_trigger() {
        let s = one("sign off", "Best,\nAda");
        let (prefix, polished_prefix, polished_whole) =
            prefix_violation(&s, "Hello. Sign off.").expect("snippets cannot be prefix-stable");
        assert_eq!(prefix, "Hello. S");
        assert_eq!(polished_prefix, "Hello. S");
        assert_eq!(polished_whole, "Hello. Best,\nAda");
    }

    /// The other half of the same story: even a prefix that ends exactly on a
    /// completed trigger is unsafe, because the model's punctuation moves.
    #[test]
    fn prefix_violation_also_fires_on_a_completed_trigger() {
        let s = one("sign off", "Best,\nAda");
        assert!(prefix_violation(&s, "Sign off. Bye.").is_some());
    }

    // ---- composition with the custom dictionary (#43) ----------------------

    /// `Polish` runs `dictionary` before `snippets`, so a dictionary rule
    /// rewrites words *inside* a trigger phrase before snippets ever see it.
    /// Documented here as a runnable example rather than a paragraph, with a
    /// stand-in for #43 so the two issues can land in either order.
    #[test]
    fn a_dictionary_rule_reaches_inside_a_trigger() {
        use crate::{BoxedTransform, Polish};

        /// Word-boundary-anchored substitution, which is what #43 does.
        struct WordRule(&'static str, &'static str);
        impl WordRule {
            fn flush(&self, word: &mut String, out: &mut String) {
                if word.eq_ignore_ascii_case(self.0) {
                    out.push_str(self.1);
                } else {
                    out.push_str(word);
                }
                word.clear();
            }
        }
        impl Transform for WordRule {
            fn name(&self) -> &'static str {
                "dictionary"
            }
            fn apply(&self, text: &str) -> String {
                let mut out = String::new();
                let mut word = String::new();
                for c in text.chars() {
                    if c.is_alphanumeric() {
                        word.push(c);
                    } else {
                        self.flush(&mut word, &mut out);
                        out.push(c);
                    }
                }
                self.flush(&mut word, &mut out);
                out
            }
            fn prefix_stable(&self) -> bool {
                false
            }
        }

        let snips = || Snippets::from_source(REAL_FILE);

        // desirable: the dictionary repairs a trigger the model misheard, and
        // the snippet then fires on text the user never actually said
        let chain = Polish::from_transforms(vec![
            Box::new(WordRule("of", "off")) as BoxedTransform,
            Box::new(snips()),
        ]);
        assert_eq!(chain.apply("Sign of."), "Best,\nAda\nFounder, WhisprCatch");

        // undesirable, and the reason this is written down: a rule the user
        // added for prose ("email" -> "e-mail") silently stops a trigger that
        // contains the same word from ever matching, with no error anywhere
        let chain = Polish::from_transforms(vec![
            Box::new(WordRule("email", "e-mail")) as BoxedTransform,
            Box::new(snips()),
        ]);
        assert_eq!(chain.apply("Insert my email."), "Insert my e-mail.");
        // case is not the failure mode: matching folds case, so a rule that
        // only changes capitalisation leaves the trigger working
        let chain = Polish::from_transforms(vec![
            Box::new(WordRule("email", "EMAIL")) as BoxedTransform,
            Box::new(snips()),
        ]);
        assert_eq!(chain.apply("Insert my email."), "ada@example.com");

        // the reverse order would be worse: the dictionary would rewrite the
        // user's own saved snippet text, which they already typed the way they
        // want it
        let reversed = Polish::from_transforms(vec![
            Box::new(snips()) as BoxedTransform,
            Box::new(WordRule("ada", "Ada Lovelace")),
        ]);
        assert_eq!(
            reversed.apply("Sign off."),
            "Best,\nAda Lovelace\nFounder, WhisprCatch"
        );
    }

    // ---- cost -------------------------------------------------------------

    /// The number the issue asks for, measured rather than asserted: the
    /// ceiling is deliberately far above the real figure so a loaded CI runner
    /// cannot make it flake. Run with `--nocapture` to see the measurement.
    #[test]
    fn expansion_cost_on_a_realistic_utterance() {
        // ~60 words, the length of a long dictated paragraph, with one real
        // expansion in it
        let utterance = "Thanks for the update this morning. I read through the plan and it \
             looks right to me. Let us keep the scope where it is for now and revisit the \
             rest after the release goes out. Trigger number 7. I will send the notes over \
             later today so everyone has them before the standup tomorrow.";

        // 50 is a realistic library; 500 is the ceiling #43 sizes the
        // dictionary against. Cost should barely move between them — the work
        // is one hash lookup per sentence, not one comparison per snippet.
        for count in [50, 500] {
            let mut file = String::new();
            for i in 0..count {
                file.push_str(&format!("[trigger number {i}]\nbody number {i}\n"));
            }
            let s = Snippets::from_source(&file);
            assert!(s.validate().is_empty(), "{:?}", s.validate());
            assert!(s.apply(utterance).contains("body number 7"));

            let runs = 2_000;
            let start = Instant::now();
            for _ in 0..runs {
                std::hint::black_box(s.apply(std::hint::black_box(utterance)));
            }
            let per = start.elapsed() / runs;
            println!(
                "snippets: {per:?} per utterance, {count} snippets, {} chars",
                utterance.len()
            );
            assert_cost(per);
        }
    }

    fn assert_cost(per: Duration) {
        assert!(
            per < Duration::from_millis(2),
            "{per:?} per utterance is far above the measured cost — something turned quadratic"
        );
    }

    /// The 2 MB entry in the torture corpus would take minutes if matching
    /// were quadratic in the utterance. Bounded so a regression fails the
    /// suite instead of hanging it.
    #[test]
    fn a_two_megabyte_utterance_stays_linear() {
        let s = real();
        let big = "the quick brown fox jumps over the lazy dog. ".repeat(45_000);
        let start = Instant::now();
        let out = s.apply(&big);
        assert_eq!(out, big);
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
    }

    // ---- config -----------------------------------------------------------

    #[test]
    fn config_round_trips_through_toml() {
        let text = toml::to_string_pretty(&SnippetsConfig::default()).unwrap();
        assert!(
            !text.contains("path"),
            "an unset path must not be written: {text}"
        );
        let back: SnippetsConfig = toml::from_str(&text).unwrap();
        assert!(!back.enabled);
        assert!(back.path.is_none());

        let cfg = SnippetsConfig {
            enabled: true,
            path: Some("/home/ada/dotfiles/snippets.txt".into()),
        };
        let back: SnippetsConfig = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert!(back.enabled);
        assert_eq!(back.path, cfg.path);
    }

    #[test]
    fn a_config_without_the_path_key_still_loads() {
        let cfg: SnippetsConfig = toml::from_str("enabled = true\n").unwrap();
        assert!(cfg.enabled);
        assert!(cfg.path.is_none());
    }

    // ---- helpers ----------------------------------------------------------

    /// A file in the temp directory that deletes itself. Only the handful of
    /// tests that exercise loading touch the disk; everything else goes
    /// through `from_source`.
    struct TempFile(PathBuf);

    impl TempFile {
        fn new(name: &str, contents: &str) -> Self {
            Self::new_bytes(name, contents.as_bytes())
        }

        fn new_bytes(name: &str, contents: &[u8]) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let mut path = std::env::temp_dir();
            path.push(format!(
                "wc-text-snippets-{}-{name}-{nanos}.txt",
                std::process::id()
            ));
            std::fs::write(&path, contents).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}
