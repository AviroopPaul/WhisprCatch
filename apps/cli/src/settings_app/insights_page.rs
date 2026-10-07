//! The Insights page: stat cards, app usage and the streak heatmap, all drawn
//! from [`Stats`] (computed once, in `App::new` / `reload_history`).

use chrono::{Datelike, Duration, NaiveDate};
use eframe::egui;
use egui_phosphor::regular as icons;

use super::{App, Section};
use crate::insights::{level_for, Stats, HEATMAP_WEEKS, OTHER_APP, TYPING_WPM};
use crate::theme;

/// Below this page width the cards stack instead of sitting side by side.
const STACK_BELOW: f32 = 640.0;
const GAP: f32 = 16.0;
/// Card chrome: 20px inner margin plus the 1px ring, each side.
const CARD_CHROME: f32 = 42.0;
/// The gauge's full scale.
const GAUGE_MAX_WPM: f32 = 200.0;
/// Card inner width needed for a title and its readout on one line.
const TITLE_ROW_MIN: f32 = 400.0;
const MAX_APP_ROWS: usize = 6;
const DAY_LABELS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

impl App {
    pub(super) fn insights_page(&mut self, ui: &mut egui::Ui) {
        theme::page_header_with(ui, "Insights", None, |_| {});
        ui.add_space(20.0);

        if !self.cfg.history {
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(
                            "History is off, so there is nothing to count. \
                             Turn it on in Settings > General.",
                        )
                        .size(14.0)
                        .color(theme::TEXT_2),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::button(ui, theme::Variant::Secondary, "Open Settings").clicked()
                        {
                            self.modal = Some(Section::General);
                        }
                    });
                });
            });
            ui.add_space(GAP);
            if self.stats.dictations == 0 {
                return;
            }
        }

        let stats = &self.stats;
        let today = self.today;
        let w = ui.available_width();
        let wide = w >= STACK_BELOW;
        let cols = |n: usize| if wide { (w - GAP * (n as f32 - 1.0)) / n as f32 } else { w };

        // Row 1: three stat cards.
        let cw = cols(3);
        flow(ui, wide, |ui, i| match i {
            0 => card_at(ui, cw, 236.0, |ui, _| wpm_card(ui, stats)),
            1 => card_at(ui, cw, 236.0, |ui, _| saved_card(ui, stats)),
            _ => card_at(ui, cw, 236.0, |ui, _| words_card(ui, stats)),
        });
        ui.add_space(GAP);

        // Row 2: app usage and the streak.
        let cw = cols(2);
        let rows = stats.apps.len().clamp(1, MAX_APP_ROWS) as f32;
        let title_h = if cw - CARD_CHROME >= TITLE_ROW_MIN { 32.0 } else { 56.0 };
        let h = (CARD_CHROME + title_h + 18.0 + rows * 40.0).max(CARD_CHROME + title_h + 14.0 + 150.0);
        flow2(ui, wide, |ui, i| match i {
            0 => card_at(ui, cw, h, |ui, inner| apps_card(ui, stats, inner)),
            _ => card_at(ui, cw, h, |ui, inner| streak_card(ui, stats, today, inner)),
        });

        ui.add_space(22.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(egui::RichText::new(icons::LOCK_SIMPLE).size(14.0).color(theme::MUTED));
            ui.label(
                egui::RichText::new(
                    "Worked out on this Mac from your history. Nothing is sent anywhere.",
                )
                .size(12.5)
                .color(theme::MUTED),
            );
        });
    }
}

/// Three cards in a row, or stacked.
fn flow(ui: &mut egui::Ui, wide: bool, mut card: impl FnMut(&mut egui::Ui, usize)) {
    if wide {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for i in 0..3 {
                card(ui, i);
            }
        });
    } else {
        ui.spacing_mut().item_spacing.y = GAP;
        for i in 0..3 {
            card(ui, i);
        }
    }
}

/// Two cards in a row, or stacked.
fn flow2(ui: &mut egui::Ui, wide: bool, mut card: impl FnMut(&mut egui::Ui, usize)) {
    if wide {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for i in 0..2 {
                card(ui, i);
            }
        });
    } else {
        ui.spacing_mut().item_spacing.y = GAP;
        for i in 0..2 {
            card(ui, i);
        }
    }
}

