//! Unicode measuring for the injector. Pure: no platform code, no display
//! server, no I/O — every function here is exercised by the tests at the
//! bottom of this file, on both CI runners.
//!
//! Two units show up in this crate and they are not interchangeable:
//!
//! * **`char`** (a Unicode scalar value) is the unit at the API boundary.
//!   `replace_last(n_chars, ..)` takes chars because that is what a Rust
//!   caller can compute for free with `text.chars().count()` and cannot get
//!   wrong.
//! * **grapheme cluster** is the unit at the keyboard. One Backspace deletes
//!   one extended grapheme cluster in AppKit, GTK, Qt, Blink and Gecko, so a
//!   backspace *count* must be a cluster count. `"👨‍👩‍👧‍👦"` is seven chars and one
//!   press; counting chars there would eat six characters of text the user
//!   typed themselves.
//!
//! Conversion between the two needs the actual text, which is why the injector
//! keeps a record of what it typed (see [`crate::plan::Typed`]).

use unicode_segmentation::UnicodeSegmentation;

/// Number of extended grapheme clusters — i.e. the number of Backspace
/// presses it takes to remove `s` from a text field.
pub fn cluster_count(s: &str) -> usize {
    s.graphemes(true).count()
}

/// The grapheme-cluster boundary at or before `byte`.
///
/// Used to widen a deletion outwards: if a caller asks to take back a count of
/// chars that lands inside a cluster (the combining acute of `é`, one half of
/// a flag), the keyboard cannot honour that — Backspace removes the whole
/// cluster. Snapping back makes the request expressible; the characters swept
/// in are retyped, see [`crate::plan::Typed::plan_replace`].
pub fn snap_to_cluster(s: &str, byte: usize) -> usize {
    if byte >= s.len() {
        return s.len();
    }
    let mut last = 0;
    for (i, _) in s.grapheme_indices(true) {
        if i > byte {
            break;
        }
        last = i;
    }
    last
}

/// Byte length of the longest prefix `a` and `b` share, measured in whole
/// grapheme clusters.
///
/// Cluster-wise rather than char-wise on purpose: `"cafe"` and `"cafe\u{301}"`
/// share three clusters (`caf`), not four chars, because the fourth cluster is
/// `e` on one side and `é` on the other. Trusting the char answer would leave
/// one Backspace to delete a combining mark, and a real text field would take
/// the whole `é` with it.
pub fn common_cluster_prefix(a: &str, b: &str) -> usize {
    let mut n = 0;
    for (ga, gb) in a.graphemes(true).zip(b.graphemes(true)) {
        if ga != gb {
            break;
        }
        n += ga.len();
    }
    n
}

/// Byte index `n_chars` chars from the end of `s`, clamped to the start.
pub fn char_index_from_end(s: &str, n_chars: usize) -> usize {
    if n_chars == 0 {
        return s.len();
    }
    s.char_indices()
        .rev()
        .nth(n_chars - 1)
        .map_or(0, |(i, _)| i)
}

/// Drop leading text so `s` holds at most `max_chars`, cutting on a cluster
/// boundary so the record never begins inside a cluster.
pub fn trim_to_last_chars(s: &mut String, max_chars: usize) {
    if s.chars().count() <= max_chars {
        return;
    }
    let want = char_index_from_end(s, max_chars);
    // Forwards, not back: keeping *fewer* chars than asked is safe, keeping a
    // half cluster is not.
    let cut = s
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .find(|&i| i >= want)
        .unwrap_or(s.len());
    s.drain(..cut);
}

