//! Settings & history window (eframe/egui), launched as
//! `whisper-catch settings [--tab history|settings]` — from the tray menu
//! or the shell.
//!
//! Layout per docs/DESIGN.md: top header with a centered segmented control;
//! History = 288px sidebar (search + chronological list) + a detail pane
//! with metadata and copy/delete; Settings = sections with small mono
//! uppercase headings. Dark-only, "tactile engineer" language.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use anyhow::Result;
use eframe::egui;
use egui_phosphor::regular as icons;
use wc_models::ModelId;

use crate::{autostart, config, theme};
use wc_core::history;

const SIDEBAR_W: f32 = 288.0;
const SETTINGS_COL: f32 = 560.0;
const GITHUB_URL: &str = "https://github.com/AviroopPaul/whisper-catch";
const SITE_URL: &str = "https://whisper-catch.vercel.app";

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    History,
    Settings,
}

/// Opening size of the settings window, in points.
const WINDOW_W: f32 = 1000.0;
const WINDOW_H: f32 = 680.0;

/// Dev-only: `WC_DEMO_HISTORY=1` swaps the real transcript log for a fixed
/// sample set. Screenshots for the README and the website are taken with this
/// on, so nobody's actual dictation ends up published.
fn demo_history() -> Option<(Vec<history::Entry>, (u64, u64, f32))> {
    if std::env::var("WC_DEMO_HISTORY").is_err() {
        return None;
    }
    // Fixed timestamps so consecutive captures are identical.
    let base = 1_760_000_000_u64;
    let rows: [(u64, f32, f32, &str); 7] = [
        (base, 11.2, 0.31,
         "Morning. The migration finished overnight and nothing looks broken, \
          but I'd like a second pair of eyes on the rollback path before we \
          call it done. Can you take a look this afternoon?"),
        (base - 900, 6.4, 0.19,
         "Push the release notes to the draft branch and I'll do a pass on \
          the wording tonight."),
        (base - 2_400, 18.7, 0.44,
         "Three things from standup. The flaky overlay test is quarantined, \
          the Wayland fallback landed behind a flag, and we still need someone \
          to own the notarisation ticket before the next tag."),
        (base - 5_100, 4.1, 0.12,
         "Reply to Priya: yes to Thursday, and I'll bring the latency numbers."),
        (base - 9_800, 26.3, 0.61,
         "Longer one. The reason inference feels instant is that the model is \
          already resident when the key goes down, so the only work left on \
          release is the tail of the audio. That's why the first word appears \
          before you've finished the sentence."),
        (base - 14_200, 9.8, 0.27,
         "Note to self: check whether the 300 millisecond pre-roll is still \
          enough on the older MacBook."),
        (base - 21_600, 3.2, 0.09, ""),
    ];
    let entries: Vec<history::Entry> = rows
        .iter()
        .map(|(ts, dur_s, infer_s, text)| history::Entry {
            ts: *ts,
            dur_s: *dur_s,
            infer_s: *infer_s,
            text: (*text).to_string(),
            // sample rows are already what the polish chain would emit
            raw: None,
        })
        .collect();
    let words = entries
        .iter()
        .map(|e| e.text.split_whitespace().count() as u64)
        .sum();
    let secs = entries.iter().map(|e| e.dur_s).sum();
    let count = entries.len() as u64;
    Some((entries, (count, words, secs)))
}

pub fn run(tab: Option<String>) -> Result<()> {
    let cfg = config::load().unwrap_or_default();
    let options = eframe::NativeOptions {
        // A utility window, not a workspace: opening maximized left a narrow
        // column of content stranded in the middle of a 27" display. Geometry
        // is deliberately not persisted either — a remembered 1440-wide window
        // would keep reproducing that, and this always opens composed.
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WINDOW_W, WINDOW_H])
            .with_min_inner_size([720.0, 480.0]),
        centered: true,
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        crate::app_name(),
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            theme::install_fonts(&cc.egui_ctx);
            Ok(Box::new(App::new(cfg, tab.as_deref())) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| anyhow::anyhow!("settings window failed: {e}"))
}