/// One card, `w` wide and at least `h` tall. `add` also gets the inner width.
fn card_at(ui: &mut egui::Ui, w: f32, h: f32, add: impl FnOnce(&mut egui::Ui, f32)) {
    let inner = (w - CARD_CHROME).max(80.0);
    ui.allocate_ui_with_layout(
        egui::vec2(w, h),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            theme::card(ui).show(ui, |ui| {
                ui.set_width(inner);
                ui.set_min_height(h - CARD_CHROME);
                ui.spacing_mut().item_spacing.y = 0.0;
                add(ui, inner);
            });
        },
    );
}

fn big_number(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(theme::semibold(34.0)).color(theme::FG));
}

/// Mono uppercase caption, with an optional info icon and tooltip after it.
fn caption(ui: &mut egui::Ui, text: &str, tip: Option<&str>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(theme::mono_upper(text, 11.5, theme::TEXT_2));
        if let Some(tip) = tip {
            info(ui, tip);
        }
    });
}

pub(super) fn info(ui: &mut egui::Ui, tip: &str) {
    ui.add(
        egui::Label::new(egui::RichText::new(icons::INFO).size(14.0).color(theme::MUTED))
            .sense(egui::Sense::hover()),
    )
    .on_hover_text(tip);
}

/// A body line with an info icon at the right edge.
fn info_row(ui: &mut egui::Ui, text: &str, tip: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.scope(|ui| {
            ui.set_max_width((ui.available_width() - 24.0).max(40.0));
            ui.add(egui::Label::new(egui::RichText::new(text).size(14.0).color(theme::FG)).wrap());
        });
        info(ui, tip);
    });
}

/// A label on the left, a value on the right.
fn kv_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).size(14.0).color(theme::TEXT_2));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(value).font(theme::medium(14.0)).color(theme::FG));
        });
    });
}

fn wpm_card(ui: &mut egui::Ui, s: &Stats) {
    let wpm = s.wpm.unwrap_or(0.0);
    big_number(ui, &format!("{}", wpm.round() as u32));
    ui.add_space(2.0);
    caption(
        ui,
        "Words per minute",
        Some("Words divided by minutes of speech, from your history"),
    );
    ui.add_space(14.0);
    gauge(ui, wpm);
}

/// A semicircle: track, then an arc filled to `wpm` of [`GAUGE_MAX_WPM`], with
/// the speed-up over typing in the middle.
fn gauge(ui: &mut egui::Ui, wpm: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 92.0), egui::Sense::hover());
    let thick = 14.0;
    let r = 72.0_f32.min(width / 2.0 - thick);
    let c = egui::pos2(rect.center().x, rect.top() + r + thick / 2.0);
    let arc = |frac: f32| -> Vec<egui::Pos2> {
        let n = 56;
        (0..=n)
            .map(|i| {
                let a = std::f32::consts::PI * (1.0 + frac * i as f32 / n as f32);
                egui::pos2(c.x + r * a.cos(), c.y + r * a.sin())
            })
            .collect()
    };
    let p = ui.painter();
    let draw = |frac: f32, color: egui::Color32| {
        let pts = arc(frac);
        p.add(egui::Shape::line(pts.clone(), egui::Stroke::new(thick, color)));
        for end in [pts[0], pts[pts.len() - 1]] {
            p.circle_filled(end, thick / 2.0, color);
        }
    };
    draw(1.0, theme::SURFACE_3);
    let frac = (wpm / GAUGE_MAX_WPM).clamp(0.0, 1.0);
    if frac > 0.0 {
        draw(frac, theme::ACCENT);
    }
    let times = wpm / TYPING_WPM;
    p.text(
        egui::pos2(c.x, c.y - 24.0),
        egui::Align2::CENTER_CENTER,
        format!("{times:.1}x"),
        theme::semibold(22.0),
        theme::FG,
    );
    p.text(
        egui::pos2(c.x, c.y - 4.0),
        egui::Align2::CENTER_CENTER,
        "faster than typing",
        egui::FontId::proportional(11.0),
        theme::TEXT_2,
    );
}

fn saved_card(ui: &mut egui::Ui, s: &Stats) {
    big_number(ui, &group_digits(s.minutes_saved.round() as u64));
    ui.add_space(2.0);
    caption(ui, "Minutes saved", Some("Time to type these words at 40 words per minute, minus time spent speaking"));
    ui.add_space(18.0);
    theme::hairline(ui);
    ui.add_space(18.0);
    ui.spacing_mut().item_spacing.y = 14.0;
    info_row(
        ui,
        &format!("{} {}", group_digits(s.dictations), plural(s.dictations, "dictation", "dictations")),
        "Every time you held the key and spoke",
    );
    info_row(
        ui,
        &format!("{} cleaned up by Text cleanup", group_digits(s.cleaned)),
        &format!(
            "Dictations where Text cleanup changed the text, {} {} in all",
            group_digits(s.words_changed),
            plural(s.words_changed, "word", "words")
        ),
    );
}

