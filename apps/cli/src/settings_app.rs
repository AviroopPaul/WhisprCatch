//! Settings & history window (eframe/egui), launched as
//! `whisper-catch settings [--tab history|settings]` — from the tray menu
//! or the shell.
//!
//! Layout per docs/DESIGN.md: top header with a centered segmented control;
//! History = 288px sidebar (search + chronological list) + a detail pane
//! with metadata and copy/delete; Settings = sections with small mono
//! uppercase headings. Dark-only, "tactile engineer" language.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use anyhow::Result;
use eframe::egui;
use egui_phosphor::regular as icons;
use wc_models::ModelId;
// Not re-exported from the crate root, unlike the six `*Config` types.
use wc_text::fillers::FillerLevel;

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

/// Opening size, unless `WC_WINDOW=1440x900` says otherwise.
///
/// Dev-only, like the other capture hooks in docs/DESIGN.md §B7 and reachable
/// no other way: this window is deliberately not resizable-by-memory, so a
/// capture that has to show the layout at another width has no way to ask for
/// one. The window manager still enforces the 720×480 minimum.
fn window_size() -> egui::Vec2 {
    std::env::var("WC_WINDOW")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once(['x', 'X'])?;
            Some(egui::vec2(w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
        .unwrap_or(egui::vec2(WINDOW_W, WINDOW_H))
}

/// Dev-only: `WC_SCROLL=<points>` opens the Settings tab already scrolled, so a
/// capture can show a section that sits below the fold. Same reasoning as
/// [`window_size`]; the window cannot be made tall enough to hold every section
/// on a laptop display.
fn shot_scroll() -> Option<f32> {
    std::env::var("WC_SCROLL").ok()?.trim().parse().ok()
}

/// The fixed sample transcripts behind `WC_DEMO_HISTORY`.
///
/// `(ts, dur_s, infer_s, text, raw)`. `raw` follows the real rule from
/// `history::Entry`: `Some` only where the polish chain changed something, so
/// most rows are `None` and their `text` *is* what the model said. The two that
/// carry a `raw` are what the cleanup preview replays in a capture; the rest
/// exercise the `raw: None` path, which is every entry a real user has today.
fn demo_rows() -> [(u64, f32, f32, &'static str, Option<&'static str>); 7] {
    // Fixed timestamps so consecutive captures are identical.
    let base = 1_760_000_000_u64;
    [
        (base, 11.2, 0.31,
         "Morning. The migration finished overnight and nothing looks broken, \
          but I'd like a second pair of eyes on the rollback path before we \
          call it done. Can you take a look this afternoon?",
         Some("Morning. The migration finished overnight and nothing looks \
               broken, um, but I'd like a second pair of eyes on the rollback \
               path before we call it done. Can you um take a look this \
               afternoon?")),
        (base - 900, 6.4, 0.19,
         "Push the release notes to the draft branch and I'll do a pass on \
          the wording tonight.",
         None),
        (base - 2_400, 18.7, 0.44,
         "Three things from standup. The flaky overlay test is quarantined, \
          the Wayland fallback landed behind a flag, and we still need someone \
          to own the notarisation ticket before the next tag.",
         Some("Three things from standup. The the flaky overlay test is \
               quarantined, the Wayland fallback landed behind a flag, and we \
               still need someone to own the notarisation ticket before the \
               next tag.")),
        (base - 5_100, 4.1, 0.12,
         "Reply to Priya: yes to Thursday, and I'll bring the latency numbers.",
         None),
        (base - 9_800, 26.3, 0.61,
         "Longer one. The reason inference feels instant is that the model is \
          already resident when the key goes down, so the only work left on \
          release is the tail of the audio. That's why the first word appears \
          before you've finished the sentence.",
         None),
        (base - 14_200, 9.8, 0.27,
         "Note to self: check whether the 300 millisecond pre-roll is still \
          enough on the older MacBook.",
         None),
        (base - 21_600, 3.2, 0.09, "", None),
    ]
}

/// Dev-only: `WC_DEMO_HISTORY=1` swaps the real transcript log for a fixed
/// sample set. Screenshots for the README and the website are taken with this
/// on, so nobody's actual dictation ends up published.
fn demo_history() -> Option<(Vec<history::Entry>, (u64, u64, f32))> {
    if std::env::var("WC_DEMO_HISTORY").is_err() {
        return None;
    }
    let entries: Vec<history::Entry> = demo_rows()
        .iter()
        .map(|(ts, dur_s, infer_s, text, raw)| history::Entry {
            ts: *ts,
            dur_s: *dur_s,
            infer_s: *infer_s,
            text: (*text).to_string(),
            raw: raw.map(str::to_string),
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
            .with_inner_size(window_size())
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
    /// Text cleanup: the problems in the user's own rule files, and their own
    /// dictations replayed through the current settings. Rebuilt only when
    /// `[polish]` changes; see [`Cleanup`].
    cleanup: Cleanup,
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
        let mut cfg = cfg;
        // A hand-edited `enabled = true` with `level = "off"` is a transform in
        // the chain that removes nothing: the daemon warns about it, the chain
        // readout lists it, and the panel can only honestly draw it as "Off".
        // Settle it once, at open, rather than showing one thing and meaning
        // another. Both states are byte-identical no-ops, so nothing is lost.
        if cfg.polish.fillers.enabled && cfg.polish.fillers.level == FillerLevel::Off {
            cfg.polish.fillers.enabled = false;
        }
        let cleanup = Cleanup::build(&cfg.polish, &entries);
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
            cleanup,
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
            self.cleanup = Cleanup::build(&self.cfg.polish, &self.entries);
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
        // The preview replays these entries, so it is stale the moment they
        // change. Config changes are caught by the fingerprint; this is the
        // other half.
        self.cleanup = Cleanup::build(&self.cfg.polish, &self.entries);
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

fn seg_button(ui: &mut egui::Ui, selected: bool, label: &str, min_w: f32) -> bool {
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
        .min_size(egui::vec2(min_w, 24.0));
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
                if seg_button(ui, *tab == Tab::History, "History", 92.0) {
                    *tab = Tab::History;
                }
                if seg_button(ui, *tab == Tab::Settings, "Settings", 92.0) {
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
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(window_size()));
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
        // Cheap struct compare; the rebuild behind it reads the user's rule
        // files off disk and replays their history, so it must not run per
        // frame. See `Cleanup`.
        if self.cleanup.fp != PolishFingerprint::of(&self.cfg.polish) {
            self.cleanup = Cleanup::build(&self.cfg.polish, &self.entries);
        }

        let mut area = egui::ScrollArea::vertical().auto_shrink(false);
        if let Some(offset) = shot_scroll() {
            area = area.vertical_scroll_offset(offset);
        }
        area.show(ui, |ui| {
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

            theme::section_label(ui, "Text cleanup");
            ui.add_space(6.0);
            self.cleanup_card(ui);
            ui.add_space(18.0);

            if self.cleanup.has_problems() {
                theme::section_label(ui, "Problems");
                ui.add_space(6.0);
                self.problems_card(ui);
                ui.add_space(18.0);
            }

            theme::section_label(ui, "Cleanup preview");
            ui.add_space(6.0);
            self.preview_card(ui);
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
            self.status =
                "Saved. Model, key and cleanup changes apply after the daemon restarts.".into();
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

// ------------------------------------------------------------ text cleanup

/// How many recent dictations the preview replays through the chain.
const PREVIEW_SCAN: usize = 40;
/// How many changed dictations it shows at once.
const PREVIEW_SHOW: usize = 3;
/// Unchanged words kept either side of a change in a shown diff.
const PREVIEW_CONTEXT: usize = 8;
/// Longest utterance the preview will diff. The diff is quadratic in words, and
/// something this long is not a readable preview anyway. It still counts toward
/// "would change", which only needs `apply`.
const PREVIEW_MAX_WORDS: usize = 400;

/// What happened to one word between the raw transcript and the polished one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Same,
    Removed,
    Added,
    /// Stands in for a run of unchanged words the preview dropped.
    Elided,
}

/// One dictation, replayed.
struct Sample {
    ts: u64,
    pieces: Vec<(Change, String)>,
}

/// What Settings can say about a rule file without opening an editor.
struct FileFacts {
    /// Where the transform will actually look: the config override if there is
    /// one, else the default location. `None` when the platform will not say
    /// where config lives.
    path: Option<PathBuf>,
    exists: bool,
    /// Entries parsed out of it, including any `validate` rejected.
    count: usize,
}

impl FileFacts {
    fn of(path: Option<PathBuf>, count: usize) -> Self {
        Self {
            exists: path.as_ref().is_some_and(|p| p.exists()),
            path,
            count,
        }
    }

    /// "~/.config/whisper-catch/dictionary.csv · 12 rules", or why there is
    /// nothing to count.
    fn readout(&self, one: &str, many: &str) -> String {
        match &self.path {
            None => "no config directory on this platform".to_string(),
            Some(p) if !self.exists => format!("{} · no file yet", tilde(p)),
            Some(p) => format!("{} · {}", tilde(p), plural(self.count, one, many)),
        }
    }
}

/// A path with the home directory written as `~`, which is both how people
/// write it and the difference between one line and three in a 560px column.
fn tilde(path: &std::path::Path) -> String {
    let home = dirs::home_dir();
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// "1 rule" / "12 rules". Both forms spelled out, because English does not
/// derive the ones this panel needs ("entry", "entries").
fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Everything in `[polish]`, flattened for comparison.
///
/// The panel needs to know when to rebuild the chain, and it cannot ask the
/// config: `PolishConfig` is not `PartialEq`, and adding that to `wc-text`
/// would not help anyway, because a rebuild also has to re-read the rule files.
/// So the panel keeps the config it last built from and compares. Cloning two
/// `Option<PathBuf>` per frame is nothing; `Cleanup::build` behind it reads two
/// files off disk and replays the user's history, and that must not run per
/// frame. An earlier surface called `Polish::from_config` straight from the
/// paint loop; this exists so this one does not.
#[derive(Debug, Clone, PartialEq)]
struct PolishFingerprint {
    dictionary: (bool, Option<PathBuf>),
    snippets: (bool, Option<PathBuf>),
    spoken: (bool, bool, bool),
    self_correct: bool,
    fillers: (bool, FillerLevel),
    numbers: bool,
}

impl PolishFingerprint {
    /// Every field of every transform's config. A field missing here is a
    /// control that changes nothing until something else is touched, which is
    /// what `fingerprint_notices_every_setting` guards.
    fn of(cfg: &wc_text::PolishConfig) -> Self {
        Self {
            dictionary: (cfg.dictionary.enabled, cfg.dictionary.path.clone()),
            snippets: (cfg.snippets.enabled, cfg.snippets.path.clone()),
            spoken: (
                cfg.spoken.enabled,
                cfg.spoken.structural,
                cfg.spoken.punctuation,
            ),
            self_correct: cfg.self_correct.enabled,
            fillers: (cfg.fillers.enabled, cfg.fillers.level),
            numbers: cfg.numbers.enabled,
        }
    }
}

/// The state behind the cleanup panel: the problems in the user's rule files,
/// and their own dictations replayed through the current settings.
struct Cleanup {
    /// The config this was built from. Compared per frame, rebuilt on a change.
    fp: PolishFingerprint,
    /// Enabled transforms, in the order they run.
    chain: Vec<&'static str>,
    /// `validate()` messages that mean an entry is switched off.
    faults: Vec<String>,
    /// `validate()` messages that mean the entry works and there is something
    /// to know. The `note:` marker is stripped.
    notes: Vec<String>,
    dictionary: FileFacts,
    snippets: FileFacts,
    /// Dictations replayed. Silent utterances are not counted: replaying
    /// nothing proves nothing.
    scanned: usize,
    /// How many of them the chain would change.
    changed: usize,
    /// The first few of those, diffed.
    samples: Vec<Sample>,
}

impl Cleanup {
    fn build(cfg: &wc_text::PolishConfig, entries: &[history::Entry]) -> Self {
        let polish = wc_text::Polish::from_config(cfg);
        let (faults, notes) = split_problems(cfg.validate());

        let mut scanned = 0;
        let mut changed = 0;
        let mut samples = Vec::new();
        for e in entries.iter().take(PREVIEW_SCAN) {
            let before = preview_input(e);
            if before.trim().is_empty() {
                continue;
            }
            scanned += 1;
            if polish.is_empty() {
                continue;
            }
            let after = polish.apply(before);
            if after == before {
                continue;
            }
            changed += 1;
            if samples.len() < PREVIEW_SHOW
                && before.split_whitespace().count() <= PREVIEW_MAX_WORDS
            {
                samples.push(Sample {
                    ts: e.ts,
                    pieces: trim_to_changes(&word_diff(before, &after), PREVIEW_CONTEXT),
                });
            }
        }

        Self {
            fp: PolishFingerprint::of(cfg),
            chain: polish.names(),
            faults,
            notes,
            dictionary: FileFacts::of(
                cfg.dictionary
                    .path
                    .clone()
                    .or_else(wc_text::Dictionary::default_path),
                wc_text::Dictionary::new(cfg.dictionary.clone()).rule_count(),
            ),
            snippets: FileFacts::of(
                cfg.snippets
                    .path
                    .clone()
                    .or_else(wc_text::snippets::default_path),
                wc_text::Snippets::new(cfg.snippets.clone())
                    .snippets()
                    .len(),
            ),
            scanned,
            changed,
            samples,
        }
    }

    fn has_problems(&self) -> bool {
        !self.faults.is_empty() || !self.notes.is_empty()
    }
}

/// What the model actually said, for one history entry.
///
/// `Entry.raw` holds the pre-polish text *only when a transform changed
/// something*, so on every history written before the cleanup stack shipped —
/// which today is every history any user has — it is `None` and `text` is
/// itself the raw model output. Reading `raw` alone would preview nothing at
/// all on exactly the histories real people have.
fn preview_input(entry: &history::Entry) -> &str {
    entry.raw.as_deref().unwrap_or(&entry.text)
}

/// Splits `PolishConfig::validate` by the severity convention the transforms
/// write into the message itself: a `note:` prefix means the entry still works
/// and there is something worth knowing, anything else means the entry is
/// switched off until it is fixed. `Vec<String>` is all the trait gives us, and
/// a list that mixes "this is off" with "this works, but read it" is worse than
/// useless to the person reading it.
///
/// Returns `(faults, notes)`, notes with the marker stripped.
fn split_problems(msgs: Vec<String>) -> (Vec<String>, Vec<String>) {
    let mut faults = Vec::new();
    let mut notes = Vec::new();
    for m in msgs {
        match m.strip_prefix("note:") {
            Some(rest) => notes.push(rest.trim_start().to_string()),
            None => faults.push(m),
        }
    }
    (faults, notes)
}

/// Splits text into diffable tokens: words, plus any whitespace run containing
/// a line break as a token of its own.
///
/// The break tokens are not pedantry. `spoken` synthesises `\n\n` and list
/// markers from dictated commands, and that is most of what it does; a preview
/// that flattened them would show the user nothing of the transform they just
/// switched on.
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut space = String::new();
    for c in text.trim().chars() {
        if c.is_whitespace() {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            space.push(c);
        } else {
            if space.contains('\n') {
                out.push(std::mem::take(&mut space));
            } else {
                space.clear();
            }
            word.push(c);
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// Word-level diff of the raw transcript against the polished one.
///
/// A plain longest-common-subsequence walk: the inputs are one utterance long,
/// and anything cleverer would be a second thing to get wrong.
fn word_diff(before: &str, after: &str) -> Vec<(Change, String)> {
    let a = tokens(before);
    let b = tokens(after);
    let (n, m) = (a.len(), b.len());
    // lcs[i][j] = length of the longest common subsequence of a[i..], b[j..]
    let mut lcs = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[at(i, j)] = if a[i] == b[j] {
                lcs[at(i + 1, j + 1)] + 1
            } else {
                lcs[at(i + 1, j)].max(lcs[at(i, j + 1)])
            };
        }
    }

    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((Change::Same, a[i].clone()));
            i += 1;
            j += 1;
        } else if lcs[at(i + 1, j)] >= lcs[at(i, j + 1)] {
            // Removals first on a tie, so a word the chain replaced reads as
            // "this went, that came" rather than the other way round.
            out.push((Change::Removed, a[i].clone()));
            i += 1;
        } else {
            out.push((Change::Added, b[j].clone()));
            j += 1;
        }
    }
    out.extend(a[i..].iter().map(|t| (Change::Removed, t.clone())));
    out.extend(b[j..].iter().map(|t| (Change::Added, t.clone())));
    out
}

/// Keeps `context` unchanged words either side of every change and replaces
/// each dropped run with one [`Change::Elided`] marker.
///
/// A 200-word dictation with one deleted "um" is a preview of nothing: the
/// change is somewhere in the wall of text. This is what makes it findable.
fn trim_to_changes(diff: &[(Change, String)], context: usize) -> Vec<(Change, String)> {
    let mut keep = vec![false; diff.len()];
    let mut any = false;
    for (i, (change, _)) in diff.iter().enumerate() {
        if *change == Change::Same {
            continue;
        }
        any = true;
        let from = i.saturating_sub(context);
        let to = (i + context).min(diff.len() - 1);
        keep[from..=to].fill(true);
    }
    if !any {
        return diff.to_vec();
    }

    let mut out = Vec::new();
    let mut dropped = false;
    for (i, piece) in diff.iter().enumerate() {
        if !keep[i] {
            dropped = true;
            continue;
        }
        if dropped {
            out.push((Change::Elided, "…".to_string()));
            dropped = false;
        }
        out.push(piece.clone());
    }
    if dropped {
        out.push((Change::Elided, "…".to_string()));
    }
    out
}

/// Human name for a filler level. A `match` rather than `as_str` so adding a
/// level is a compile error here, not a lowercase word in the UI.
fn filler_label(level: FillerLevel) -> &'static str {
    match level {
        FillerLevel::Off => "Off",
        FillerLevel::Light => "Light",
        FillerLevel::Medium => "Medium",
    }
}

/// Which level the picker draws as selected.
///
/// `enabled` and `level` are two config fields and the panel offers one
/// control, because their fourth combination is a lie: `enabled` with
/// `level = off` puts a transform in the chain that removes nothing. Nobody
/// means that, so the panel never draws it as anything but "Off".
fn shown_level(cfg: &wc_text::FillersConfig) -> FillerLevel {
    if cfg.enabled {
        cfg.level
    } else {
        FillerLevel::Off
    }
}

/// What picking a level in the panel does to config: a level switches the
/// transform on, `Off` switches it off. The level itself is left alone when
/// switching off, so turning filler removal back on returns the user to the
/// setting they had rather than to the weakest one.
fn pick_level(cfg: &mut wc_text::FillersConfig, level: FillerLevel) {
    cfg.enabled = level != FillerLevel::Off;
    if cfg.enabled {
        cfg.level = level;
    }
}

fn diff_format(change: Change, font: &egui::FontId) -> egui::TextFormat {
    let mut f = egui::TextFormat {
        font_id: font.clone(),
        color: theme::TEXT_2,
        ..Default::default()
    };
    match change {
        Change::Same => {}
        Change::Removed => {
            f.color = theme::RED;
            f.strikethrough = egui::Stroke::new(1.0, theme::RED);
        }
        Change::Added => f.color = theme::MINT,
        Change::Elided => f.color = theme::MUTED,
    }
    f
}

/// Paints one replayed dictation: removed words struck through in `RED`, added
/// words in `MINT`, everything else quiet. Newsreader, because this is the
/// user's own speech and not UI chrome.
fn diff_label(ui: &mut egui::Ui, pieces: &[(Change, String)]) {
    let font = theme::serif(15.0);
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    let mut first = true;
    let mut after_break = false;
    for (change, text) in pieces {
        let is_break = text.starts_with('\n');
        if !first && !is_break && !after_break {
            job.append(" ", 0.0, diff_format(Change::Same, &font));
        }
        job.append(text, 0.0, diff_format(*change, &font));
        first = false;
        after_break = is_break;
    }
    ui.add(egui::Label::new(job));
}

impl App {
    /// The panel itself: one row per transform that does something today, then
    /// the two that do not.
    fn cleanup_card(&mut self, ui: &mut egui::Ui) {
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 12.0;

            Self::setting_row(
                ui,
                "Custom dictionary",
                "Names, jargon and acronyms, spelled the way you write them",
                |ui| {
                    theme::toggle(ui, &mut self.cfg.polish.dictionary.enabled);
                },
            );
            Self::file_readout(ui, &self.cleanup.dictionary, "rule", "rules");

            Self::setting_row(
                ui,
                "Snippets",
                "Say a trigger, get the text you saved for it",
                |ui| {
                    theme::toggle(ui, &mut self.cfg.polish.snippets.enabled);
                },
            );
            Self::file_readout(ui, &self.cleanup.snippets, "entry", "entries");

            Self::setting_row(
                ui,
                "Spoken commands",
                "Turns \"new paragraph\" and \"bullet point\" into what they describe",
                |ui| {
                    theme::toggle(ui, &mut self.cfg.polish.spoken.enabled);
                },
            );
            if self.cfg.polish.spoken.enabled {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 10.0;
                        Self::setting_row(
                            ui,
                            "Structure",
                            "Paragraphs, line breaks, bullets, numbered lists",
                            |ui| {
                                theme::toggle(ui, &mut self.cfg.polish.spoken.structural);
                            },
                        );
                        Self::setting_row(
                            ui,
                            "Punctuation",
                            "\"comma\", \"period\", \"question mark\"",
                            |ui| {
                                theme::toggle(ui, &mut self.cfg.polish.spoken.punctuation);
                            },
                        );
                        // DESIGN.md and the transform's own docs both require
                        // this said out loud, and on its own line: a row
                        // description long enough to reach the toggle is drawn
                        // underneath it.
                        ui.label(
                            egui::RichText::new(
                                "Your model already punctuates. This overrides what it heard \
                                 with a literal word-for-character rule.",
                            )
                            .small()
                            .color(theme::MUTED),
                        );
                    });
                });
            }

            Self::setting_row(ui, "Filler words", "Hesitation sounds and stutters", |ui| {
                Self::level_picker(ui, &mut self.cfg.polish.fillers);
            });
            ui.label(
                egui::RichText::new(
                    "Light removes \"um\", \"uh\" and \"the the\". Hedges like \"you know\" are \
                     left alone: a comma is not proof that one is filler.",
                )
                .small()
                .color(theme::MUTED),
            );

            ui.add_space(2.0);
            ui.separator();
            ui.label(theme::mono_upper("not available yet", 10.0, theme::MUTED));
            Self::pending_row(
                ui,
                "Self-correction",
                "\"Tuesday, I mean Wednesday\" would keep only the correction",
                self.cfg.polish.self_correct.enabled,
            );
            Self::pending_row(
                ui,
                "Number formatting",
                "\"twenty five people\" would become \"25 people\"",
                self.cfg.polish.numbers.enabled,
            );

            if !self.cleanup.chain.is_empty() {
                ui.add_space(2.0);
                ui.label(theme::mono_upper(
                    &format!(
                        "runs {} · applies after the daemon restarts",
                        self.cleanup.chain.join(" then ")
                    ),
                    10.0,
                    theme::MUTED,
                ));
                if self.cfg.streaming {
                    // Words typed by a streaming pass are already on the user's
                    // screen and cannot be taken back, so cleanup only reaches
                    // what is typed on release. Saying so here beats letting
                    // them find out by dictating.
                    ui.label(
                        egui::RichText::new(format!(
                            "{} Live typing is on, so words already on screen stay as you said \
                             them. Cleanup only reaches what is typed when you release the key.",
                            icons::WARNING
                        ))
                        .small()
                        .color(theme::AMBER),
                    );
                }
            }
        });
    }

    /// Where a transform's rules live, and how many of them there are.
    fn file_readout(ui: &mut egui::Ui, facts: &FileFacts, one: &str, many: &str) {
        // path stays lowercase — it's case-sensitive
        ui.label(
            egui::RichText::new(facts.readout(one, many))
                .font(egui::FontId::monospace(9.5))
                .color(theme::MUTED),
        );
    }

    /// Filler level, from `FillerLevel::SELECTABLE` and never a list written
    /// here: `Medium` is gated pending #74, and hardcoding the levels is how
    /// that gate would quietly come back. When #74 opens it, this picker gains
    /// the level without an edit.
    fn level_picker(ui: &mut egui::Ui, cfg: &mut wc_text::FillersConfig) {
        let shown = shown_level(cfg);
        egui::Frame::default()
            .fill(theme::SURFACE_2)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(3.0)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                // The row this sits in lays out right to left and the plate
                // inherits that, so walk the levels in whichever direction the
                // parent set: a picker reading "Light | Off" is a different
                // control, and forcing a direction here stretches the plate
                // across the whole row instead of wrapping the buttons.
                let mut levels = FillerLevel::SELECTABLE;
                if ui.layout().main_dir() == egui::Direction::RightToLeft {
                    levels.reverse();
                }
                for level in levels {
                    if seg_button(ui, shown == level, filler_label(level), 56.0) {
                        pick_level(cfg, level);
                    }
                }
            });
    }

    /// A transform that is merged but still a no-op stub.
    ///
    /// Listed rather than hidden, for two reasons: a toggle that silently does
    /// nothing is the worst of the three options, and a `config.toml` that
    /// already switches one of these on has nowhere else to show up. When they
    /// land they become ordinary rows.
    fn pending_row(ui: &mut egui::Ui, label: &str, desc: &str, on_in_config: bool) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(egui::RichText::new(label).color(theme::TEXT_2));
                ui.label(egui::RichText::new(desc).small().color(theme::MUTED));
                if on_in_config {
                    ui.label(
                        egui::RichText::new("Switched on in config.toml, and still does nothing.")
                            .small()
                            .color(theme::AMBER),
                    );
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(theme::mono_upper("not yet", 10.0, theme::MUTED));
            });
        });
    }

    /// Everything the transforms found wrong with the user's own rule files.
    /// Every transform has written into `validate()` since the seam landed and
    /// nothing has ever shown it, which is why a malformed dictionary entry has
    /// been silently invisible.
    fn problems_card(&mut self, ui: &mut egui::Ui) {
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            // One wrapped paragraph rather than a horizontal row: these
            // messages carry a path and a line number and are long, and a
            // label in a horizontal layout does not wrap. It runs off the
            // right of the window instead, taking the card with it.
            let bullet = |ui: &mut egui::Ui, color: egui::Color32, msg: &str| {
                let font = egui::FontId::monospace(10.5);
                let mut job = egui::text::LayoutJob::default();
                job.wrap.max_width = ui.available_width();
                job.append(
                    "· ",
                    0.0,
                    egui::TextFormat {
                        font_id: font.clone(),
                        color,
                        ..Default::default()
                    },
                );
                job.append(
                    msg,
                    0.0,
                    egui::TextFormat {
                        font_id: font,
                        color: theme::TEXT_2,
                        ..Default::default()
                    },
                );
                ui.add(egui::Label::new(job));
            };

            if !self.cleanup.faults.is_empty() {
                ui.label(
                    egui::RichText::new(format!(
                        "{} {} switched off until fixed",
                        icons::WARNING,
                        plural(self.cleanup.faults.len(), "entry", "entries")
                    ))
                    .font(theme::medium(13.0))
                    .color(theme::RED),
                );
                ui.add_space(4.0);
                for msg in &self.cleanup.faults {
                    bullet(ui, theme::RED, msg);
                }
            }
            if !self.cleanup.notes.is_empty() {
                if !self.cleanup.faults.is_empty() {
                    ui.add_space(10.0);
                }
                ui.label(
                    egui::RichText::new(format!(
                        "{} {} on entries that still work",
                        icons::INFO,
                        plural(self.cleanup.notes.len(), "note", "notes")
                    ))
                    .font(theme::medium(13.0))
                    .color(theme::AMBER),
                );
                ui.add_space(4.0);
                for msg in &self.cleanup.notes {
                    bullet(ui, theme::AMBER, msg);
                }
            }
        });
    }

    /// The heart of the panel: the user's own dictations, replayed through the
    /// settings above. Not a canned example, and not a promise about what the
    /// daemon is doing right now, which is why the header says "would".
    fn preview_card(&mut self, ui: &mut egui::Ui) {
        let mut recheck = false;
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let head = if self.cleanup.chain.is_empty() {
                    "nothing enabled".to_string()
                } else if self.cleanup.scanned == 0 {
                    "nothing to replay".to_string()
                } else {
                    format!(
                        "{} of your last {} would change",
                        self.cleanup.changed,
                        plural(self.cleanup.scanned, "dictation", "dictations")
                    )
                };
                ui.label(theme::mono_upper(&head, 10.0, theme::MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ghost_button(
                        ui,
                        egui::RichText::new(format!("{} Recheck", icons::ARROWS_CLOCKWISE)),
                    )
                    .on_hover_text("Re-read your rule files and your latest dictations")
                    .clicked()
                    {
                        recheck = true;
                    }
                });
            });
            ui.add_space(10.0);

            let quiet = |ui: &mut egui::Ui, text: &str| {
                ui.label(egui::RichText::new(text).color(theme::TEXT_2));
            };
            let hint = |ui: &mut egui::Ui, text: &str| {
                ui.label(egui::RichText::new(text).small().color(theme::MUTED));
            };

            if self.cleanup.chain.is_empty() {
                quiet(ui, "Your words are typed exactly as the model heard them.");
                ui.add_space(2.0);
                hint(
                    ui,
                    "Switch something on above and this shows what it would do to your own \
                     dictations, before any of it reaches your keyboard.",
                );
            } else if self.cleanup.scanned == 0 {
                if self.cfg.history {
                    quiet(ui, "Nothing dictated yet.");
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(egui::RichText::new("Hold").small().color(theme::MUTED));
                        theme::key_chip(ui, self.key_label());
                        ui.label(
                            egui::RichText::new("and say something, then come back.")
                                .small()
                                .color(theme::MUTED),
                        );
                    });
                } else {
                    quiet(ui, "Keep history is off, so there is nothing of yours to replay.");
                    ui.add_space(2.0);
                    hint(
                        ui,
                        "Turn it on under Output behavior to preview cleanup on your own words. \
                         History never leaves this machine.",
                    );
                }
            } else if self.cleanup.changed == 0 {
                quiet(
                    ui,
                    "Every one of them comes out exactly as it went in. Nothing here would \
                     touch a word you have said so far.",
                );
            } else if self.cleanup.samples.is_empty() {
                // Everything that would change was too long to diff readably.
                quiet(
                    ui,
                    "The dictations this would change are all too long to show a useful \
                     diff of.",
                );
            } else {
                for (n, sample) in self.cleanup.samples.iter().enumerate() {
                    if n > 0 {
                        ui.add_space(12.0);
                        ui.separator();
                        ui.add_space(6.0);
                    }
                    ui.label(theme::mono_upper(&list_time(sample.ts), 10.0, theme::MUTED));
                    ui.add_space(6.0);
                    diff_label(ui, &sample.pieces);
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(
                        egui::RichText::new("removed")
                            .small()
                            .strikethrough()
                            .color(theme::RED),
                    );
                    ui.label(egui::RichText::new("·").small().color(theme::MUTED));
                    ui.label(egui::RichText::new("added").small().color(theme::MINT));
                    if self.cleanup.changed > self.cleanup.samples.len() {
                        ui.label(theme::mono_upper(
                            &format!(
                                "· {} more",
                                self.cleanup.changed - self.cleanup.samples.len()
                            ),
                            10.0,
                            theme::MUTED,
                        ));
                    }
                });
            }
        });
        if recheck {
            // Re-reads dictionary.csv and snippets.txt as well as the history,
            // so editing a rule file with this window open is one click away
            // from being visible.
            self.reload_history();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wc_text::{FillersConfig, PolishConfig, SpokenConfig};

    fn entry(ts: u64, text: &str, raw: Option<&str>) -> history::Entry {
        history::Entry {
            ts,
            dur_s: 1.0,
            infer_s: 0.1,
            text: text.into(),
            raw: raw.map(str::to_string),
        }
    }

    /// Filler removal, the one shipping transform that reads no files — so
    /// these tests do not depend on what is in the config dir of whoever runs
    /// them.
    fn light() -> PolishConfig {
        PolishConfig {
            fillers: FillersConfig {
                enabled: true,
                level: FillerLevel::Light,
            },
            ..Default::default()
        }
    }

    fn changes(pieces: &[(Change, String)]) -> Vec<(Change, &str)> {
        pieces
            .iter()
            .filter(|(c, _)| *c != Change::Same)
            .map(|(c, t)| (*c, t.as_str()))
            .collect()
    }

    // ---- which text the preview replays ----------------------------------

    /// The subtlety that decides whether the preview works at all on a real
    /// history: `raw` is written only when polish changed something, so on
    /// every entry any user has today it is `None` and `text` is itself the
    /// raw model output.
    #[test]
    fn an_unpolished_entry_replays_its_own_text() {
        assert_eq!(
            preview_input(&entry(1, "so um yeah", None)),
            "so um yeah",
            "with no raw stored, text IS the model output"
        );
    }

    #[test]
    fn a_polished_entry_replays_what_the_model_said() {
        assert_eq!(
            preview_input(&entry(1, "so yeah", Some("so um yeah"))),
            "so um yeah",
            "replaying the polished text would preview the wrong input"
        );
    }

    /// The regression this pair exists to stop: a history where nothing has
    /// ever been polished — which is every history in the wild — must still
    /// produce a preview.
    #[test]
    fn a_history_with_no_raw_anywhere_still_previews() {
        let entries = [entry(2, "so um yeah, it shipped", None)];
        let c = Cleanup::build(&light(), &entries);
        assert_eq!((c.scanned, c.changed), (1, 1));
        assert_eq!(changes(&c.samples[0].pieces), [(Change::Removed, "um")]);
    }

    // ---- validate(), and its two severities ------------------------------

    #[test]
    fn a_note_is_advisory_and_everything_else_switches_an_entry_off() {
        let (faults, notes) = split_problems(vec![
            "snippets.txt line 7: duplicate trigger".to_string(),
            "note: line 12: a multi-line body sends early in some apps".to_string(),
            "dictionary.csv line 3: empty pattern".to_string(),
        ]);
        assert_eq!(
            faults,
            [
                "snippets.txt line 7: duplicate trigger",
                "dictionary.csv line 3: empty pattern"
            ]
        );
        assert_eq!(
            notes,
            ["line 12: a multi-line body sends early in some apps"],
            "the marker is the severity, not part of the message"
        );
    }

    #[test]
    fn an_empty_validate_is_no_problems_at_all() {
        let (faults, notes) = split_problems(Vec::new());
        assert!(faults.is_empty() && notes.is_empty());
    }

    /// The marker is written by a `format!`, so a future transform could emit
    /// it without the space. Severity must not hang on that.
    #[test]
    fn the_note_marker_is_recognised_without_its_space() {
        let (faults, notes) = split_problems(vec!["note:tight".to_string()]);
        assert!(faults.is_empty());
        assert_eq!(notes, ["tight"]);
    }

    /// A message that merely mentions notes is a fault, not a note.
    #[test]
    fn only_a_leading_marker_counts() {
        let (faults, notes) =
            split_problems(vec!["line 4: take note: this is broken".to_string()]);
        assert_eq!(faults.len(), 1);
        assert!(notes.is_empty());
    }

    // ---- the diff --------------------------------------------------------

    #[test]
    fn a_deleted_word_is_the_only_thing_marked() {
        let d = word_diff("so um yeah", "so yeah");
        assert_eq!(changes(&d), [(Change::Removed, "um")]);
        assert_eq!(d.len(), 3);
    }

    #[test]
    fn a_substitution_reads_as_a_removal_then_an_addition() {
        let d = word_diff("ship it on get hub", "ship it on GitHub");
        assert_eq!(
            changes(&d),
            [
                (Change::Removed, "get"),
                (Change::Removed, "hub"),
                (Change::Added, "GitHub")
            ]
        );
    }

    /// Most of what spoken commands do is structure, and a diff that flattened
    /// whitespace would show the user none of it.
    #[test]
    fn a_new_paragraph_shows_up_as_a_change() {
        let d = word_diff("one new paragraph two", "one\n\ntwo");
        assert_eq!(
            changes(&d),
            [
                (Change::Removed, "new"),
                (Change::Removed, "paragraph"),
                (Change::Added, "\n\n")
            ]
        );
    }

    #[test]
    fn identical_text_has_nothing_to_show() {
        assert!(changes(&word_diff("nothing to do", "nothing to do")).is_empty());
    }

    #[test]
    fn trimming_keeps_context_and_says_where_it_cut() {
        let long: Vec<String> = (0..40).map(|i| format!("w{i}")).collect();
        let mut with_um = long.clone();
        with_um.insert(20, "um".into());
        let trimmed = trim_to_changes(&word_diff(&with_um.join(" "), &long.join(" ")), 3);
        let rendered: Vec<&str> = trimmed.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(
            rendered,
            ["…", "w17", "w18", "w19", "um", "w20", "w21", "w22", "…"]
        );
    }

    #[test]
    fn trimming_leaves_a_short_dictation_whole() {
        let d = word_diff("so um yeah", "so yeah");
        assert_eq!(trim_to_changes(&d, PREVIEW_CONTEXT), d);
    }

    #[test]
    fn trimming_an_unchanged_diff_changes_nothing() {
        let d = word_diff("a b c", "a b c");
        assert_eq!(trim_to_changes(&d, 1), d);
    }

    // ---- when to rebuild -------------------------------------------------

    /// The fingerprint is what keeps `Polish::from_config` off the paint loop,
    /// so a field missing from it is a control that appears to do nothing until
    /// something else is touched.
    #[test]
    fn the_fingerprint_notices_every_setting() {
        /// One `[polish]` field, and the name to blame if it goes missing.
        type Mutation = (&'static str, fn(&mut PolishConfig));
        let mutations: [Mutation; 9] = [
            ("dictionary.enabled", |c| c.dictionary.enabled = true),
            ("dictionary.path", |c| {
                c.dictionary.path = Some(PathBuf::from("/tmp/d.csv"))
            }),
            ("snippets.enabled", |c| c.snippets.enabled = true),
            ("snippets.path", |c| {
                c.snippets.path = Some(PathBuf::from("/tmp/s.txt"))
            }),
            ("spoken.enabled", |c| c.spoken.enabled = true),
            ("spoken.structural", |c| c.spoken.structural = false),
            ("spoken.punctuation", |c| c.spoken.punctuation = true),
            ("self_correct.enabled", |c| c.self_correct.enabled = true),
            ("numbers.enabled", |c| c.numbers.enabled = true),
        ];
        let base = PolishConfig::default();
        for (name, mutate) in mutations {
            let mut cfg = PolishConfig::default();
            mutate(&mut cfg);
            assert_ne!(
                PolishFingerprint::of(&base),
                PolishFingerprint::of(&cfg),
                "{name} does not reach the preview"
            );
        }
        // fillers takes both its fields at once, since the panel never sets
        // `enabled` without a level
        assert_ne!(
            PolishFingerprint::of(&base),
            PolishFingerprint::of(&light()),
            "fillers does not reach the preview"
        );
    }

    #[test]
    fn an_unchanged_config_does_not_ask_for_a_rebuild() {
        let cfg = light();
        assert_eq!(PolishFingerprint::of(&cfg), PolishFingerprint::of(&cfg));
    }

    // ---- the filler level control ----------------------------------------

    /// The gate three review rounds put in: `medium` is not selectable until
    /// #74. The picker renders `SELECTABLE` itself, so this is the only place
    /// that has to hold.
    #[test]
    fn medium_is_not_offered() {
        assert!(!FillerLevel::SELECTABLE.contains(&FillerLevel::Medium));
        assert_eq!(FillerLevel::SELECTABLE.len(), 2);
    }

    #[test]
    fn picking_a_level_switches_the_transform_on() {
        let mut cfg = FillersConfig::default();
        for level in FillerLevel::SELECTABLE {
            pick_level(&mut cfg, level);
            assert_eq!(shown_level(&cfg), level, "{level} did not stick");
            assert_eq!(
                cfg.enabled,
                level != FillerLevel::Off,
                "{level} left the chain in the wrong state"
            );
        }
    }

    /// Off then on again returns the user to the level they chose, not to the
    /// weakest one.
    #[test]
    fn switching_off_remembers_the_level() {
        let mut cfg = FillersConfig::default();
        pick_level(&mut cfg, FillerLevel::Light);
        pick_level(&mut cfg, FillerLevel::Off);
        assert_eq!(cfg.level, FillerLevel::Light);
        assert!(!cfg.enabled);
        assert_eq!(shown_level(&cfg), FillerLevel::Off);
    }

    #[test]
    fn every_selectable_level_has_a_human_label() {
        for level in FillerLevel::SELECTABLE {
            let label = filler_label(level);
            assert!(
                label.starts_with(|c: char| c.is_ascii_uppercase()),
                "{label:?} is not how a person writes it"
            );
        }
    }

    // ---- what the panel saves --------------------------------------------

    /// Everything the panel can write, through `config.toml` and back. Unknown
    /// `[polish.*]` keys are dropped on save by design, so what the panel
    /// itself writes had better survive.
    #[test]
    fn what_the_panel_writes_survives_a_save_and_a_load() {
        let mut cfg = config::Config::default();
        cfg.polish.dictionary.enabled = true;
        cfg.polish.snippets.enabled = true;
        cfg.polish.snippets.path = Some(PathBuf::from("/tmp/team/snippets.txt"));
        cfg.polish.spoken = SpokenConfig {
            enabled: true,
            structural: true,
            punctuation: true,
        };
        pick_level(&mut cfg.polish.fillers, FillerLevel::Light);

        let text = toml::to_string_pretty(&cfg).expect("Settings → Save must not fail");
        let back: config::Config = toml::from_str(&text).unwrap();

        assert_eq!(
            PolishFingerprint::of(&back.polish),
            PolishFingerprint::of(&cfg.polish)
        );
        assert_eq!(
            wc_text::Polish::from_config(&back.polish).names(),
            ["dictionary", "snippets", "spoken", "fillers"]
        );
        assert_eq!(back.polish.fillers.level, FillerLevel::Light);
        assert!(back.polish.spoken.punctuation);
        assert_eq!(
            back.polish.snippets.path,
            Some(PathBuf::from("/tmp/team/snippets.txt"))
        );
    }

    /// No user data in `config.toml`: the rules themselves live in their own
    /// files, and this panel must not be the thing that changes that.
    #[test]
    fn the_panel_writes_no_rules_into_config_toml() {
        let mut cfg = config::Config::default();
        cfg.polish.dictionary.enabled = true;
        cfg.polish.snippets.enabled = true;
        let text = toml::to_string_pretty(&cfg).unwrap();
        for key in ["pattern", "replacement", "trigger", "body"] {
            assert!(!text.contains(key), "{key} leaked into config.toml:\n{text}");
        }
    }

    // ---- replaying a history ---------------------------------------------

    #[test]
    fn an_empty_history_previews_nothing_rather_than_pretending() {
        let c = Cleanup::build(&light(), &[]);
        assert_eq!((c.scanned, c.changed), (0, 0));
        assert!(c.samples.is_empty());
    }

    #[test]
    fn nothing_enabled_counts_the_history_but_changes_none_of_it() {
        let entries = [entry(1, "so um yeah", None)];
        let c = Cleanup::build(&PolishConfig::default(), &entries);
        assert!(c.chain.is_empty());
        assert_eq!((c.scanned, c.changed), (1, 0));
    }

    /// A silent utterance is a row in History and proves nothing here.
    #[test]
    fn silent_utterances_are_not_replayed() {
        let entries = [entry(2, "", None), entry(1, "   ", None)];
        let c = Cleanup::build(&light(), &entries);
        assert_eq!(c.scanned, 0);
    }

    #[test]
    fn the_preview_shows_a_few_and_counts_the_rest() {
        let entries: Vec<history::Entry> = (0..PREVIEW_SHOW as u64 + 3)
            .map(|i| entry(i, "um so it shipped", None))
            .collect();
        let c = Cleanup::build(&light(), &entries);
        assert_eq!(c.changed, entries.len());
        assert_eq!(c.samples.len(), PREVIEW_SHOW);
    }

    #[test]
    fn only_the_most_recent_entries_are_replayed() {
        let entries: Vec<history::Entry> = (0..PREVIEW_SCAN as u64 + 10)
            .map(|i| entry(i, "um so it shipped", None))
            .collect();
        assert_eq!(Cleanup::build(&light(), &entries).scanned, PREVIEW_SCAN);
    }

    // ---- the capture fixture ---------------------------------------------

    /// `WC_DEMO_HISTORY` rows are what published screenshots show, so a row
    /// whose `raw` does not actually polish to the `text` beside it would put
    /// a preview of something that never happened on the README.
    #[test]
    fn every_demo_row_polishes_to_the_text_beside_it() {
        let polish = wc_text::Polish::from_config(&light());
        let mut checked = 0;
        for (_, _, _, text, raw) in demo_rows() {
            let Some(raw) = raw else { continue };
            assert_eq!(polish.apply(raw), text, "demo row {raw:?} is not honest");
            checked += 1;
        }
        assert!(checked > 0, "no demo row exercises the preview");
    }
}