/// Split `s` into runs of at most `max_units` UTF-16 code units, preferring
/// cluster boundaries.
///
/// `CGEventKeyboardSetUnicodeString` is documented to take up to 20 UTF-16
/// units and gets unreliable past that. The limit is in *UTF-16 units*, not
/// chars: an emoji is one char and two units, so a chunker that counts chars
/// silently doubles the payload on exactly the input most likely to break.
///
/// A cluster longer than the limit (a long ZWJ sequence, a base plus a pile of
/// combining marks) is split by chars rather than dropped — never mid-surrogate,
/// since `char` boundaries are always whole code points.
// Only the macOS backend has a per-event limit, but the tests below run on
// every platform: this is the kind of arithmetic that should not first be
// exercised on the machine that ships it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn utf16_chunks(s: &str, max_units: usize) -> Vec<&str> {
    debug_assert!(max_units >= 2, "a single char can need two UTF-16 units");
    let mut out = Vec::new();
    let mut start = 0;
    let mut units = 0;
    for (i, cluster) in s.grapheme_indices(true) {
        let n: usize = cluster.chars().map(char::len_utf16).sum();
        if n > max_units {
            if start < i {
                out.push(&s[start..i]);
            }
            let mut cut = i;
            let mut held = 0;
            for (ci, c) in cluster.char_indices() {
                let cu = c.len_utf16();
                if held + cu > max_units {
                    out.push(&s[cut..i + ci]);
                    cut = i + ci;
                    held = 0;
                }
                held += cu;
            }
            // The tail of the oversized cluster can still take a passenger.
            start = cut;
            units = held;
            continue;
        }
        if units + n > max_units {
            out.push(&s[start..i]);
            start = i;
            units = 0;
        }
        units += n;
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Family: four people joined by zero-width joiners. One cluster, seven
    /// chars, eleven UTF-16 units — the input that breaks every unit mix-up.
    const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    /// `é` as `e` plus a combining acute.
    const E_ACUTE: &str = "e\u{301}";

    #[test]
    fn cluster_count_is_not_char_count() {
        assert_eq!(FAMILY.chars().count(), 7);
        assert_eq!(cluster_count(FAMILY), 1);
        assert_eq!(cluster_count(E_ACUTE), 1);
        assert_eq!(cluster_count("👍🏽"), 1, "skin tone modifier joins its base");
        assert_eq!(
            cluster_count("🇯🇵"),
            1,
            "regional indicator pair is one flag"
        );
        assert_eq!(cluster_count("你好世界"), 4);
        assert_eq!(cluster_count("שלום"), 4, "RTL counts like any other script");
        assert_eq!(cluster_count(""), 0);
    }

    #[test]
    fn snap_to_cluster_widens_to_the_enclosing_cluster() {
        let s = format!("caf{E_ACUTE}");
        assert_eq!(snap_to_cluster(&s, s.len()), s.len(), "end is a boundary");
        // 3 = start of the `e`, i.e. the start of the é cluster.
        assert_eq!(snap_to_cluster(&s, 4), 3, "mid-cluster snaps back");
        assert_eq!(snap_to_cluster(&s, 3), 3, "a boundary stays put");
        assert_eq!(snap_to_cluster(&s, 0), 0);
        assert_eq!(snap_to_cluster(FAMILY, 9), 0, "deep inside the ZWJ run");
    }

    #[test]
    fn common_cluster_prefix_stops_at_a_changed_cluster() {
        assert_eq!(common_cluster_prefix("hello world", "hello there"), 6);
        assert_eq!(common_cluster_prefix("abc", "abc"), 3);
        assert_eq!(common_cluster_prefix("abc", "xyz"), 0);
        assert_eq!(common_cluster_prefix("", "abc"), 0);
        assert_eq!(
            common_cluster_prefix("cafe", &format!("caf{E_ACUTE}")),
            3,
            "`e` and `é` are different clusters, so the shared prefix is `caf`"
        );
        assert_eq!(
            common_cluster_prefix("你好世界", "你好朋友"),
            6,
            "two CJK clusters, three bytes each"
        );
    }

    #[test]
    fn char_index_from_end_clamps() {
        assert_eq!(char_index_from_end("abc", 0), 3);
        assert_eq!(char_index_from_end("abc", 1), 2);
        assert_eq!(char_index_from_end("abc", 3), 0);
        assert_eq!(char_index_from_end("abc", 99), 0, "more than exists");
        assert_eq!(char_index_from_end("你好", 1), 3, "bytes, not chars");
        assert_eq!(char_index_from_end("", 4), 0);
    }

    #[test]
    fn trim_to_last_chars_keeps_the_tail_on_a_cluster_boundary() {
        let mut s = "abcdef".to_string();
        trim_to_last_chars(&mut s, 3);
        assert_eq!(s, "def");

        let mut s = "abc".to_string();
        trim_to_last_chars(&mut s, 10);
        assert_eq!(s, "abc", "shorter than the cap is left alone");

        // Cutting at 1 char would keep a bare combining acute; cut forwards.
        let mut s = format!("ab{E_ACUTE}");
        trim_to_last_chars(&mut s, 1);
        assert_eq!(s, "", "never keep half a cluster");

        let mut s = format!("ab{E_ACUTE}");
        trim_to_last_chars(&mut s, 2);
        assert_eq!(s, E_ACUTE);
    }

    #[test]
    fn utf16_chunks_respect_the_unit_limit() {
        assert_eq!(utf16_chunks("", 20), Vec::<&str>::new());
        assert_eq!(utf16_chunks("abc", 20), vec!["abc"]);
        assert_eq!(utf16_chunks("abcdef", 3), vec!["abc", "def"]);

        // Ten emoji: one char each, two UTF-16 units each. A char-counting
        // chunker would emit one 20-unit event; the limit allows ten per event.
        let emoji = "😀".repeat(10);
        let chunks = utf16_chunks(&emoji, 20);
        assert_eq!(chunks.len(), 1);
        let emoji = "😀".repeat(11);
        let chunks = utf16_chunks(&emoji, 20);
        assert_eq!(chunks.len(), 2);
        for c in &chunks {
            assert!(c.encode_utf16().count() <= 20);
        }
    }

    #[test]
    fn utf16_chunks_split_an_oversized_cluster() {
        // One cluster, 31 UTF-16 units: it cannot fit and must not be dropped.
        let monster = format!("a{}", "\u{301}".repeat(30));
        assert_eq!(cluster_count(&monster), 1);
        let chunks = utf16_chunks(&monster, 20);
        assert!(chunks.len() > 1);
        assert_eq!(chunks.concat(), monster, "nothing lost, nothing reordered");
        for c in &chunks {
            assert!(c.encode_utf16().count() <= 20);
        }
    }

    #[test]
    fn utf16_chunks_never_lose_or_reorder_text() {
        let mixed = format!("hi {FAMILY} 你好 שלום {E_ACUTE} 🇯🇵 done");
        for max in [2, 3, 5, 16, 20, 64] {
            let chunks = utf16_chunks(&mixed, max);
            assert_eq!(chunks.concat(), mixed, "max={max}");
            for c in &chunks {
                assert!(c.encode_utf16().count() <= max, "max={max} chunk={c:?}");
            }
        }
    }
}