fn words_card(ui: &mut egui::Ui, s: &Stats) {
    big_number(ui, &group_digits(s.total_words));
    ui.add_space(2.0);
    caption(ui, "Total words dictated", None);
    ui.add_space(18.0);
    theme::hairline(ui);
    ui.add_space(18.0);
    ui.spacing_mut().item_spacing.y = 14.0;
    kv_row(
        ui,
        "This week",
        &format!("{} {}", group_digits(s.words_this_week), plural(s.words_this_week, "word", "words")),
    );
    kv_row(ui, "Speaking time", &duration_label(s.speech_secs));
}

/// Card title with a mono readout on the right, or under it when the card is
/// too narrow for both on one line.
fn card_title(ui: &mut egui::Ui, title: &str, right: &str, inner: f32) {
    let title = egui::RichText::new(title).font(theme::semibold(24.0)).color(theme::FG);
    if inner >= TITLE_ROW_MIN {
        ui.horizontal(|ui| {
            ui.label(title);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(theme::mono_upper(right, 11.5, theme::TEXT_2));
            });
        });
    } else {
        ui.label(title);
        ui.add_space(4.0);
        ui.label(theme::mono_upper(right, 11.5, theme::TEXT_2));
    }
}

fn apps_card(ui: &mut egui::Ui, s: &Stats, inner: f32) {
    card_title(ui, "App usage", &format!("Apps used | {}", s.apps_used()), inner);
    ui.add_space(18.0);
    if s.apps.is_empty() {
        ui.label(
            egui::RichText::new("Dictate in a few apps to see where your words go.")
                .size(14.0)
                .color(theme::MUTED),
        );
        return;
    }
    let total: u64 = s.apps.iter().map(|(_, w)| w).sum::<u64>().max(1);
    // The biggest app gets the full bar; the rest scale against it.
    let top_share = (s.apps[0].1 as f32 / total as f32).max(0.01);
    let icon_w = 30.0;
    let label_w = (inner * 0.55).clamp(120.0, 190.0);
    let bar_max = (inner - icon_w - label_w - 12.0).max(44.0);
    for (name, words) in s.apps.iter().take(MAX_APP_ROWS) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(inner, 40.0), egui::Sense::hover());
        let cy = rect.center().y;
        let p = ui.painter();
        let icon = if name == OTHER_APP { icons::DOTS_THREE_CIRCLE } else { icons::APP_WINDOW };
        p.text(
            egui::pos2(rect.left() + 2.0, cy),
            egui::Align2::LEFT_CENTER,
            icon,
            egui::FontId::proportional(20.0),
            theme::TEXT_2,
        );
        let share = *words as f32 / total as f32;
        let pct = if share > 0.0 && share < 0.01 {
            "<1%".to_string()
        } else {
            format!("{:.0}%", share * 100.0)
        };
        let bar_w = (bar_max * share / top_share).max(44.0);
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + icon_w, cy - 15.0),
            egui::vec2(bar_w, 30.0),
        );
        let big = bar_w >= 64.0;
        let (fill, text_color) = if big {
            (theme::ACCENT, theme::ON_ACCENT)
        } else {
            (theme::tint_strong(theme::ACCENT), theme::ACCENT)
        };
        p.rect_filled(bar, 6.0, fill);
        p.text(
            bar.center(),
            egui::Align2::CENTER_CENTER,
            pct,
            theme::medium(13.0),
            text_color,
        );
        let label = egui::Rect::from_min_size(
            egui::pos2(bar.right() + 12.0, cy - 10.0),
            egui::vec2((rect.right() - bar.right() - 12.0).max(40.0), 20.0),
        );
        let text = format!("{} {} · {}", group_digits(*words), plural(*words, "word", "words"), name);
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(label)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.add(egui::Label::new(theme::mono_upper(&text, 11.5, theme::TEXT_2)).truncate());
            },
        );
    }
}

fn streak_card(ui: &mut egui::Ui, s: &Stats, today: NaiveDate, inner: f32) {
    let n = s.current_streak;
    card_title(
        ui,
        &format!("{n} day streak"),
        &format!("Longest streak | {} {}", s.longest_streak, plural(s.longest_streak as u64, "day", "days")),
        inner,
    );
    ui.add_space(14.0);
    heatmap(ui, s, today, inner);
}

