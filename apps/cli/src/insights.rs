//! Numbers behind the Insights page. Pure: history entries in, stats out, no
//! I/O. The date conversion is injected so tests do not depend on the time zone
//! of whoever runs them.

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, Duration, NaiveDate};
use wc_core::history::Entry;

/// What a typist manages, for "minutes saved" and "faster than typing". The
/// commonly quoted average for typing on a keyboard is 40 words per minute.
pub const TYPING_WPM: f32 = 40.0;
/// Weeks shown in the heatmap, this week included.
pub const HEATMAP_WEEKS: usize = 20;
/// Under this much total speech, words per minute is noise, so it is hidden.
const MIN_SPEECH_FOR_WPM: f32 = 30.0;
/// Label for dictations recorded with no app name (older history, other OSes).
pub const OTHER_APP: &str = "Other";

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Stats {
    pub total_words: u64,
    pub dictations: u64,
    pub speech_secs: f32,
    /// `None` until there are about 30 seconds of speech to average over.
    pub wpm: Option<f32>,
    /// Typing time at [`TYPING_WPM`] minus speaking time, never below zero.
    pub minutes_saved: f32,
    /// Dictations the cleanup chain changed (their entry carries a `raw`).
    pub cleaned: u64,
    /// Words the cleanup chain changed, over all dictations.
    pub words_changed: u64,
    /// Words per app, most words first. No-app entries sit under "Other".
    pub apps: Vec<(String, u64)>,
    /// Words per day with any dictation.
    pub daily: BTreeMap<NaiveDate, u64>,
    pub words_this_week: u64,
    pub current_streak: u32,
    pub longest_streak: u32,
}

impl Stats {
    /// `date_of` maps a unix timestamp to the local calendar day.
    pub fn compute(
        entries: &[Entry],
        today: NaiveDate,
        date_of: impl Fn(u64) -> NaiveDate,
    ) -> Self {
        let mut s = Stats::default();
        let mut per_app: HashMap<String, u64> = HashMap::new();
        for e in entries {
            let words = e.text.split_whitespace().count() as u64;
            s.total_words += words;
            s.dictations += 1;
            s.speech_secs += e.dur_s;
            if let Some(raw) = &e.raw {
                s.cleaned += 1;
                s.words_changed += changed_words(raw, &e.text);
            }
            let app = e
                .app
                .as_deref()
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .unwrap_or(OTHER_APP);
            *per_app.entry(app.to_string()).or_default() += words;
            if words > 0 {
                *s.daily.entry(date_of(e.ts)).or_default() += words;
            }
        }
        s.wpm = (s.speech_secs >= MIN_SPEECH_FOR_WPM)
            .then(|| s.total_words as f32 / (s.speech_secs / 60.0));
        s.minutes_saved = (s.total_words as f32 / TYPING_WPM - s.speech_secs / 60.0).max(0.0);
        s.apps = per_app.into_iter().filter(|(_, w)| *w > 0).collect();
        s.apps.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let sunday = week_start(today);
        s.words_this_week = s.daily.range(sunday..=today).map(|(_, w)| w).sum();
        let (current, longest) = streaks(&s.daily, today);
        s.current_streak = current;
        s.longest_streak = longest;
        s
    }

    /// Named apps only; "Other" is not an app.
    pub fn apps_used(&self) -> usize {
        self.apps.iter().filter(|(a, _)| a != OTHER_APP).count()
    }

    /// First day (a Sunday) of the heatmap grid.
    pub fn grid_start(today: NaiveDate) -> NaiveDate {
        week_start(today) - Duration::weeks(HEATMAP_WEEKS as i64 - 1)
    }

    pub fn words_on(&self, day: NaiveDate) -> u64 {
        self.daily.get(&day).copied().unwrap_or(0)
    }

    /// Busiest day inside the heatmap grid.
    pub fn grid_max(&self, today: NaiveDate) -> u64 {
        let start = Self::grid_start(today);
        self.daily
            .range(start..=today)
            .map(|(_, w)| *w)
            .max()
            .unwrap_or(0)
    }
}