/// Background model download driven from Settings → Engine Parameters.
struct ModelDl {
    model: ModelId,
    rx: Receiver<DlMsg>,
    file: String,
    done: u64,
    total: u64,
    error: Option<String>,
}

enum DlMsg {
    Progress { file: String, done: u64, total: u64 },
    Finished,
    Failed(String),
}

struct App {
    tab: Tab,
    cfg: config::Config,
    autostart_on: bool,
    entries: Vec<history::Entry>,
    totals: (u64, u64, f32),
    status: String,
    saved_ok: bool,
    confirm_clear: bool,
    search: String,
    /// Timestamp of the entry shown in the detail pane.
    selected: Option<u64>,
    /// Entry ts awaiting delete confirmation.
    confirm_delete: Option<u64>,
    /// When the selected transcript was copied — drives the "COPIED" flash.
    copied: Option<Instant>,
    /// In-flight model download, if any.
    dl: Option<ModelDl>,
    /// Cleared after the opening size has been asserted once.
    needs_size: bool,
    shot: crate::shot::Shot,
}

impl App {
    fn new(cfg: config::Config, tab: Option<&str>) -> Self {
        let autostart_on = autostart::is_enabled();
        let (entries, totals) = match demo_history() {
            Some(demo) => demo,
            None => (history::load(500).unwrap_or_default(), history::totals()),
        };
        let selected = entries.first().map(|e| e.ts);
        Self {
            tab: if tab == Some("settings") {
                Tab::Settings
            } else {
                Tab::History
            },
            cfg,
            autostart_on,
            entries,
            totals,
            status: String::new(),
            saved_ok: false,
            confirm_clear: false,
            search: String::new(),
            selected,
            confirm_delete: None,
            copied: None,
            dl: None,
            needs_size: true,
            shot: crate::shot::Shot::from_env(),
        }
    }

    fn selected_model(&self) -> ModelId {
        ModelId::parse(&self.cfg.model)
    }

    fn key_label(&self) -> &str {
        config::key_label(&self.cfg.key)
    }

    fn start_download(&mut self, model: ModelId, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        self.dl = Some(ModelDl {
            model,
            rx,
            file: String::new(),
            done: 0,
            total: model.spec().total_size(),
            error: None,
        });
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let res = model.spec().ensure_with(&wc_core::models_dir(), &|f, d, t| {
                let _ = tx.send(DlMsg::Progress {
                    file: f.to_string(),
                    done: d,
                    total: t,
                });
                ctx.request_repaint();
            });
            let _ = tx.send(match res {
                Ok(_) => DlMsg::Finished,
                Err(e) => DlMsg::Failed(format!("{e:#}")),
            });
            ctx.request_repaint();
        });
    }

    fn poll_download(&mut self) {
        let mut clear = false;
        if let Some(dl) = self.dl.as_mut() {
            while let Ok(msg) = dl.rx.try_recv() {
                match msg {
                    DlMsg::Progress { file, done, total } => {
                        dl.file = file;
                        dl.done = done;
                        dl.total = total;
                    }
                    DlMsg::Finished => clear = true,
                    DlMsg::Failed(e) => dl.error = Some(e),
                }
            }
        }
        if clear {
            self.dl = None;
        }
    }

    fn reload_history(&mut self) {
        if let Some((entries, totals)) = demo_history() {
            self.entries = entries;
            self.totals = totals;
            self.selected = self.entries.first().map(|e| e.ts);
            self.confirm_delete = None;
            return;
        }
        self.entries = history::load(500).unwrap_or_default();
        self.totals = history::totals();
        if !self
            .entries
            .iter()
            .any(|e| Some(e.ts) == self.selected)
        {
            self.selected = self.entries.first().map(|e| e.ts);
        }
        self.confirm_delete = None;
    }
}

// ------------------------------------------------------------- formatting

