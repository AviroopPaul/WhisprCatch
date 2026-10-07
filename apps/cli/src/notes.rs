//! Notes: one plain-text Markdown file per note, in
//! `<data_dir>/whisper-catch/notes/<unix_millis>.md`.
//!
//! The user owns these files and can open them in any editor. Nothing here
//! touches the network. A note that is empty never has a file: `save` of
//! blank text removes it, and `create` only reserves an id.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Longest title shown, in chars.
const TITLE_MAX: usize = 60;
/// Longest preview kept, in chars.
const PREVIEW_MAX: usize = 160;
/// Search text kept per note, in chars. Bounds memory for a huge note.
const SEARCH_MAX: usize = 20_000;
const EXT: &str = "md";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteMeta {
    /// The file stem: unix milliseconds at creation.
    pub id: String,
    pub title: String,
    pub preview: String,
    /// Unix seconds of the file's last modification.
    pub modified: u64,
    /// Lower-cased text for the search box.
    pub search: String,
}

/// First non-empty line, trimmed and cut to [`TITLE_MAX`] chars; "Untitled"
/// when there is none. Leading `#` marks of a Markdown heading are dropped.
pub fn title_of(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let line = line.trim_start_matches('#').trim();
    if line.is_empty() {
        return "Untitled".into();
    }
    cut(line, TITLE_MAX)
}

/// The text after the title line, on one line.
pub fn preview_of(text: &str) -> String {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    lines.next();
    let rest = lines.collect::<Vec<_>>().join(" ");
    cut(&rest, PREVIEW_MAX)
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.truncate(out.trim_end().len());
    out.push('…');
    out
}

/// Ids are digits only, so an id from the command line can never name a path
/// outside the notes folder.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
}

/// Where notes live, next to `history.jsonl`.
pub fn dir() -> PathBuf {
    wc_core::history::history_path()
        .parent()
        .map(|p| p.join("notes"))
        .unwrap_or_else(|| PathBuf::from("notes"))
}

/// A notes folder. The folder is a field so tests can point it at a temp dir.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Default for Store {
    fn default() -> Self {
        Self { dir: dir() }
    }
}