/// 7 rows (Sunday first) by [`HEATMAP_WEEKS`] columns, oldest week on the left.
fn heatmap(ui: &mut egui::Ui, s: &Stats, today: NaiveDate, inner: f32) {
    let gap = 3.0;
    let label_w = 34.0;
    let cell = ((inner - label_w - gap * (HEATMAP_WEEKS as f32 - 1.0)) / HEATMAP_WEEKS as f32)
        .clamp(8.0, 13.0);
    let step = cell + gap;
    let month_h = 20.0;
    let size = egui::vec2(label_w + step * HEATMAP_WEEKS as f32, month_h + step * 7.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    let origin = egui::pos2(rect.left() + label_w, rect.top() + month_h);
    let start = Stats::grid_start(today);
    let max = s.grid_max(today);
    let day_at = |col: usize, row: usize| start + Duration::days((col * 7 + row) as i64);
    let cell_rect = |col: usize, row: usize| {
        egui::Rect::from_min_size(
            egui::pos2(origin.x + col as f32 * step, origin.y + row as f32 * step),
            egui::vec2(cell, cell),
        )
    };

    let hover = resp.hover_pos().and_then(|pos| {
        let (x, y) = (pos.x - origin.x, pos.y - origin.y);
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let (col, row) = ((x / step) as usize, (y / step) as usize);
        (col < HEATMAP_WEEKS && row < 7 && day_at(col, row) <= today).then_some((col, row))
    });

    let p = ui.painter();
    let label_font = egui::FontId::proportional(11.0);
    for (row, name) in DAY_LABELS.iter().enumerate() {
        p.text(
            egui::pos2(rect.left(), cell_rect(0, row).center().y),
            egui::Align2::LEFT_CENTER,
            *name,
            label_font.clone(),
            theme::MUTED,
        );
    }
    for col in 0..HEATMAP_WEEKS {
        // A month is named over the column that holds its 1st.
        if let Some(first) = (0..7).map(|r| day_at(col, r)).find(|d| d.day() == 1) {
            p.text(
                egui::pos2(origin.x + col as f32 * step, rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                month_name(first.month()),
                label_font.clone(),
                theme::MUTED,
            );
        }
        for row in 0..7 {
            let day = day_at(col, row);
            if day > today {
                continue;
            }
            let level = level_for(s.words_on(day), max);
            p.rect_filled(cell_rect(col, row), 3.0, theme::heat(level));
        }
    }
    if let Some((col, row)) = hover {
        p.rect_stroke(
            cell_rect(col, row).expand(1.0),
            3.0,
            egui::Stroke::new(1.0, theme::FG),
            egui::StrokeKind::Outside,
        );
        let day = day_at(col, row);
        let words = s.words_on(day);
        let tip = format!(
            "{}, {} {}: {} {}",
            DAY_LABELS[row],
            month_name(day.month()),
            day.day(),
            group_digits(words),
            plural(words, "word", "words")
        );
        resp.on_hover_text_at_pointer(tip);
    }

    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add_space(label_w);
        ui.label(egui::RichText::new("Less").size(11.5).color(theme::MUTED));
        ui.add_space(2.0);
        for level in 1..=4 {
            let (r, _) = ui.allocate_exact_size(egui::vec2(cell, cell), egui::Sense::hover());
            ui.painter().rect_filled(r, 3.0, theme::heat(level));
        }
        ui.add_space(2.0);
        ui.label(egui::RichText::new("More").size(11.5).color(theme::MUTED));
    });
}

fn month_name(m: u32) -> &'static str {
    const NAMES: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    NAMES[(m as usize).saturating_sub(1).min(11)]
}

fn plural(n: u64, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 {
        one
    } else {
        many
    }
}

/// 2077 -> "2,077".
fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "1h 12m", "12m", "45s".
fn duration_label(secs: f32) -> String {
    let secs = secs.max(0.0).round() as u64;
    match (secs / 3600, secs / 60 % 60) {
        (0, 0) => format!("{secs}s"),
        (0, m) => format!("{m}m"),
        (h, m) => format!("{h}h {m}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_are_grouped_in_threes() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(2077), "2,077");
        assert_eq!(group_digits(1_234_567), "1,234,567");
    }

    #[test]
    fn speaking_time_reads_naturally() {
        assert_eq!(duration_label(45.0), "45s");
        assert_eq!(duration_label(12.0 * 60.0), "12m");
        assert_eq!(duration_label(72.0 * 60.0), "1h 12m");
    }
}