/// Sidebar timestamp: "TODAY 23:21" / "YESTERDAY 09:12" / "JUL 02 22:28".
fn list_time(ts: u64) -> String {
    let Some(t) = chrono::DateTime::from_timestamp(ts as i64, 0) else {
        return String::new();
    };
    let local = t.with_timezone(&chrono::Local);
    let today = chrono::Local::now().date_naive();
    let d = local.date_naive();
    if d == today {
        format!("today {}", local.format("%H:%M"))
    } else if today.pred_opt() == Some(d) {
        format!("yesterday {}", local.format("%H:%M"))
    } else {
        local.format("%b %d %H:%M").to_string()
    }
}

/// Detail-pane timestamp: "FRIDAY, JUL 04 · 23:21".
fn detail_time(ts: u64) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%A, %b %d · %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

// -------------------------------------------------------------- widgets

/// Constrains content to a centered column (Settings tab).
fn centered_col<R>(ui: &mut egui::Ui, w_max: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let full = ui.available_width();
    let w = full.min(w_max);
    let pad = ((full - w) / 2.0).max(0.0);
    let mut rect = ui.available_rect_before_wrap();
    rect.min.x += pad;
    rect.max.x = rect.min.x + w;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_width(w);
        add(ui)
    })
    .inner
}

fn seg_button(ui: &mut egui::Ui, selected: bool, label: &str) -> bool {
    let text = egui::RichText::new(label)
        .font(theme::medium(12.5))
        .color(if selected { theme::FG } else { theme::MUTED });
    let btn = egui::Button::new(text)
        .fill(if selected {
            theme::SURFACE_3
        } else {
            egui::Color32::TRANSPARENT
        })
        .stroke(if selected {
            egui::Stroke::new(1.0, theme::RING)
        } else {
            egui::Stroke::NONE
        })
        .corner_radius(egui::CornerRadius::same(6))
        .min_size(egui::vec2(92.0, 24.0));
    ui.add(btn).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Top-center segmented control (History | Settings).
fn segmented(ui: &mut egui::Ui, tab: &mut Tab) {
    egui::Frame::default()
        .fill(theme::SURFACE)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(3.0)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.horizontal(|ui| {
                if seg_button(ui, *tab == Tab::History, "History") {
                    *tab = Tab::History;
                }
                if seg_button(ui, *tab == Tab::Settings, "Settings") {
                    *tab = Tab::Settings;
                }
            });
        });
}

/// Ghost button: transparent fill, hairline ring.
fn ghost_button(ui: &mut egui::Ui, text: impl Into<egui::RichText>) -> egui::Response {
    ui.add(
        egui::Button::new(text.into().font(theme::medium(12.0)).color(theme::TEXT_2))
            .fill(egui::Color32::TRANSPARENT)
            .stroke(egui::Stroke::new(1.0, theme::RING))
            .corner_radius(egui::CornerRadius::same(6)),
    )
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // macOS hands a resizable window back at whatever size it was last
        // seen, which for anyone who ran the old maximized build means a
        // 27-inch window of mostly background. Assert the designed size once,
        // then leave the window alone for the rest of the session.
        if self.needs_size {
            self.needs_size = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                WINDOW_W, WINDOW_H,
            )));
        }
        self.shot.tick(ctx);
        self.poll_download();
        if self.dl.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }

        egui::TopBottomPanel::top("header")
            .exact_height(52.0)
            .frame(
                egui::Frame::default()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(16, 0)),
            )
            .show(ctx, |ui| {
                let narrow = ui.available_width() < 760.0;
                ui.columns(3, |cols| {
                    cols[0].with_layout(
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_height(52.0);
                            theme::led(ui, theme::MINT, false);
                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new(crate::app_name())
                                    .font(theme::semibold(14.0))
                                    .color(theme::FG),
                            );
                        },
                    );
                    cols[1].with_layout(
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_height(52.0);
                            let w = ui.available_width();
                            ui.add_space(((w - 196.0) / 2.0).max(0.0));
                            segmented(ui, &mut self.tab);
                        },
                    );
                    cols[2].with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.set_min_height(52.0);
                            if !narrow {
                                let (n, w, s) = self.totals;
                                ui.label(theme::mono_upper(
                                    &format!("{w} words · {n} utt · {:.0} min", s / 60.0),
                                    10.5,
                                    theme::MUTED,
                                ));
                            }
                        },
                    );
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(theme::BG))
            .show(ctx, |ui| match self.tab {
                Tab::History => self.history_tab(ui),
                Tab::Settings => {
                    egui::Frame::default()
                        .inner_margin(egui::Margin {
                            left: 24,
                            right: 24,
                            top: 20,
                            bottom: 16,
                        })
                        .show(ui, |ui| {
                            centered_col(ui, SETTINGS_COL, |ui| self.settings_tab(ui));
                        });
                }
            });
    }
}