impl Store {
    #[cfg(test)]
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, id: &str) -> Option<PathBuf> {
        valid_id(id).then(|| self.dir.join(format!("{id}.{EXT}")))
    }

    /// Every note, newest modified first. A missing folder is an empty list;
    /// a file that cannot be read is skipped.
    pub fn list(&self) -> Vec<NoteMeta> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return vec![];
        };
        let mut notes: Vec<NoteMeta> = rd
            .filter_map(|e| {
                let path = e.ok()?.path();
                if path.extension()? != EXT {
                    return None;
                }
                let id = path.file_stem()?.to_str()?.to_string();
                if !valid_id(&id) {
                    return None;
                }
                let text = std::fs::read_to_string(&path).ok()?;
                let modified = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs());
                Some(NoteMeta {
                    title: title_of(&text),
                    preview: preview_of(&text),
                    search: text.chars().take(SEARCH_MAX).collect::<String>().to_lowercase(),
                    id,
                    modified,
                })
            })
            .collect();
        notes.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| b.id.cmp(&a.id)));
        notes
    }

    /// Reserves an id for a new note. No file exists until it has text.
    pub fn create(&self) -> String {
        let mut ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        loop {
            let id = ms.to_string();
            if !self.path(&id).is_some_and(|p| p.exists()) {
                return id;
            }
            ms += 1;
        }
    }

    /// The note's text; empty when there is no such note.
    pub fn load(&self, id: &str) -> String {
        self.path(id)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default()
    }

    /// Writes the note atomically (temp file, then rename). Blank text
    /// removes the file instead, so an emptied note leaves nothing behind.
    pub fn save(&self, id: &str, text: &str) -> std::io::Result<()> {
        let Some(path) = self.path(id) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "bad note id",
            ));
        };
        if text.trim().is_empty() {
            return self.delete(id);
        }
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join(format!("{id}.{EXT}.tmp"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)
    }

    pub fn delete(&self, id: &str) -> std::io::Result<()> {
        let Some(path) = self.path(id) else {
            return Ok(());
        };
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> (PathBuf, Store) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "wc-notes-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        (dir.clone(), Store::at(dir.join("notes")))
    }

    fn touch(path: &Path, secs: u64) {
        let t = UNIX_EPOCH + std::time::Duration::from_secs(secs);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn the_title_is_the_first_non_empty_line() {
        assert_eq!(title_of("\n\n  Buy milk  \nand eggs"), "Buy milk");
        assert_eq!(title_of("# Plan\nbody"), "Plan");
        assert_eq!(title_of(""), "Untitled");
        assert_eq!(title_of("  \n \t\n"), "Untitled");
        assert_eq!(title_of("###"), "Untitled");
    }

    #[test]
    fn a_long_title_is_cut_with_an_ellipsis() {
        let t = title_of(&"a".repeat(200));
        assert_eq!(t.chars().count(), TITLE_MAX + 1);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn the_preview_is_the_text_after_the_title() {
        assert_eq!(preview_of("Title\nfirst line\n\nsecond line"), "first line second line");
        assert_eq!(preview_of("Only a title"), "");
        assert_eq!(preview_of(""), "");
    }

    #[test]
    fn unicode_is_cut_on_char_boundaries() {
        let t = title_of(&"é".repeat(100));
        assert_eq!(t.chars().count(), TITLE_MAX + 1);
        assert_eq!(title_of("日本語のメモ\n二行目"), "日本語のメモ");
        assert_eq!(preview_of("日本語のメモ\n二行目"), "二行目");
    }

    #[test]
    fn only_digit_ids_name_a_file() {
        assert!(valid_id("1735689600000"));
        assert!(!valid_id(""));
        assert!(!valid_id("../x"));
        assert!(!valid_id("12a"));
        let (_d, s) = temp();
        assert!(s.save("../evil", "x").is_err());
        assert_eq!(s.load("../evil"), "");
    }

    #[test]
    fn a_missing_folder_is_an_empty_list() {
        let (_d, s) = temp();
        assert!(s.list().is_empty());
    }

    #[test]
    fn save_and_load_round_trip_with_unicode() {
        let (d, s) = temp();
        let id = s.create();
        let text = "Café ☕\nline two\n日本語 🎙";
        s.save(&id, text).unwrap();
        assert_eq!(s.load(&id), text);
        // the atomic write leaves no temp file behind
        let names: Vec<_> = std::fs::read_dir(s.dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, [format!("{id}.md")]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_new_note_that_is_never_typed_into_leaves_no_file() {
        let (d, s) = temp();
        let id = s.create();
        s.save(&id, "").unwrap();
        s.save(&id, "  \n ").unwrap();
        assert!(s.list().is_empty());
        assert!(!s.dir().exists() || std::fs::read_dir(s.dir()).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn emptying_a_note_removes_its_file() {
        let (d, s) = temp();
        let id = s.create();
        s.save(&id, "something").unwrap();
        assert_eq!(s.list().len(), 1);
        s.save(&id, "").unwrap();
        assert!(s.list().is_empty());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_list_is_newest_modified_first() {
        let (d, s) = temp();
        s.save("100", "old").unwrap();
        s.save("200", "middle").unwrap();
        s.save("300", "new").unwrap();
        touch(&s.dir().join("100.md"), 3000);
        touch(&s.dir().join("200.md"), 1000);
        touch(&s.dir().join("300.md"), 2000);
        let ids: Vec<_> = s.list().into_iter().map(|n| n.id).collect();
        assert_eq!(ids, ["100", "300", "200"]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn listing_carries_title_preview_and_search_text() {
        let (d, s) = temp();
        s.save("1", "Groceries\nMilk and EGGS").unwrap();
        let n = &s.list()[0];
        assert_eq!(n.title, "Groceries");
        assert_eq!(n.preview, "Milk and EGGS");
        assert!(n.search.contains("eggs") && n.search.contains("groceries"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn stray_and_unreadable_files_are_skipped() {
        let (d, s) = temp();
        s.save("1", "real").unwrap();
        std::fs::write(s.dir().join("notes.txt"), "x").unwrap();
        std::fs::write(s.dir().join("abc.md"), "x").unwrap();
        std::fs::write(s.dir().join("2.md"), [0xff, 0xfe, 0xfd]).unwrap(); // not UTF-8
        std::fs::create_dir(s.dir().join("3.md")).unwrap();
        let ids: Vec<_> = s.list().into_iter().map(|n| n.id).collect();
        assert_eq!(ids, ["1"]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn create_never_reuses_an_existing_id() {
        let (d, s) = temp();
        let a = s.create();
        s.save(&a, "x").unwrap();
        let b = s.create();
        assert_ne!(a, b);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn deleting_a_missing_note_is_fine() {
        let (_d, s) = temp();
        s.delete("123").unwrap();
    }
}