/// Sunday on or before `day`.
fn week_start(day: NaiveDate) -> NaiveDate {
    day - Duration::days(day.weekday().num_days_from_sunday() as i64)
}

/// Heatmap level, 0 (empty) to 4, relative to the busiest day.
pub fn level_for(words: u64, max: u64) -> u8 {
    if words == 0 || max == 0 {
        return 0;
    }
    let f = words as f32 / max as f32;
    if f > 0.75 {
        4
    } else if f > 0.5 {
        3
    } else if f > 0.25 {
        2
    } else {
        1
    }
}

/// (current, longest) run of consecutive days with dictation. Current ends
/// today, or yesterday when nothing is dictated yet today.
fn streaks(daily: &BTreeMap<NaiveDate, u64>, today: NaiveDate) -> (u32, u32) {
    let mut longest = 0;
    let mut run = 0;
    let mut prev: Option<NaiveDate> = None;
    for day in daily.keys() {
        run = match prev {
            Some(p) if *day - p == Duration::days(1) => run + 1,
            _ => 1,
        };
        longest = longest.max(run);
        prev = Some(*day);
    }
    let mut cursor = if daily.contains_key(&today) {
        today
    } else {
        today - Duration::days(1)
    };
    let mut current = 0;
    while daily.contains_key(&cursor) {
        current += 1;
        cursor -= Duration::days(1);
    }
    (current, longest)
}