// ---------------------------------------------------------------- history

impl App {
    fn history_tab(&mut self, ui: &mut egui::Ui) {
        if self.entries.is_empty() {
            self.history_empty_state(ui);
            return;
        }

        egui::SidePanel::left("history-list")
            .exact_width(SIDEBAR_W)
            .resizable(false)
            .frame(
                egui::Frame::default()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin {
                        left: 16,
                        right: 12,
                        top: 14,
                        bottom: 10,
                    }),
            )
            .show_inside(ui, |ui| self.history_sidebar(ui));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin {
                        left: 28,
                        right: 28,
                        top: 18,
                        bottom: 16,
                    }),
            )
            .show_inside(ui, |ui| self.history_detail(ui));
    }

    fn history_empty_state(&mut self, ui: &mut egui::Ui) {
        ui.add_space((ui.available_height() * 0.32).clamp(24.0, 260.0));
        ui.vertical_centered(|ui| {
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::hover());
            let p = ui.painter();
            p.circle_filled(rect.center(), 32.0, theme::SURFACE);
            p.circle_stroke(rect.center(), 32.0, egui::Stroke::new(1.0, theme::BORDER));
            p.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                icons::MICROPHONE,
                egui::FontId::proportional(26.0),
                theme::MUTED,
            );
            ui.add_space(18.0);
            theme::display(ui, "Nothing said ", "yet.", 27.0);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let w = 240.0;
                ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(egui::RichText::new("Hold").color(theme::TEXT_2));
                theme::key_chip(ui, self.key_label());
                ui.label(egui::RichText::new("and speak to dictate.").color(theme::TEXT_2));
            });
        });
    }

    fn history_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text(
                    egui::RichText::new(format!("{}  Search", icons::MAGNIFYING_GLASS))
                        .color(theme::MUTED),
                )
                .desired_width(f32::INFINITY),
        );
        ui.add_space(8.0);

        let q = self.search.to_lowercase();
        let shown: Vec<(u64, String, f32)> = self
            .entries
            .iter()
            .filter(|e| q.is_empty() || e.text.to_lowercase().contains(&q))
            .map(|e| (e.ts, e.text.clone(), e.dur_s))
            .collect();

        // keep the selection inside the filtered set
        if !shown.iter().any(|(ts, ..)| Some(*ts) == self.selected) {
            self.selected = shown.first().map(|(ts, ..)| *ts);
        }

        let footer_h = 30.0;
        let list_h = (ui.available_height() - footer_h).max(60.0);
        egui::ScrollArea::vertical()
            .max_height(list_h)
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                if shown.is_empty() {
                    ui.add_space(16.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new("No matches")
                                .small()
                                .color(theme::MUTED),
                        );
                    });
                }
                for (ts, text, dur) in &shown {
                    if self.history_row(ui, *ts, text, *dur) {
                        self.selected = Some(*ts);
                        self.confirm_delete = None;
                        self.copied = None;
                    }
                }
            });

        // footer: count + clear-all with inline confirm
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(theme::mono_upper(
                &format!("{} transcripts", shown.len()),
                10.0,
                theme::MUTED,
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.confirm_clear {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Delete all")
                                    .font(theme::medium(11.0))
                                    .color(theme::RED),
                            )
                            .fill(theme::tint(theme::RED))
                            .stroke(egui::Stroke::NONE)
                            .corner_radius(egui::CornerRadius::same(4)),
                        )
                        .clicked()
                    {
                        let _ = history::clear();
                        self.reload_history();
                        self.confirm_clear = false;
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Keep")
                                    .font(theme::medium(11.0))
                                    .color(theme::TEXT_2),
                            )
                            .fill(egui::Color32::TRANSPARENT)
                            .stroke(egui::Stroke::new(1.0, theme::RING))
                            .corner_radius(egui::CornerRadius::same(4)),
                        )
                        .clicked()
                    {
                        self.confirm_clear = false;
                    }
                } else if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Clear all")
                                .font(theme::medium(11.0))
                                .color(theme::MUTED),
                        )
                        .fill(egui::Color32::TRANSPARENT)
                        .stroke(egui::Stroke::NONE)
                        .corner_radius(egui::CornerRadius::same(4)),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    self.confirm_clear = true;
                }
            });
        });
    }

    /// One sidebar row: mono uppercase timestamp + duration, then a 2-line
    /// clamped preview. Returns true when clicked.
    fn history_row(&self, ui: &mut egui::Ui, ts: u64, text: &str, dur: f32) -> bool {
        let selected = Some(ts) == self.selected;
        let resp = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .id_salt(ts)
                    .sense(egui::Sense::click()),
                |ui| {
                    let fill = if selected {
                        theme::SURFACE
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    egui::Frame::default()
                        .fill(fill)
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(egui::Margin::symmetric(10, 8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 4.0;
                            ui.horizontal(|ui| {
                                ui.label(theme::mono_upper(
                                    &list_time(ts),
                                    10.0,
                                    if selected { theme::TEXT_2 } else { theme::MUTED },
                                ));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(theme::mono_upper(
                                            &format!("{dur:.1}s"),
                                            10.0,
                                            theme::MUTED,
                                        ));
                                    },
                                );
                            });
                            // A silent utterance still gets a row; without this
                            // it renders as a blank gap that reads as a bug.
                            let blank = text.trim().is_empty();
                            let preview = if blank { "(nothing was said)" } else { text };
                            let mut job = egui::text::LayoutJob::single_section(
                                preview.to_owned(),
                                egui::TextFormat {
                                    font_id: egui::FontId::proportional(12.5),
                                    color: if blank {
                                        theme::MUTED
                                    } else if selected {
                                        theme::FG
                                    } else {
                                        theme::TEXT_2
                                    },
                                    italics: blank,
                                    ..Default::default()
                                },
                            );
                            job.wrap = egui::text::TextWrapping {
                                max_width: ui.available_width(),
                                max_rows: 2,
                                break_anywhere: false,
                                overflow_character: Some('…'),
                            };
                            ui.add(egui::Label::new(job).selectable(false));
                        });
                },
            )
            .response;
        if selected {
            ui.painter().rect_stroke(
                resp.rect,
                egui::CornerRadius::same(6),
                egui::Stroke::new(1.0, theme::RING),
                egui::StrokeKind::Inside,
            );
            // mint rail on the selected row, as on the website's feature list
            let r = resp.rect;
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(r.left(), r.top() + 6.0),
                    egui::vec2(2.0, r.height() - 12.0),
                ),
                egui::CornerRadius::same(1),
                theme::MINT,
            );
        } else if resp.hovered() {
            ui.painter().rect_stroke(
                resp.rect,
                egui::CornerRadius::same(6),
                egui::Stroke::new(1.0, theme::BORDER),
                egui::StrokeKind::Inside,
            );
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    }

    fn history_detail(&mut self, ui: &mut egui::Ui) {
        let Some(entry) = self
            .entries
            .iter()
            .find(|e| Some(e.ts) == self.selected)
            .cloned()
        else {
            return;
        };

        let words = entry.text.split_whitespace().count();
        let mut do_copy = false;
        let mut do_delete = false;

        ui.horizontal(|ui| {
            ui.label(theme::mono_upper(
                &detail_time(entry.ts),
                11.0,
                theme::MUTED,
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if self.confirm_delete == Some(entry.ts) {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(format!("{} Confirm", icons::TRASH))
                                    .font(theme::medium(12.0))
                                    .color(theme::RED),
                            )
                            .fill(theme::tint(theme::RED))
                            .stroke(egui::Stroke::NONE)
                            .corner_radius(egui::CornerRadius::same(6)),
                        )
                        .clicked()
                    {
                        do_delete = true;
                    }
                } else if ghost_button(
                    ui,
                    egui::RichText::new(format!("{} Delete", icons::TRASH)),
                )
                .clicked()
                {
                    self.confirm_delete = Some(entry.ts);
                }
                let flash = self
                    .copied
                    .is_some_and(|at| at.elapsed() < Duration::from_millis(1500));
                if flash {
                    ui.label(theme::mono_upper("copied", 10.5, theme::MINT));
                    ui.ctx().request_repaint_after(Duration::from_millis(200));
                } else if ghost_button(
                    ui,
                    egui::RichText::new(format!("{} Copy", icons::COPY)),
                )
                .clicked()
                {
                    do_copy = true;
                }
            });
        });
        ui.add_space(2.0);
        ui.label(theme::mono_upper(
            &format!(
                "{:.1}s spoken · {words} words · {:.2}s inference",
                entry.dur_s, entry.infer_s
            ),
            10.5,
            theme::MUTED,
        ));
        ui.add_space(12.0);
        // hairline
        let w = ui.available_width();
        let y = ui.cursor().top();
        ui.painter().hline(
            egui::Rangef::new(ui.cursor().left(), ui.cursor().left() + w),
            y,
            egui::Stroke::new(1.0, theme::BORDER),
        );
        ui.add_space(14.0);

        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            // readable measure: cap the transcript column
            ui.set_max_width(680.0);
            if entry.text.trim().is_empty() {
                ui.label(
                    egui::RichText::new("Nothing was said in this one.")
                        .size(15.0)
                        .italics()
                        .color(theme::MUTED),
                );
                return;
            }
            // Newsreader for the transcript itself: this is prose to read,
            // and it is the same face the website sets its headlines in.
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&entry.text)
                        .font(theme::serif(18.0))
                        .color(theme::FG),
                )
                .wrap(),
            );
        });

        if do_copy {
            ui.ctx().copy_text(entry.text.clone());
            self.copied = Some(Instant::now());
        }
        if do_delete {
            let _ = history::delete(entry.ts);
            self.reload_history();
        }
    }
}

