//! Transcription history: append-only JSONL at
//! <data-dir>/whisper-catch/history.jsonl. Local only, user-clearable.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Unix timestamp (seconds)
    pub ts: u64,
    /// Utterance length in seconds
    pub dur_s: f32,
    /// Inference time in seconds
    pub infer_s: f32,
    /// What the user gets: the transcript after the polish chain (#40).
    pub text: String,
    /// What the model said, kept only when polish changed it. Undo (#42) and
    /// the Settings preview (#49) read this.
    ///
    /// `Option` + `#[serde(default)]` is load-bearing, not tidiness: every
    /// `history.jsonl` written before v0.5 has no `raw` key, and without the
    /// default those lines stop deserializing and a user's whole history
    /// silently disappears from the window. `skip_serializing_if` keeps new
    /// lines byte-identical to the old format when nothing was polished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

pub fn history_path() -> PathBuf {
    dirs::data_dir()
        .expect("no data dir on this platform")
        .join("whisper-catch")
        .join("history.jsonl")
}

pub fn append(entry: &Entry) -> Result<()> {
    let path = history_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    writeln!(f, "{}", serde_json::to_string(entry)?)?;
    Ok(())
}

/// Returns up to `limit` most-recent entries, newest first.
/// Malformed lines are skipped rather than failing the whole load.
pub fn load(limit: usize) -> Result<Vec<Entry>> {
    let path = history_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    Ok(parse(&std::fs::read_to_string(&path)?, limit))
}

/// The file-format half of [`load`], split out so it can be tested without
/// touching the user's real history.
fn parse(file: &str, limit: usize) -> Vec<Entry> {
    let mut entries: Vec<Entry> = file
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    entries.reverse();
    entries.truncate(limit);
    entries
}

/// Removes a single entry, matched by timestamp. Rewrites the file; a
/// history this size (append-only text) makes that cheap.
pub fn delete(ts: u64) -> Result<()> {
    let path = history_path();
    if !path.exists() {
        return Ok(());
    }
    let file = std::fs::read_to_string(&path)?;
    std::fs::write(&path, without_ts(&file, ts))
        .with_context(|| format!("rewriting {}", path.display()))
}

/// The file-format half of [`delete`]. Unparseable lines are kept rather than
/// dropped: deleting one entry should never quietly discard the rest.
fn without_ts(file: &str, ts: u64) -> String {
    let kept: Vec<&str> = file
        .lines()
        .filter(|l| {
            serde_json::from_str::<Entry>(l)
                .map(|e| e.ts != ts)
                .unwrap_or(true)
        })
        .collect();
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

pub fn clear() -> Result<()> {
    match std::fs::remove_file(history_path()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// All-time totals: (utterances, words, audio seconds).
pub fn totals() -> (u64, u64, f32) {
    load(usize::MAX)
        .unwrap_or_default()
        .iter()
        .fold((0, 0, 0.0), |(n, w, s), e| {
            (n + 1, w + e.text.split_whitespace().count() as u64, s + e.dur_s)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim shape of a line written by v0.4.0 and earlier: no `raw` key.
    const V0_4_LINE: &str = r#"{"ts":1754000000,"dur_s":2.5,"infer_s":0.31,"text":"hello world"}"#;

    fn entry(ts: u64, text: &str, raw: Option<&str>) -> Entry {
        Entry {
            ts,
            dur_s: 1.0,
            infer_s: 0.1,
            text: text.into(),
            raw: raw.map(str::to_string),
        }
    }

    // ---- backward compatibility ------------------------------------------

    /// The regression that would eat an existing user's whole history: `raw`
    /// is new, every line already on disk lacks it.
    #[test]
    fn a_pre_polish_line_still_deserializes() {
        let e: Entry = serde_json::from_str(V0_4_LINE).unwrap();
        assert_eq!(e.ts, 1_754_000_000);
        assert_eq!(e.text, "hello world");
        assert_eq!(e.raw, None);
    }

    #[test]
    fn old_and_new_lines_coexist_in_one_file() {
        let file = format!(
            "{V0_4_LINE}\n{}\n",
            serde_json::to_string(&entry(1_754_000_001, "polished", Some("raw"))).unwrap()
        );
        let entries = parse(&file, 10);
        assert_eq!(entries.len(), 2);
        // newest first
        assert_eq!(entries[0].raw.as_deref(), Some("raw"));
        assert_eq!(entries[1].raw, None);
    }

    /// Nothing polished means nothing extra on disk — a v0.5 history file with
    /// the chain off is byte-for-byte what v0.4.0 wrote.
    #[test]
    fn unpolished_entries_serialize_without_a_raw_key() {
        let line = serde_json::to_string(&entry(1_754_000_000, "hello world", None)).unwrap();
        assert!(!line.contains("raw"), "{line}");
        // and a v0.4.0 line survives a full read-write cycle unchanged
        let old: Entry = serde_json::from_str(V0_4_LINE).unwrap();
        assert_eq!(serde_json::to_string(&old).unwrap(), V0_4_LINE);
    }

    #[test]
    fn polished_entries_round_trip_through_json() {
        let e = entry(
            42,
            "I said Wednesday",
            Some("I said Tuesday, I mean Wednesday"),
        );
        let back: Entry = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back.text, e.text);
        assert_eq!(back.raw, e.raw);
        assert_eq!(back.ts, e.ts);
    }

    #[test]
    fn an_explicit_null_raw_reads_as_none() {
        let e: Entry =
            serde_json::from_str(r#"{"ts":1,"dur_s":1.0,"infer_s":0.1,"text":"hi","raw":null}"#)
                .unwrap();
        assert_eq!(e.raw, None);
    }

    // ---- parse -----------------------------------------------------------

    #[test]
    fn parse_skips_malformed_lines_without_losing_the_rest() {
        let file = format!("not json\n{V0_4_LINE}\n{{\"half\": \n");
        let entries = parse(&file, 10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "hello world");
    }

    #[test]
    fn parse_returns_newest_first_and_honours_the_limit() {
        let file: String = (0..5)
            .map(|i| serde_json::to_string(&entry(i, &format!("line {i}"), None)).unwrap() + "\n")
            .collect();
        let entries = parse(&file, 2);
        assert_eq!(
            entries.iter().map(|e| e.ts).collect::<Vec<_>>(),
            [4, 3],
            "history should read newest first"
        );
    }

    #[test]
    fn parse_handles_an_empty_file() {
        assert!(parse("", 10).is_empty());
    }

    // ---- delete ----------------------------------------------------------

    #[test]
    fn without_ts_drops_only_the_matching_entry() {
        let file: String = (0..3)
            .map(|i| serde_json::to_string(&entry(i, &format!("line {i}"), None)).unwrap() + "\n")
            .collect();
        let out = without_ts(&file, 1);
        assert_eq!(
            parse(&out, 10).iter().map(|e| e.ts).collect::<Vec<_>>(),
            [2, 0]
        );
        assert!(out.ends_with('\n'));
    }

    /// Deleting one entry must not quietly discard a line we failed to parse.
    #[test]
    fn without_ts_keeps_unparseable_lines() {
        let file = format!("garbage\n{V0_4_LINE}\n");
        assert_eq!(without_ts(&file, 1_754_000_000), "garbage\n");
    }

    #[test]
    fn without_ts_on_the_last_entry_leaves_an_empty_file() {
        assert_eq!(without_ts(&format!("{V0_4_LINE}\n"), 1_754_000_000), "");
    }
}