/// How many words differ between the raw transcript and the polished one:
/// the larger of the words dropped and the words added, so a replaced word
/// counts once.
fn changed_words(raw: &str, polished: &str) -> u64 {
    let a: Vec<&str> = raw.split_whitespace().collect();
    let b: Vec<&str> = polished.split_whitespace().collect();
    let (n, m) = (a.len(), b.len());
    // An utterance is short. Past this the quadratic walk is not worth it, and
    // the length difference is a fair floor.
    if n * m > 250_000 {
        return n.abs_diff(m) as u64;
    }
    let mut prev = vec![0usize; m + 1];
    for x in &a {
        let mut cur = vec![0usize; m + 1];
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = if x == y {
                prev[j] + 1
            } else {
                prev[j + 1].max(cur[j])
            };
        }
        prev = cur;
    }
    let common = prev[m];
    (n - common).max(m - common) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Timestamps in these tests are whole days since the epoch.
    fn date_of(ts: u64) -> NaiveDate {
        day(1970, 1, 1) + Duration::days(ts as i64)
    }

    fn at(d: NaiveDate) -> u64 {
        (d - day(1970, 1, 1)).num_days() as u64
    }

    fn entry(d: NaiveDate, words: usize, dur: f32, app: Option<&str>, raw: Option<&str>) -> Entry {
        Entry {
            ts: at(d),
            dur_s: dur,
            infer_s: 0.1,
            text: vec!["word"; words].join(" "),
            raw: raw.map(str::to_string),
            app: app.map(str::to_string),
        }
    }

    /// A Tuesday.
    fn today() -> NaiveDate {
        day(2025, 10, 7)
    }

    fn streak_of(days: &[NaiveDate]) -> (u32, u32) {
        let entries: Vec<Entry> = days.iter().map(|d| entry(*d, 5, 2.0, None, None)).collect();
        let s = Stats::compute(&entries, today(), date_of);
        (s.current_streak, s.longest_streak)
    }

    #[test]
    fn empty_history_is_all_zero() {
        let s = Stats::compute(&[], today(), date_of);
        assert_eq!(s, Stats::default());
        assert_eq!(s.wpm, None);
        assert_eq!(s.apps_used(), 0);
    }

    #[test]
    fn a_single_day_today_is_a_one_day_streak() {
        assert_eq!(streak_of(&[today()]), (1, 1));
    }

    #[test]
    fn nothing_yet_today_still_counts_yesterdays_streak() {
        let t = today();
        assert_eq!(
            streak_of(&[t - Duration::days(2), t - Duration::days(1)]),
            (2, 2)
        );
    }

    #[test]
    fn a_gap_ends_the_current_streak_and_keeps_the_longest() {
        let t = today();
        let days = [
            t - Duration::days(9),
            t - Duration::days(8),
            t - Duration::days(7),
            t - Duration::days(6),
            t - Duration::days(1),
            t,
        ];
        assert_eq!(streak_of(&days), (2, 4));
    }

    #[test]
    fn two_days_ago_is_no_current_streak() {
        assert_eq!(streak_of(&[today() - Duration::days(2)]), (0, 1));
    }

    #[test]
    fn wpm_waits_for_half_a_minute_of_speech() {
        let t = today();
        let short = Stats::compute(&[entry(t, 100, 29.0, None, None)], t, date_of);
        assert_eq!(short.wpm, None);
        let long = Stats::compute(&[entry(t, 100, 60.0, None, None)], t, date_of);
        assert_eq!(long.wpm, Some(100.0));
    }

    #[test]
    fn minutes_saved_is_typing_time_minus_speaking_time_and_never_negative() {
        let t = today();
        // 400 words: 10 min typing, 2 min speaking.
        let s = Stats::compute(&[entry(t, 400, 120.0, None, None)], t, date_of);
        assert!((s.minutes_saved - 8.0).abs() < 1e-4);
        // Slower than typing: floored at zero.
        let s = Stats::compute(&[entry(t, 10, 120.0, None, None)], t, date_of);
        assert_eq!(s.minutes_saved, 0.0);
    }

    #[test]
    fn apps_group_sort_and_collect_the_unnamed_under_other() {
        let t = today();
        let entries = [
            entry(t, 10, 5.0, Some("Mail"), None),
            entry(t, 30, 5.0, Some("Notes"), None),
            entry(t, 5, 5.0, Some("Mail"), None),
            entry(t, 7, 5.0, None, None),
            entry(t, 4, 5.0, Some("  "), None),
        ];
        let s = Stats::compute(&entries, t, date_of);
        assert_eq!(
            s.apps,
            vec![
                ("Notes".to_string(), 30),
                ("Mail".to_string(), 15),
                (OTHER_APP.to_string(), 11),
            ]
        );
        assert_eq!(s.apps_used(), 2);
    }

    #[test]
    fn cleaned_counts_only_dictations_with_a_raw() {
        let t = today();
        let mut polished = entry(t, 3, 2.0, None, Some("word um word word"));
        polished.text = "word word word".into();
        let s = Stats::compute(&[polished, entry(t, 3, 2.0, None, None)], t, date_of);
        assert_eq!(s.cleaned, 1);
        assert_eq!(s.words_changed, 1);
    }

    #[test]
    fn a_replaced_word_counts_once() {
        assert_eq!(changed_words("meet at twenty", "meet at 20"), 1);
        assert_eq!(changed_words("a b c", "a b c"), 0);
    }

    #[test]
    fn this_week_runs_from_sunday() {
        let t = today(); // Tuesday; its Sunday is Oct 5
        let entries = [
            entry(day(2025, 10, 4), 50, 5.0, None, None),
            entry(day(2025, 10, 5), 20, 5.0, None, None),
            entry(t, 5, 5.0, None, None),
        ];
        let s = Stats::compute(&entries, t, date_of);
        assert_eq!(s.words_this_week, 25);
        assert_eq!(Stats::grid_start(t), day(2025, 10, 5) - Duration::weeks(19));
    }

    #[test]
    fn levels_scale_to_the_busiest_day() {
        assert_eq!(level_for(0, 100), 0);
        assert_eq!(level_for(10, 100), 1);
        assert_eq!(level_for(40, 100), 2);
        assert_eq!(level_for(60, 100), 3);
        assert_eq!(level_for(100, 100), 4);
    }
}