// ---------------------------------------------------------------- settings

impl App {
    fn settings_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            theme::section_label(ui, "Engine parameters");
            ui.add_space(6.0);
            self.engine_card(ui);
            ui.add_space(18.0);

            theme::section_label(ui, "Hotkey");
            ui.add_space(6.0);
            self.hotkey_card(ui);
            ui.add_space(18.0);

            theme::section_label(ui, "Output behavior");
            ui.add_space(6.0);
            self.output_card(ui);
            ui.add_space(18.0);

            #[cfg(target_os = "macos")]
            {
                theme::section_label(ui, "Permissions");
                ui.add_space(6.0);
                self.permissions_card(ui);
                ui.add_space(18.0);
            }

            theme::section_label(ui, "About");
            ui.add_space(6.0);
            self.about_card(ui);
            ui.add_space(20.0);

            ui.horizontal(|ui| {
                if theme::primary_button(ui, "Save changes").clicked() {
                    self.save();
                }
                if !self.status.is_empty() {
                    let color = if self.saved_ok { theme::MINT } else { theme::RED };
                    let prefix = if self.saved_ok { icons::CHECK } else { icons::WARNING };
                    ui.label(
                        egui::RichText::new(format!("{prefix} {}", self.status))
                            .small()
                            .color(color),
                    );
                }
            });
            ui.add_space(8.0);
        });
    }

    fn save(&mut self) {
        let mut ok = true;
        if let Err(e) = config::save(&self.cfg) {
            self.status = format!("save failed: {e}");
            ok = false;
        }
        let res = if self.autostart_on {
            autostart::enable()
        } else {
            autostart::disable()
        };
        if let Err(e) = res {
            self.status = format!("autostart failed: {e}");
            ok = false;
        }
        if ok {
            self.status = "Saved. Model and key changes apply after the daemon restarts.".into();
        }
        self.saved_ok = ok;
    }

    /// Label + muted description on the left, control on the right.
    fn setting_row(
        ui: &mut egui::Ui,
        label: &str,
        desc: &str,
        control: impl FnOnce(&mut egui::Ui),
    ) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(egui::RichText::new(label).color(theme::FG));
                if !desc.is_empty() {
                    ui.label(egui::RichText::new(desc).small().color(theme::MUTED));
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
        });
    }

    fn engine_card(&mut self, ui: &mut egui::Ui) {
        let selected = self.selected_model();
        let complete = selected.spec().is_complete(&wc_core::models_dir());
        let downloading = self.dl.as_ref().map(|d| d.model) == Some(selected);
        let mut do_download: Option<ModelId> = None;

        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            Self::setting_row(ui, "Speech model", selected.blurb(), |ui| {
                egui::ComboBox::from_id_salt("model")
                    .selected_text(selected.label())
                    .show_ui(ui, |ui| {
                        for m in ModelId::ALL {
                            ui.selectable_value(
                                &mut self.cfg.model,
                                m.slug().to_string(),
                                m.label(),
                            );
                        }
                    });
            });
            ui.add_space(10.0);
            ui.label(theme::mono_upper(
                &format!("{} · {} MB download", selected.ram_hint(), selected.download_mb()),
                10.0,
                theme::MUTED,
            ));
            ui.add_space(10.0);

            if downloading {
                let dl = self.dl.as_ref().unwrap();
                let frac = if dl.total > 0 {
                    dl.done as f32 / dl.total as f32
                } else {
                    0.0
                };
                ui.add(
                    egui::ProgressBar::new(frac)
                        .desired_height(6.0)
                        .fill(theme::MINT)
                        .corner_radius(egui::CornerRadius::same(4)),
                );
                ui.add_space(6.0);
                if let Some(e) = &dl.error {
                    ui.label(egui::RichText::new(e).small().color(theme::RED));
                } else {
                    ui.label(theme::mono_upper(
                        &format!(
                            "{:.0}% · {:.0} / {:.0} MB · {}",
                            frac * 100.0,
                            dl.done as f64 / 1e6,
                            dl.total as f64 / 1e6,
                            if dl.file.is_empty() { "preparing" } else { &dl.file }
                        ),
                        10.0,
                        theme::MUTED,
                    ));
                }
            } else if complete {
                ui.horizontal(|ui| {
                    theme::led(ui, theme::MINT, false);
                    ui.label(theme::mono_upper("ready", 10.5, theme::MINT));
                    ui.label(theme::mono_upper(
                        "· applies after the daemon restarts",
                        10.0,
                        theme::MUTED,
                    ));
                });
            } else {
                ui.horizontal(|ui| {
                    if ghost_button(
                        ui,
                        egui::RichText::new(format!(
                            "{} Download ({} MB)",
                            icons::DOWNLOAD_SIMPLE,
                            selected.download_mb()
                        )),
                    )
                    .clicked()
                    {
                        do_download = Some(selected);
                    }
                    ui.label(theme::mono_upper("not downloaded", 10.0, theme::MUTED));
                });
            }
        });

        if let Some(m) = do_download {
            let ctx = ui.ctx().clone();
            self.start_download(m, &ctx);
        }
    }

    fn hotkey_card(&mut self, ui: &mut egui::Ui) {
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            Self::setting_row(
                ui,
                "Push-to-talk key",
                "Held to record, released to type",
                |ui| {
                    egui::ComboBox::from_id_salt("key")
                        .selected_text(config::key_label(&self.cfg.key))
                        .show_ui(ui, |ui| {
                            for (k, label) in config::KEYS {
                                ui.selectable_value(&mut self.cfg.key, k.to_string(), *label);
                            }
                        });
                },
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(egui::RichText::new("Hold").small().color(theme::MUTED));
                theme::key_chip(ui, config::key_label(&self.cfg.key));
                ui.label(
                    egui::RichText::new("and speak. Release to type.")
                        .small()
                        .color(theme::MUTED),
                );
            });
        });
    }

    fn output_card(&mut self, ui: &mut egui::Ui) {
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 12.0;
            Self::setting_row(ui, "Live typing", "Words appear while you speak", |ui| {
                theme::toggle(ui, &mut self.cfg.streaming);
            });
            Self::setting_row(
                ui,
                "Recording indicator",
                "Floating pill while dictating",
                |ui| {
                    theme::toggle(ui, &mut self.cfg.overlay);
                },
            );
            Self::setting_row(ui, "Keep history", "Log transcriptions locally", |ui| {
                theme::toggle(ui, &mut self.cfg.history);
            });
            Self::setting_row(ui, "Start on login", "Launch the daemon with your session", |ui| {
                theme::toggle(ui, &mut self.autostart_on);
            });
        });
    }

    /// macOS permission manager — live status plus a jump to each Privacy pane.
    /// The wizard only guides the first grant; this is where a user fixes a
    /// permission they skipped or that got revoked.
    #[cfg(target_os = "macos")]
    fn permissions_card(&mut self, ui: &mut egui::Ui) {
        fn open_pane(anchor: &str) {
            let _ = std::process::Command::new("open")
                .arg(format!(
                    "x-apple.systempreferences:com.apple.preference.security?{anchor}"
                ))
                .status();
        }

        // `granted: None` for Microphone — there is no cheap way to read its TCC
        // state without triggering the prompt, and it is requested on first
        // capture anyway, so we show the pane link without a verdict.
        fn row(ui: &mut egui::Ui, label: &str, desc: &str, granted: Option<bool>, anchor: &str) {
            App::setting_row(ui, label, desc, |ui| {
                if ghost_button(ui, "Open").clicked() {
                    open_pane(anchor);
                }
                ui.add_space(8.0);
                match granted {
                    Some(true) => {
                        ui.label(
                            egui::RichText::new(format!("{} granted", icons::CHECK))
                                .small()
                                .color(theme::MINT),
                        );
                    }
                    Some(false) => {
                        ui.label(
                            egui::RichText::new("not granted")
                                .small()
                                .color(theme::AMBER),
                        );
                    }
                    None => {}
                }
            });
            ui.add_space(10.0);
        }

        let acc = wc_hotkey::keyboard_accessible();
        let inp = wc_hotkey::input_monitoring_granted();

        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            row(
                ui,
                "Accessibility",
                "Types transcribed text into the focused app",
                Some(acc),
                "Privacy_Accessibility",
            );
            row(
                ui,
                "Input Monitoring",
                "Notices the push-to-talk key globally",
                Some(inp),
                "Privacy_ListenEvent",
            );
            row(
                ui,
                "Microphone",
                "Captures speech while the key is held",
                None,
                "Privacy_Microphone",
            );
            ui.label(
                egui::RichText::new(
                    "macOS only re-reads these when an app starts. After enabling one, \
                     quit WhisprCatch (menu bar → Quit) and open it again.",
                )
                .small()
                .color(theme::MUTED),
            );
        });
    }

    fn about_card(&mut self, ui: &mut egui::Ui) {
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(crate::app_name())
                        .font(theme::medium(13.0))
                        .color(theme::FG),
                );
                ui.label(theme::mono_upper(
                    &format!("v{}", env!("CARGO_PKG_VERSION")),
                    10.5,
                    theme::MUTED,
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    ui.hyperlink_to(
                        egui::RichText::new(format!("{} Site", icons::GLOBE))
                            .small()
                            .color(theme::TEXT_2),
                        SITE_URL,
                    );
                    ui.hyperlink_to(
                        egui::RichText::new(format!("{} GitHub", icons::GITHUB_LOGO))
                            .small()
                            .color(theme::TEXT_2),
                        GITHUB_URL,
                    );
                });
            });
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Push-to-talk dictation that runs entirely on your machine.")
                    .small()
                    .color(theme::MUTED),
            );
            ui.add_space(4.0);
            // path stays lowercase — it's case-sensitive
            ui.label(
                egui::RichText::new(format!("config · {}", config::config_path().display()))
                    .font(egui::FontId::monospace(9.5))
                    .color(theme::MUTED),
            );
        });
    }
}
