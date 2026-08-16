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
//! - A body line that has to start with `[`, `#` or `\` is escaped with a
//!   backslash: `\[draft]`, `\# heading`, `\\[not a header]`.
//! - **Leave a blank line between entries.** A `[...]` line directly under
//!   body text is ambiguous — bracketed placeholders like `[Auto-reply]` are
//!   exactly how people write canned replies — so it is read as a new entry
//!   *and* reported, because the alternative is silently eating the rest of
//!   somebody's signature.
//! - `\r\n` files are read as if they were `\n` files, and a UTF-8 BOM is
//!   ignored, so a file written by Notepad or synced from another machine
//!   still works.
//! - **An entry with no text under it is reported and switched off.** It would
//!   expand its trigger to nothing — that is, saying the phrase would *delete*
//!   the sentence the user just dictated — and that is almost always a header
//!   nobody meant to write rather than a feature anybody wanted.
//!
//! # Matching: whole phrase, never mid-sentence
//!
//! This is the one thing that separates snippets from the custom dictionary
//! (#43). The dictionary rewrites *words* wherever they appear. A snippet
//! fires only when its trigger is an **entire sentence** of the utterance —
//! the text between one sentence boundary (`.` `!` `?` `…`, a newline, or
//! their CJK equivalents) and the next.
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
//! **`,` `;` and `:` do not end a sentence.** They are the punctuation of a
//! clause, not of a command, and the cases they would break are ordinary
//! written English: `"Sign off, and let me know."`, `"Next: sign off."`,
//! `"Owner: sign off; Reviewer: Ada."`. An earlier revision split on `;` and
//! `:` so that `"Address: insert my address"` would expand; that convenience
//! is not worth mangling a sentence somebody dictated. The cost is real and
//! stated here so it is a decision: a label-then-value dictation only expands
//! if the user says the trigger as its own sentence, or on its own line.
//!
//! Comparison collapses runs of whitespace and lowercases both sides with
//! Rust's `str::to_lowercase`, so "Sign  off" and "SIGN OFF" both fire. That
//! is simple Unicode lowercasing and nothing more: there is no NFC/NFD
//! normalization, no width folding and no locale-aware casing, so `"SİGN
//! OFF."` (Turkish dotted capital I) and fullwidth `"ｓｉｇｎ　ｏｆｆ"` stay
//! quiet. `unicode_triggers_match_by_case_folded_content` and
//! `case_folding_is_simple_lowercasing_only` pin both halves.
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
//! executable version of all three cases. There is a fourth, and it is the
//! dangerous one — see the note on composition under *Idempotence* below.
//!
//! # A multi-line body is advisory, not free (#73)
//!
//! The signature block is the reason people want this feature, and it is also
//! the one shape the injector cannot type safely yet: with no clipboard
//! backend (#68), a newline goes out as a Return key, which *sends the
//! message* in Slack, Discord and most chat boxes. Nothing in a pure text
//! crate can fix that, so [`Transform::validate`] warns on every multi-line
//! body and leaves it enabled — it is correct in any editor, and a user should
//! learn the limit from Settings rather than from a customer thread. The first
//! "Done when" of #47 is not met until #73 lands.
//!
//! # Idempotence
//!
//! **For this transform in isolation**, `apply(apply(x)) == apply(x)`, and the
//! proof is structural rather than a loop with a counter: an entry whose body
//! contains any trigger as a whole sentence is reported by
//! [`Transform::validate`] and **disabled**, so no expansion can ever produce
//! text that a later pass would expand again. Snippets do not nest.
//!
//! **The chain is a different question, and composition can break it.** The
//! nesting guard inspects bodies as written, at load time — it cannot see a
//! body that only *becomes* a trigger once the dictionary has run:
//!
//! ```text
//! dictionary: "regards"  -> "sign off"
//! snippet:    "sign off" -> "Best,\nregards\nAda"
//!
//! chain.apply("Sign off.") == "Best,\nregards\nAda"
//! chain.apply(that)        == "Best,\nBest,\nregards\nAda\nAda"   // grows
//! ```
//!
//! `validate()` reports nothing, because in isolation both files are fine.
//! Production is safe today — `finish()` in `apps/cli` applies the chain
//! exactly once — but `lib.rs::applying_the_chain_twice_changes_nothing`
//! asserts the invariant for the whole chain, and #49's "preview, then
//! dictate" is a documented double-apply. Whoever builds that surface owns the
//! cross-file check; `composition_with_a_dictionary_can_break_idempotence` is
//! the failing case, written down so it is inherited rather than rediscovered.

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

/// How much a problem costs the user. [`Transform::validate`] flattens this to
/// strings because that is the trait's shape, but the distinction is real and
/// tests assert on it: a fault means an entry is inert, an advisory means it
/// works and there is something the user should know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Fault,
    Advisory,
}

#[derive(Debug, Clone)]
struct Problem {
    /// Line of the `[trigger]` header this is about; 0 for the file itself.
    line: usize,
    severity: Severity,
    message: String,
}

impl Problem {
    fn fault(line: usize, message: String) -> Self {
        Self {
            line,
            severity: Severity::Fault,
            message,
        }
    }

    fn advisory(line: usize, message: String) -> Self {
        Self {
            line,
            severity: Severity::Advisory,
            message,
        }
    }
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
    /// `(line number, message)`. Kept with the line so [`Transform::validate`]
    /// can hand #49 a list in file order however many passes produced it;
    /// line 0 means "about the file as a whole".
    problems: Vec<Problem>,
}

impl Snippets {
    /// Loads the snippets file named by `cfg` (or the default location).
    ///
    /// This is the only place in the crate that touches the filesystem, and it
    /// happens once, when the chain is built. [`Transform::apply`] stays a
    /// pure function of its input.
    pub fn new(cfg: SnippetsConfig) -> Self {
        let mut loaded = match cfg.path.clone() {
            Some(p) => Self::from_file(&p),
            None => match default_path_for_load() {
                DefaultPath::Found(p) => Self::from_file(&p),
                DefaultPath::NoHome => {
                    let mut s = Self::from_source("");
                    s.problems.push(Problem::fault(
                        0,
                        "snippets: no usable $HOME, so there is nowhere to look for \
                         snippets.txt — set HOME, or point [polish.snippets] path at the file"
                            .to_string(),
                    ));
                    s
                }
                DefaultPath::Skipped => Self::from_source(""),
            },
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
                s.problems.push(Problem::fault(
                    0,
                    format!("snippets: cannot read {}: {e}", path.display()),
                ));
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
        // A file written by Notepad or VS Code as "UTF-8 with BOM" starts
        // EF BB BF. U+FEFF is not whitespace, so without this the first
        // `[trigger]` line is not a header, the entry is lost, and the user is
        // told there is stray text before the first header. One character.
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let (snippets, mut problems) = parse(source);
        let (by_trigger, max_trigger_chars, more) = compile(&snippets);
        problems.extend(more);
        // Two passes produced these; #49 renders them next to the file, so
        // they come out in the order the user reads. Stable, so problems about
        // the same line stay in the order they were found.
        problems.sort_by_key(|p| p.line);
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

    /// Problems with the file, in file order, each naming the line so Settings
    /// can jump to it.
    ///
    /// Two kinds, and the distinction matters:
    ///
    /// - **Rejections** — empty, whitespace-only or duplicate triggers, a
    ///   trigger containing sentence punctuation, an empty body, a body that
    ///   contains another trigger. The entry is inert; a snippet the app
    ///   cannot make sense of must never fire. `a_reported_entry_never_expands`
    ///   pins that.
    /// - **Advisories** — a `[...]` line directly under body text (the file has
    ///   two readings and only the user knows which is right; a compactly
    ///   written file is a false positive and dropping every entry in it would
    ///   be far worse than a warning), and a multi-line body, which works
    ///   everywhere except the apps #73 breaks. Advisories never disable
    ///   anything.
    fn validate(&self) -> Vec<String> {
        self.problems
            .iter()
            .map(|p| match p.severity {
                Severity::Fault => p.message.clone(),
                // Marked in the text, because `Vec<String>` is all the trait
                // gives us and a list that mixes "this entry is switched off"
                // with "this entry works, but read this" is worse than useless
                // to the person reading it.
                Severity::Advisory => format!("note: {}", p.message),
            })
            .collect()
    }
}

// ---- parsing --------------------------------------------------------------

/// Splits a snippets file into entries. The second half of the pair is the
/// problems the *parse* can see; [`compile`] finds the rest.
fn parse(source: &str) -> (Vec<Snippet>, Vec<Problem>) {
    let mut snippets: Vec<Snippet> = Vec::new();
    let mut problems: Vec<Problem> = Vec::new();
    // (trigger, header line number, body lines so far)
    let mut pending: Option<(String, usize, Vec<String>)> = None;
    let mut warned_about_preamble = false;

    for (i, line) in source.lines().enumerate() {
        let line_no = i + 1;
        if let Some(trigger) = header_trigger(line) {
            if let Some((t, n, body)) = pending.take() {
                // `[Auto-reply]` and `[Founder, WhisprCatch]` are how people
                // actually write canned replies and signatures, and read as
                // headers here. Taking the reading silently would truncate the
                // snippet above *and* invent a trigger that fires on ordinary
                // dictation — and with an empty body, one that deletes the
                // sentence the user just spoke. Ambiguous, so: say so.
                if body.last().is_some_and(|l| !l.trim().is_empty()) {
                    problems.push(Problem::advisory(
                        line_no,
                        format!(
                            "snippets line {line_no}: {line:?} sits directly under body text, so \
                             it is read as a new snippet — leave a blank line above it, or write \
                             it as \\{line} if it belongs to the snippet above"
                        ),
                    ));
                }
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
                    problems.push(Problem::fault(
                        line_no,
                        format!(
                            "snippets line {line_no}: text before the first [trigger] line \
                             is ignored"
                        ),
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

/// Un-escapes a body line that had to start with `[`, `#` or a backslash of
/// its own. `\\[link]` is how a body line that really begins `\[link]` is
/// written; without the third case that line could not be expressed at all.
fn unescape(line: &str) -> &str {
    match line.strip_prefix('\\') {
        Some(rest) if rest.starts_with(['[', '#', '\\']) => rest,
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
fn compile(snippets: &[Snippet]) -> (HashMap<String, usize>, usize, Vec<Problem>) {
    let mut by_trigger: HashMap<String, usize> = HashMap::new();
    let mut problems: Vec<Problem> = Vec::new();

    for (idx, s) in snippets.iter().enumerate() {
        if s.trigger.is_empty() {
            problems.push(Problem::fault(
                s.line,
                format!("snippets line {}: empty trigger, `[]`", s.line),
            ));
            continue;
        }
        let key = normalize_key(&s.trigger);
        if key.is_empty() {
            problems.push(Problem::fault(
                s.line,
                format!("snippets line {}: trigger is only whitespace", s.line),
            ));
            continue;
        }
        if let Some(bad) = s.trigger.chars().find(|c| is_delimiter(*c)) {
            problems.push(Problem::fault(
                s.line,
                format!(
                    "snippets line {}: trigger {:?} contains {bad:?}, which ends a sentence, \
                     so the trigger can never match a whole one — remove it",
                    s.line, s.trigger
                ),
            ));
            continue;
        }
        // An entry with no text under it expands to nothing, which means
        // saying the trigger *deletes* the sentence the user dictated. That is
        // almost always a header the file's author did not mean to write — an
        // unescaped `[Auto-reply]` line, or a stub they never filled in — and
        // deleting somebody's words is a worse outcome than not expanding.
        if s.body.is_empty() {
            problems.push(Problem::fault(
                s.line,
                format!(
                    "snippets line {}: {:?} has no text under it, so it would delete the \
                     phrase rather than expand it — this entry is disabled",
                    s.line, s.trigger
                ),
            ));
            continue;
        }
        match by_trigger.entry(key) {
            Entry::Vacant(v) => {
                v.insert(idx);
            }
            Entry::Occupied(o) => problems.push(Problem::fault(
                s.line,
                format!(
                    "snippets line {}: duplicate trigger {:?}, already defined on line {} — \
                     only the first one is used",
                    s.line,
                    s.trigger,
                    snippets[*o.get()].line
                ),
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
            problems.push(Problem::fault(
                s.line,
                format!(
                    "snippets line {}: the body of {:?} contains the trigger {found:?} as a \
                     whole sentence — snippets do not nest, so this entry is disabled",
                    s.line, s.trigger
                ),
            ));
            nested.push(key);
        }
    }
    for key in nested {
        by_trigger.remove(&key);
    }

    // Third pass, advisory only: a multi-line body is the shape of the feature
    // users want most (an email signature) and also the shape that #73 breaks.
    // Until the injector has a clipboard backend (#68) a newline is typed as a
    // Return key, which in Slack, Discord and most chat boxes *sends* the
    // message. The snippet is not malformed and stays enabled — it is correct
    // in any editor — but a user should learn this from Settings, not from a
    // customer thread.
    for (idx, s) in snippets.iter().enumerate() {
        let key = normalize_key(&s.trigger);
        if by_trigger.get(&key) != Some(&idx) {
            continue;
        }
        let lines = s.body.lines().count();
        if lines > 1 {
            problems.push(Problem::advisory(
                s.line,
                format!(
                    "snippets line {}: {:?} has a {lines}-line body — until #73 lands, a \
                     newline is typed as Return, which sends the message in Slack, Discord \
                     and similar apps",
                    s.line, s.trigger
                ),
            ));
        }
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

/// Ends a sentence.
///
/// `,` `;` and `:` are deliberately *not* here. They punctuate a clause, not a
/// command, and treating them as boundaries fires on ordinary written English:
/// "Sign off, and let me know.", "Next: sign off.", "Owner: sign off;
/// Reviewer: Ada." — three sentences a snippet would have mangled to buy
/// "Address: my address". See the module docs; `a_clause_separator_does_not_
/// start_a_new_sentence` is the test.
fn is_delimiter(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '\n' | '。' | '！' | '？' | '…')
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

/// What [`Snippets::new`] found when the config named no path of its own.
// Which variants can occur depends on `cfg(test)`: a test build only ever
// produces `Skipped`, a real build never does.
#[allow(dead_code)]
enum DefaultPath {
    Found(PathBuf),
    /// No home directory to look in — worth saying out loud rather than
    /// loading nothing in silence, because `apps/cli` may well have found
    /// `config.toml` anyway. See [`home_dir`].
    NoHome,
    /// This crate's own unit tests, which never read a contributor's real
    /// file. Not a problem, and not reported as one.
    Skipped,
}

/// [`default_path`], except inside this crate's own unit tests.
///
/// `wc-text` has no business reading a contributor's real snippets file while
/// `cargo test` runs — `PolishConfig::validate()` builds all six transforms,
/// so without this a developer with one malformed entry at home would see
/// tests fail in `lib.rs`. Every test that exercises loading passes an
/// explicit path.
#[cfg(not(test))]
fn default_path_for_load() -> DefaultPath {
    match default_path() {
        Some(p) => DefaultPath::Found(p),
        None => DefaultPath::NoHome,
    }
}

#[cfg(test)]
fn default_path_for_load() -> DefaultPath {
    DefaultPath::Skipped
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

/// `$HOME`, when it is set to something non-empty.
///
/// **Known divergence from `dirs`, documented rather than fixed.** When `$HOME`
/// is unset or empty, `dirs_sys` falls back to `getpwuid_r`; this returns
/// `None`, so `apps/cli` can load `config.toml` from the passwd-database home
/// while snippets load from nowhere. Closing the gap means libc — a new
/// dependency in a crate whose whole point is that it has none — for a case
/// both systemd and launchd rule out (`su` without `-`, cron, a bare service
/// launch). Instead [`Snippets::new`] reports it: the user sees "no usable
/// $HOME" in Settings rather than an empty list and no explanation. If a
/// second caller ever needs this resolver, move it and the `dirs` dependency
/// somewhere both can share (#43 hand-rolled the same one).
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
        assert!(faults(&s).is_empty(), "{:?}", s.validate());
        // the signature block is multi-line, so exactly one #73 advisory
        assert_eq!(advisories(&s).len(), 1, "{:?}", s.validate());
        s
    }

    fn one(trigger: &str, body: &str) -> Snippets {
        Snippets::from_source(&format!("[{trigger}]\n{body}\n"))
    }

    /// A well-formed file: one entry per pair, blank line between them, which
    /// is what the format documents. Written as a helper so a test that is
    /// about matching does not accidentally also test the compact-file
    /// warning — `a_header_directly_under_body_text_is_reported_but_nothing_is_disabled`
    /// owns that.
    fn file_of(entries: &[(&str, &str)]) -> String {
        entries
            .iter()
            .map(|(t, b)| format!("[{t}]\n{b}\n\n"))
            .collect()
    }

    fn snips(entries: &[(&str, &str)]) -> Snippets {
        Snippets::from_source(&file_of(entries))
    }

    /// Problems that disable an entry. Most tests care about these and not
    /// about the advisories, which a perfectly good signature snippet earns
    /// just for having two lines — `validate_warns_about_a_multi_line_body`
    /// and `a_header_directly_under_body_text_is_reported_but_nothing_is_disabled`
    /// own those.
    fn faults(s: &Snippets) -> Vec<String> {
        problems_of(s, Severity::Fault)
    }

    fn advisories(s: &Snippets) -> Vec<String> {
        problems_of(s, Severity::Advisory)
    }

    fn problems_of(s: &Snippets, severity: Severity) -> Vec<String> {
        s.problems
            .iter()
            .filter(|p| p.severity == severity)
            .map(|p| p.message.clone())
            .collect()
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
        assert!(faults(&s).is_empty(), "{:?}", s.validate());
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

    /// Notepad and VS Code both offer "UTF-8 with BOM", and one invisible
    /// character at the front of the file used to swallow the first snippet
    /// whole — with a misleading "text before the first [trigger] line"
    /// message pointing at a line that looks perfectly fine.
    #[test]
    fn a_utf8_bom_does_not_swallow_the_first_snippet() {
        let s = Snippets::from_source("\u{feff}[insert my email]\nada@example.com\n");
        assert!(s.validate().is_empty(), "{:?}", s.validate());
        assert_eq!(s.snippets().len(), 1);
        assert_eq!(s.apply("Insert my email."), "ada@example.com");

        // and through a real file, bytes EF BB BF and all
        let f = TempFile::new_bytes("bom", "\u{feff}[sign off]\nBest,\nAda\n".as_bytes());
        assert_eq!(
            Snippets::from_file(f.path()).apply("Sign off."),
            "Best,\nAda"
        );
    }

    /// A backslash escapes `[`, `#` or another backslash, so a body line that
    /// really starts with `\[` can be written at all.
    #[test]
    fn a_leading_backslash_can_itself_be_escaped() {
        let s = Snippets::from_source("[a]\n\\\\[link]\n\\[link]\n\\# hash\n\\path\\to\n");
        assert!(faults(&s).is_empty(), "{:?}", s.validate());
        assert_eq!(s.snippets()[0].body, "\\[link]\n[link]\n# hash\n\\path\\to");
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
            ("1. Sign off.", "1. Best,\nAda\nFounder, WhisprCatch"),
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
            // clause separators: a colon or a semicolon does not start a new
            // sentence, so none of these are commands
            "Next: sign off.",
            "Blockers: sign off: none.",
            "Owner: sign off; Reviewer: Ada.",
            "Two things; standup link; done.",
            "Ping me: insert my email.",
            "Todo: insert my email",
            // list markers other than a number are not sentence boundaries
            "- sign off.",
            "* Sign off.",
            "• sign off",
        ];
        for input in quiet {
            assert_eq!(s.apply(input), input, "expected {input:?} to stay put");
        }
    }

    /// `,` `;` and `:` punctuate a clause, not a command. Splitting on them
    /// would buy "Address: my address" at the price of mangling ordinary
    /// written English, and this is the corpus that made that call. An earlier
    /// revision did split on `;` and `:`, and every line below was broken by
    /// it.
    #[test]
    fn a_clause_separator_does_not_start_a_new_sentence() {
        let s = real();
        for input in [
            "Sign off, and let me know.",
            "Yes, sign off, please.",
            "Next: sign off.",
            "Blockers: sign off: none.",
            "Owner: sign off; Reviewer: Ada.",
            "Agenda: standup link; notes; actions.",
        ] {
            assert_eq!(s.apply(input), input, "{input:?} was mangled");
        }
        // and the cost of that choice, stated as a test rather than a hope:
        // a label-then-value dictation does not expand unless the trigger is
        // a sentence or a line of its own
        assert_eq!(s.apply("Email: insert my email"), "Email: insert my email");
        assert_eq!(s.apply("Email\ninsert my email"), "Email\nada@example.com");
    }

    /// A numbered list marker *is* a sentence boundary ("1." ends a sentence
    /// as far as any punctuation-based split can tell), a dash or a bullet is
    /// not. Asymmetric, accepted, and pinned here so it is a known shape
    /// rather than a surprise in a bug report.
    #[test]
    fn a_numbered_list_marker_ends_a_sentence_but_a_bullet_does_not() {
        let s = real();
        assert_eq!(
            s.apply("1. Sign off."),
            "1. Best,\nAda\nFounder, WhisprCatch"
        );
        assert_eq!(s.apply("- sign off."), "- sign off.");
        assert_eq!(s.apply("* Sign off."), "* Sign off.");
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
        // a semicolon is not a sentence boundary at all, so this is one
        // segment and nothing fires
        assert_eq!(s.apply("Insert my email;"), "Insert my email;");
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
        let s = snips(&[
            ("sign", "SHORT"),
            ("sign off", "LONG"),
            ("sign off now", "LONGEST"),
            ("my", "M"),
        ]);
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
        let s = snips(&[
            ("hello world", "HELLO"),
            ("日本語のテキストです", "JA"),
            ("Правда — это не то, что кажется", "RU"),
            ("um, I mean, like, the thing", "THING"),
        ]);
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
        let s = snips(&[("loop", "before\nloop\nafter")]);
        assert_eq!(faults(&s).len(), 1, "{:?}", s.validate());
        assert!(faults(&s)[0].contains("do not nest"), "{:?}", s.validate());
        assert_eq!(s.apply("Loop."), "Loop.");

        // a mutual cycle: both are disabled, neither expands
        let s = snips(&[("a", "b"), ("b", "a")]);
        assert_eq!(faults(&s).len(), 2, "{:?}", s.validate());
        assert_eq!(s.apply("A. B."), "A. B.");

        // a one-way reference: only the referring entry is disabled
        let s = snips(&[
            ("sign off", "Best,\ninsert my email"),
            ("insert my email", "ada@example.com"),
        ]);
        assert_eq!(faults(&s).len(), 1, "{:?}", s.validate());
        assert_eq!(s.apply("Sign off."), "Sign off.");
        assert_eq!(s.apply("Insert my email."), "ada@example.com");
    }

    /// A body that merely *mentions* a trigger inside a longer sentence is
    /// fine: it can never match, so it can never re-expand.
    #[test]
    fn a_body_that_mentions_a_trigger_mid_sentence_is_not_nesting() {
        let s = snips(&[("a", "please sign off on this"), ("sign off", "Best,\nAda")]);
        assert!(faults(&s).is_empty(), "{:?}", s.validate());
        assert_eq!(s.apply("A."), "please sign off on this");
        assert_eq!(s.apply(&s.apply("A.")), s.apply("A."));
    }

    /// Expansion must not create a trigger out of the text around it either.
    #[test]
    fn expansion_does_not_create_a_new_trigger_at_the_seam() {
        // "Ha!" introduces a sentence boundary the input did not have
        let s = snips(&[("a", "Ha!"), ("ha", "NO")]);
        // "Ha!" contains "Ha" as a whole sentence, so [a] is disabled
        assert_eq!(faults(&s).len(), 1, "{:?}", s.validate());
        assert_eq!(s.apply("A. b."), "A. b.");

        // with the collision removed, the seam is stable
        let s = one("a", "Ha!");
        assert!(s.validate().is_empty(), "{:?}", s.validate());
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

    /// An earlier revision let an empty body through and expanded the trigger
    /// to nothing, so saying it *deleted* the sentence. That is how an
    /// unescaped `[Auto-reply]` line turns into a snippet that eats the user's
    /// words, and deleting dictation is worse than not expanding it.
    #[test]
    fn an_empty_body_is_reported_and_never_fires() {
        let s = snips(&[("scratch that", ""), ("keep", "kept")]);
        assert_eq!(faults(&s).len(), 1, "{:?}", s.validate());
        assert!(
            faults(&s)[0].contains("no text under it"),
            "{:?}",
            s.validate()
        );
        assert_eq!(s.snippets()[0].body, "");
        assert_eq!(s.apply("Scratch that."), "Scratch that.");
        assert_eq!(s.apply("Yes. Scratch that. No."), "Yes. Scratch that. No.");
        // the sound entry alongside it still works
        assert_eq!(s.apply("Keep."), "kept");
    }

    /// The file from the review: bracketed placeholders are how people write
    /// canned replies, and reading them as headers silently truncated one
    /// snippet, invented two more, and left both of the invented ones able to
    /// *delete* a sentence. Every part of that is now reported, and neither
    /// phantom fires.
    #[test]
    fn bracketed_placeholders_in_a_body_are_reported_not_silently_obeyed() {
        let s = Snippets::from_source(
            "[out of office]\n\
             [Auto-reply]\n\
             I am away until Monday and will reply then.\n\
             Ada\n\
             \n\
             [sign off]\n\
             Best,\n\
             Ada\n\
             [Founder, WhisprCatch]\n",
        );
        let problems = s.validate();
        assert!(!problems.is_empty(), "the review's file reported nothing");

        // the two entries that would have deleted a dictated sentence are dead
        assert_eq!(s.apply("Out of office."), "Out of office.");
        assert_eq!(s.apply("Founder, WhisprCatch."), "Founder, WhisprCatch.");
        assert!(faults(&s).iter().any(|m| m.contains("no text under it")));

        // and the ambiguous line is called out by line number, with the fix
        let ambiguous: Vec<String> = advisories(&s)
            .into_iter()
            .filter(|m| m.contains("directly under body text"))
            .collect();
        assert_eq!(ambiguous.len(), 1, "{problems:?}");
        assert!(ambiguous[0].contains("line 9"), "{problems:?}");
        assert!(ambiguous[0].contains("blank line"), "{problems:?}");
    }

    /// The other half: escaping the bracket keeps the line as body text, which
    /// is what the user meant in the first place.
    #[test]
    fn an_escaped_bracket_line_stays_in_the_body() {
        let s = Snippets::from_source("[sign off]\nBest,\nAda\n\\[Founder, WhisprCatch]\n");
        assert!(faults(&s).is_empty(), "{:?}", s.validate());
        assert_eq!(s.snippets().len(), 1);
        assert_eq!(s.apply("Sign off."), "Best,\nAda\n[Founder, WhisprCatch]");
    }

    /// A compactly written file — no blank lines between entries — is a false
    /// positive for the check above, so it is warned about and left working.
    /// Disabling those entries would be far worse than the warning.
    #[test]
    fn a_header_directly_under_body_text_is_reported_but_nothing_is_disabled() {
        let s = Snippets::from_source("[a]\nbody a\n[b]\nbody b\n[c]\nbody c\n");
        assert!(faults(&s).is_empty(), "{:?}", s.validate());
        assert_eq!(advisories(&s).len(), 2, "{:?}", s.validate());
        assert_eq!(s.apply("A."), "body a");
        assert_eq!(s.apply("B."), "body b");
        assert_eq!(s.apply("C."), "body c");
    }

    /// #73: the signature block is the flagship use and the one shape the
    /// injector cannot type safely yet. Warned about, never disabled.
    #[test]
    fn validate_warns_about_a_multi_line_body() {
        let s = one("sign off", "Best,\nAda\nFounder, WhisprCatch");
        let warned = advisories(&s);
        assert_eq!(warned.len(), 1, "{:?}", s.validate());
        assert!(warned[0].contains("3-line body"), "{warned:?}");
        assert!(warned[0].contains("#73"), "{warned:?}");
        assert!(warned[0].contains("Slack"), "{warned:?}");
        assert!(faults(&s).is_empty(), "{:?}", s.validate());
        // and it still expands, because it is correct in any editor
        assert_eq!(s.apply("Sign off."), "Best,\nAda\nFounder, WhisprCatch");
        // the flattened list marks it, so a reader can tell "switched off"
        // from "works, but read this"
        assert!(s.validate()[0].starts_with("note: "), "{:?}", s.validate());

        // a one-line body is silent
        assert!(one("insert my email", "ada@example.com")
            .validate()
            .is_empty());
        // a disabled entry does not also earn an advisory it cannot act on
        let s = snips(&[("loop", "before\nloop\nafter")]);
        assert!(advisories(&s).is_empty(), "{:?}", s.validate());
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

    /// "Case is folded" would oversell it. Matching lowercases with Rust's
    /// `str::to_lowercase` and does nothing else — no case *folding*, no
    /// NFC/NFD, no width normalization, no locale rules. These are the shapes
    /// that stay quiet as a result, pinned so the claim in the module docs
    /// cannot drift away from the code.
    #[test]
    fn case_folding_is_simple_lowercasing_only() {
        let s = one("sign off", "Best,\nAda");
        assert_eq!(s.apply("Sign off."), "Best,\nAda");
        // Turkish dotted capital I lowercases to "i" + combining dot above
        assert_eq!(s.apply("SİGN OFF."), "SİGN OFF.");
        // fullwidth Latin is a different set of scalars, and U+FF0E is not a
        // sentence boundary either
        assert_eq!(s.apply("ｓｉｇｎ　ｏｆｆ．"), "ｓｉｇｎ　ｏｆｆ．");
        // Greek final sigma is the one context-sensitive case `to_lowercase`
        // does handle, and it handles it on both sides
        let s = one("ΟΔΟΣ", "STREET");
        assert_eq!(s.apply("οδός."), "οδός.");
        assert_eq!(s.apply("ΟΔΟΣ."), "STREET");
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
        let s = snips(&[
            ("", "body"),
            ("   ", "body"),
            ("\t", "body"),
            ("ok", "fine"),
        ]);
        let problems = faults(&s);
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("line 1"), "{problems:?}");
        assert!(problems[0].contains("empty trigger"), "{problems:?}");
        assert!(problems[1].contains("only whitespace"), "{problems:?}");
        assert!(problems[2].contains("line 7"), "{problems:?}");
        // the sound entry still works
        assert_eq!(s.apply("Ok."), "fine");
    }

    #[test]
    fn validate_reports_duplicate_triggers_and_keeps_the_first() {
        let s = snips(&[("sign off", "FIRST"), ("Sign  Off", "SECOND")]);
        let problems = faults(&s);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("duplicate"), "{problems:?}");
        assert!(problems[0].contains("line 4"), "{problems:?}");
        assert!(problems[0].contains("line 1"), "{problems:?}");
        assert_eq!(s.apply("Sign off."), "FIRST");
    }

    /// A trigger written with a full stop in it can never match a whole
    /// sentence, because the full stop is where sentences are cut. Silently
    /// never firing is the worst possible outcome, so it is an error.
    #[test]
    fn validate_reports_a_trigger_that_could_never_match() {
        for bad in ["sign off.", "e.g", "done!", "a?b", "x…y"] {
            let s = one(bad, "body");
            let problems = s.validate();
            assert_eq!(problems.len(), 1, "{bad:?} -> {problems:?}");
            assert!(problems[0].contains("never match"), "{problems:?}");
            assert!(s.is_empty());
        }
    }

    /// Three passes produce problems — parse, reject, advise — and #49 renders
    /// the list next to the file, so they come out in the order the user
    /// reads, not the order the code happened to find them.
    #[test]
    fn problems_come_out_in_file_order() {
        let s = Snippets::from_source(
            "[nest]\n\
             dup\n\
             \n\
             [dup]\n\
             ONE\n\
             \n\
             [sign off]\n\
             Best,\n\
             Ada\n\
             \n\
             [dup]\n\
             TWO\n",
        );
        let lines: Vec<usize> = s.problems.iter().map(|p| p.line).collect();
        let mut sorted = lines.clone();
        sorted.sort_unstable();
        assert_eq!(lines, sorted, "{:?}", s.validate());
        assert_eq!(lines, [1, 7, 11], "{:?}", s.validate());
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
        // "valid" means nothing disabled; the multi-line signature still
        // earns its #73 advisory
        assert!(faults(&Snippets::from_source(REAL_FILE)).is_empty());
        assert!(Snippets::from_source("").validate().is_empty());
        assert!(one("insert my email", "ada@example.com")
            .validate()
            .is_empty());
    }

    /// Every problem `validate` reports must also mean the entry cannot fire.
    /// Reporting a malformed snippet and expanding it anyway would be the
    /// worst of both worlds.
    #[test]
    fn a_reported_entry_never_expands() {
        let s = snips(&[
            ("", "X"),
            ("  ", "X"),
            ("dup", "ONE"),
            ("dup", "TWO"),
            ("bad.", "X"),
            ("empty", ""),
            ("nest", "dup"),
        ]);
        assert_eq!(faults(&s).len(), 6, "{:?}", s.validate());
        assert_eq!(s.apply("Dup."), "ONE"); // the first of the duplicates
        for input in ["X.", "Bad.", "Nest.", "Empty.", "."] {
            assert_eq!(s.apply(input), input, "{input:?} expanded");
        }
        // and the converse, which is the half that protects the user: no
        // entry fires unless the file said so cleanly
        assert_eq!(s.snippets().len(), 7);
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
        assert!(faults(&s).is_empty(), "{:?}", s.validate());
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
        // one advisory (#73, the multi-line signature) and nothing disabled
        assert_eq!(cfg.validate().len(), 1, "{:?}", cfg.validate());

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

        let real_file = || Snippets::from_source(REAL_FILE);

        // desirable: the dictionary repairs a trigger the model misheard, and
        // the snippet then fires on text the user never actually said
        let chain = Polish::from_transforms(vec![
            Box::new(WordRule("of", "off")) as BoxedTransform,
            Box::new(real_file()),
        ]);
        assert_eq!(chain.apply("Sign of."), "Best,\nAda\nFounder, WhisprCatch");

        // undesirable, and the reason this is written down: a rule the user
        // added for prose ("email" -> "e-mail") silently stops a trigger that
        // contains the same word from ever matching, with no error anywhere
        let chain = Polish::from_transforms(vec![
            Box::new(WordRule("email", "e-mail")) as BoxedTransform,
            Box::new(real_file()),
        ]);
        assert_eq!(chain.apply("Insert my email."), "Insert my e-mail.");
        // case is not the failure mode: matching folds case, so a rule that
        // only changes capitalisation leaves the trigger working
        let chain = Polish::from_transforms(vec![
            Box::new(WordRule("email", "EMAIL")) as BoxedTransform,
            Box::new(real_file()),
        ]);
        assert_eq!(chain.apply("Insert my email."), "ada@example.com");

        // the reverse order would be worse: the dictionary would rewrite the
        // user's own saved snippet text, which they already typed the way they
        // want it
        let reversed = Polish::from_transforms(vec![
            Box::new(real_file()) as BoxedTransform,
            Box::new(WordRule("ada", "Ada Lovelace")),
        ]);
        assert_eq!(
            reversed.apply("Sign off."),
            "Best,\nAda Lovelace\nFounder, WhisprCatch"
        );
    }

    /// The fourth composition case, and the dangerous one. The nesting guard
    /// reads bodies as written, at load time; it cannot see a body that only
    /// *becomes* a trigger once the dictionary has rewritten it. Both files
    /// validate clean, and the chain grows on every pass.
    ///
    /// This asserts the broken behaviour on purpose. Production applies the
    /// chain once (`finish()` in `apps/cli`), so nothing is broken today — but
    /// `lib.rs::applying_the_chain_twice_changes_nothing` asserts this
    /// invariant for the whole chain, and #49's "preview, then dictate" is a
    /// double-apply. Whoever builds that surface needs the cross-file check;
    /// if this test starts failing because someone added it, delete the test
    /// and celebrate.
    #[test]
    fn composition_with_a_dictionary_can_break_idempotence() {
        use crate::{BoxedTransform, Polish};

        struct Rewrite(&'static str, &'static str);
        impl Transform for Rewrite {
            fn name(&self) -> &'static str {
                "dictionary"
            }
            fn apply(&self, text: &str) -> String {
                text.replace(self.0, self.1)
            }
            fn prefix_stable(&self) -> bool {
                false
            }
        }

        let snippet = || one("sign off", "Best,\nregards\nAda");
        // in isolation: nothing to report, and idempotent
        assert!(faults(&snippet()).is_empty());
        let once = snippet().apply("Sign off.");
        assert_eq!(snippet().apply(&once), once);

        // composed with a dictionary rule that turns the body into a trigger
        let chain = || {
            Polish::from_transforms(vec![
                Box::new(Rewrite("regards", "sign off")) as BoxedTransform,
                Box::new(snippet()),
            ])
        };
        let once = chain().apply("Sign off.");
        assert_eq!(once, "Best,\nregards\nAda");
        assert_eq!(
            chain().apply(&once),
            "Best,\nBest,\nregards\nAda\nAda",
            "if this now equals `once`, the cross-file check landed"
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
                file.push_str(&format!("[trigger number {i}]\nbody number {i}\n\n"));
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
    ///
    /// Both paths, deliberately. With short triggers every 44-character
    /// sentence is thrown out by `longer_than` without allocating, and
    /// `normalize_key` never runs at all — so a version of this test with only
    /// short triggers measures the guard and nothing else. The second half
    /// adds a trigger longer than any sentence in the corpus, which forces the
    /// slow path 45,000 times.
    #[test]
    fn a_two_megabyte_utterance_stays_linear_on_both_paths() {
        let big = "the quick brown fox jumps over the lazy dog. ".repeat(45_000);

        for (name, s) in [
            ("fast path (every sentence rejected on length)", real()),
            (
                "slow path (every sentence normalized and hashed)",
                one(
                    "a trigger deliberately longer than any sentence in the corpus above",
                    "NEVER",
                ),
            ),
        ] {
            let start = Instant::now();
            let out = s.apply(&big);
            let elapsed = start.elapsed();
            assert_eq!(out, big);
            println!("snippets: {elapsed:?} for 2 MB, {name}");
            assert!(elapsed < Duration::from_secs(20), "{name}: {elapsed:?}");
        }
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
