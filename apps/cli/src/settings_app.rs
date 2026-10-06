//! Settings & history window (eframe/egui), launched as
//! `whisper-catch settings [--tab home|history|cleanup|settings|permissions|about]`
//! from the tray menu or the shell.
//!
//! Layout per docs/DESIGN.md: a left nav rail (Home, History, Text cleanup,
//! Settings, Permissions, About) beside a content area. History is a list pane
//! beside a detail card; the other pages are a centered column of cards.
//! Dark-only, black + orange.

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

const SIDEBAR_W: f32 = 232.0;
/// Widest the centered page column grows.
const PAGE_COL: f32 = 720.0;
const GITHUB_URL: &str = "https://github.com/AviroopPaul/whisper-catch";
const SITE_URL: &str = "https://whisper-catch.vercel.app";

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Home,
    History,
    Cleanup,
    Settings,
    Permissions,
    About,
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
    /// Cached `permissions::all_granted()`, refreshed about once a second.
    perm_ok: bool,
    perm_checked: Instant,
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
            tab: match tab {
                Some("history") => Tab::History,
                Some("settings") => Tab::Settings,
                Some("permissions") => Tab::Permissions,
                Some("cleanup") | Some("text-cleanup") => Tab::Cleanup,
                Some("about") => Tab::About,
                _ => Tab::Home,
            },
            perm_ok: crate::permissions::all_granted(),
            perm_checked: Instant::now(),
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
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // macOS hands a resizable window back at whatever size it was last
        // seen, which for anyone who ran the old maximized build means a
        // 27-inch window of mostly background. Assert the designed size once,
        // then leave the window alone for the rest of the session.
        if self.needs_size {
            self.needs_size = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(window_size()));
            // Opened from the tray or the pill, this process starts behind
            // whatever app is in front; the window the user asked for should
            // not need a hunt.
            #[cfg(target_os = "macos")]
            crate::permissions::mac::bring_to_front(ctx);
        }
        self.shot.tick(ctx);
        self.poll_download();
        if self.dl.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        // Cheap struct compare; the rebuild behind it reads the user's rule
        // files off disk and replays their history, so it must not run per
        // frame. See `Cleanup`.
        if self.cleanup.fp != PolishFingerprint::of(&self.cfg.polish) {
            self.cleanup = Cleanup::build(&self.cfg.polish, &self.entries);
        }
        // Permissions can change while the window is open; re-read once a second.
        if self.perm_checked.elapsed() >= Duration::from_secs(1) {
            self.perm_ok = crate::permissions::all_granted();
            self.perm_checked = Instant::now();
        }
        ctx.request_repaint_after(Duration::from_secs(1));

        let nav = egui::SidePanel::left("nav")
            .exact_width(SIDEBAR_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::default()
                    .fill(theme::SIDEBAR)
                    .inner_margin(12.0),
            )
            .show(ctx, |ui| self.sidebar(ui));
        // 1px hairline on the right edge of the rail.
        let r = nav.response.rect;
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("nav-edge"),
        ))
        .vline(
            r.right() - 0.5,
            r.y_range(),
            egui::Stroke::new(1.0, theme::BORDER),
        );

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(theme::BG))
            .show(ctx, |ui| match self.tab {
                Tab::Home => self.scroll_col(ui, |s, ui| s.home_page(ui)),
                Tab::History => self.history_page(ui),
                Tab::Cleanup => {
                    self.save_footer(ui);
                    self.scroll_col(ui, |s, ui| s.cleanup_page(ui));
                }
                Tab::Settings => {
                    self.save_footer(ui);
                    self.scroll_col(ui, |s, ui| s.settings_page(ui));
                }
                Tab::Permissions => self.scroll_col(ui, |_, ui| {
                    theme::page_header(
                        ui,
                        "Permissions",
                        "WhisprCatch needs three macOS permissions to hear the hotkey, use the \
                         microphone and type for you.",
                    );
                    ui.add_space(20.0);
                    crate::permissions::panel(ui);
                }),
                Tab::About => self.scroll_col(ui, |s, ui| s.about_page(ui)),
            });
    }
}

// ------------------------------------------------------------------ shell

impl App {
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.add_space(4.0);
            theme::logo(ui, 28.0);
            ui.label(
                egui::RichText::new(crate::app_name())
                    .font(theme::semibold(15.0))
                    .color(theme::FG),
            );
        });
        ui.add_space(18.0);

        let items = [
            (Tab::Home, icons::HOUSE, "Home"),
            (Tab::History, icons::CLOCK_COUNTER_CLOCKWISE, "History"),
            (Tab::Cleanup, icons::MAGIC_WAND, "Text cleanup"),
            (Tab::Settings, icons::GEAR_SIX, "Settings"),
            (Tab::Permissions, icons::SHIELD_CHECK, "Permissions"),
            (Tab::About, icons::INFO, "About"),
        ];
        for (tab, icon, label) in items {
            let resp = theme::nav_item(ui, icon, label, self.tab == tab);
            if tab == Tab::Permissions && !self.perm_ok {
                let c = egui::pos2(resp.rect.right() - 16.0, resp.rect.center().y);
                ui.painter().circle_filled(c, 3.5, theme::AMBER);
            }
            if resp.clicked() {
                self.tab = tab;
            }
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            egui::Frame::default()
                .fill(theme::SURFACE)
                .stroke(egui::Stroke::new(1.0, theme::BORDER))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
                    // bottom-up layout: the last row added sits on top
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(egui::RichText::new("Hold").size(12.5).color(theme::MUTED));
                        theme::kbd(ui, self.key_label());
                        ui.label(
                            egui::RichText::new("to dictate")
                                .size(12.5)
                                .color(theme::MUTED),
                        );
                    });
                    ui.horizontal(|ui| {
                        let (led, state) = if self.perm_ok {
                            (theme::ACCENT, "Ready")
                        } else {
                            (theme::AMBER, "Setup needed")
                        };
                        theme::led(ui, led, false);
                        ui.label(
                            egui::RichText::new(state)
                                .font(theme::medium(13.0))
                                .color(theme::FG),
                        );
                    });
                });
        });
    }

    /// A vertically scrolling page with a centered column (max `PAGE_COL`).
    fn scroll_col(&mut self, ui: &mut egui::Ui, body: impl FnOnce(&mut Self, &mut egui::Ui)) {
        let mut area = egui::ScrollArea::vertical().auto_shrink(false);
        if let Some(offset) = shot_scroll() {
            area = area.vertical_scroll_offset(offset);
        }
        area.show(ui, |ui| {
            ui.add_space(32.0);
            let w = (ui.available_width() - 48.0).clamp(200.0, PAGE_COL);
            centered_col(ui, w, |ui| body(self, ui));
            ui.add_space(32.0);
        });
    }

    /// Sticky bottom bar on Settings and Text cleanup: status left, save right.
    fn save_footer(&mut self, ui: &mut egui::Ui) {
        egui::TopBottomPanel::bottom("save-footer")
            .exact_height(60.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::default()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(24, 0)),
            )
            .show_inside(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().hline(
                    r.x_range().expand(24.0),
                    r.top(),
                    egui::Stroke::new(1.0, theme::BORDER),
                );
                let w = ui.available_width().min(PAGE_COL);
                centered_col(ui, w, |ui| {
                    ui.set_min_height(60.0);
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if theme::primary_button(ui, "Save changes").clicked() {
                                self.save();
                            }
                            if !self.status.is_empty() {
                                let color = if self.saved_ok {
                                    theme::ACCENT
                                } else {
                                    theme::RED
                                };
                                let prefix =
                                    if self.saved_ok { icons::CHECK } else { icons::WARNING };
                                ui.with_layout(
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(format!(
                                                    "{prefix} {}",
                                                    self.status
                                                ))
                                                .size(12.5)
                                                .color(color),
                                            )
                                            .truncate(),
                                        );
                                    },
                                );
                            }
                        },
                    );
                });
            });
    }
}

// ------------------------------------------------------------------- home

impl App {
    fn home_page(&mut self, ui: &mut egui::Ui) {
        theme::page_header(
            ui,
            "Home",
            "Hold the key, speak, release. Your words appear where you are typing.",
        );
        ui.add_space(20.0);

        if !self.perm_ok {
            let mut review = false;
            theme::card(ui)
                .fill(theme::SURFACE)
                .stroke(egui::Stroke::new(1.0, theme::tint_strong(theme::AMBER)))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(icons::WARNING)
                                .size(18.0)
                                .color(theme::AMBER),
                        );
                        ui.vertical(|ui| {
                            ui.set_max_width((ui.available_width() - 190.0).max(160.0));
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(
                                egui::RichText::new("Finish setup")
                                    .font(theme::semibold(15.0))
                                    .color(theme::FG),
                            );
                            ui.label(
                                egui::RichText::new(
                                    "A macOS permission is missing, so dictation cannot work yet.",
                                )
                                .size(13.0)
                                .color(theme::TEXT_2),
                            );
                        });
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if theme::button(ui, theme::Variant::Primary, "Review permissions")
                                    .clicked()
                                {
                                    review = true;
                                }
                            },
                        );
                    });
                });
            if review {
                self.tab = Tab::Permissions;
            }
            ui.add_space(16.0);
        }

        // Stat tiles.
        let (n, words, secs) = self.totals;
        let mins = secs / 60.0;
        let mins_s = if mins < 10.0 {
            format!("{mins:.1}")
        } else {
            format!("{mins:.0}")
        };
        let gap = 12.0;
        let tile_w = ((ui.available_width() - gap * 2.0) / 3.0).floor();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (value, label) in [
                (words.to_string(), "Words dictated"),
                (n.to_string(), "Dictations"),
                (mins_s, "Minutes spoken"),
            ] {
                ui.allocate_ui_with_layout(
                    egui::vec2(tile_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        theme::card(ui).show(ui, |ui| {
                            ui.set_width(tile_w - 42.0);
                            ui.spacing_mut().item_spacing.y = 4.0;
                            ui.label(
                                egui::RichText::new(value)
                                    .font(theme::semibold(26.0))
                                    .color(theme::FG),
                            );
                            ui.label(egui::RichText::new(label).size(13.0).color(theme::MUTED));
                        });
                    },
                );
            }
        });
        ui.add_space(16.0);

        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_header(ui, "How it works", "");
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                theme::kbd(ui, self.key_label());
                ui.label(
                    egui::RichText::new("Hold, speak, release. Text appears at your cursor.")
                        .size(14.0)
                        .color(theme::FG),
                );
            });
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!(
                    "{}  Everything runs on this Mac. No audio leaves the device.",
                    icons::SHIELD_CHECK
                ))
                .size(13.0)
                .color(theme::TEXT_2),
            );
        });
        ui.add_space(16.0);

        let mut goto_history = false;
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Recent").font(theme::semibold(15.0)).color(theme::FG));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::button_with(
                        ui,
                        theme::Variant::Ghost,
                        Some(icons::ARROW_RIGHT),
                        "View all",
                        true,
                    )
                    .clicked()
                    {
                        goto_history = true;
                    }
                });
            });
            ui.add_space(8.0);
            if self.entries.is_empty() {
                ui.label(
                    egui::RichText::new("Nothing dictated yet.")
                        .size(13.5)
                        .color(theme::MUTED),
                );
            }
            for (i, e) in self.entries.iter().take(3).enumerate() {
                if i > 0 {
                    ui.add_space(2.0);
                    theme::hairline(ui);
                    ui.add_space(2.0);
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    ui.set_min_height(30.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(124.0, 18.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new(list_time(e.ts))
                                    .font(egui::FontId::monospace(11.5))
                                    .color(theme::MUTED),
                            );
                        },
                    );
                    let t = e.text.split_whitespace().collect::<Vec<_>>().join(" ");
                    let (t, color) = if t.is_empty() {
                        ("(nothing was said)".to_string(), theme::MUTED)
                    } else {
                        (t, theme::TEXT_2)
                    };
                    ui.add(
                        egui::Label::new(egui::RichText::new(t).size(13.5).color(color)).truncate(),
                    );
                });
            }
        });
        if goto_history {
            self.tab = Tab::History;
        }
    }
}

// ---------------------------------------------------------------- history

impl App {
    fn history_page(&mut self, ui: &mut egui::Ui) {
        egui::Frame::default()
            .inner_margin(egui::Margin {
                left: 24,
                right: 24,
                top: 32,
                bottom: 24,
            })
            .show(ui, |ui| {
                theme::page_header(
                    ui,
                    "History",
                    "Everything you dictate is saved here, on this Mac only.",
                );
                if self.entries.is_empty() {
                    self.history_empty_state(ui);
                    return;
                }
                ui.add_space(20.0);

                let list_w = (ui.available_width() * 0.4).clamp(190.0, 300.0);
                egui::SidePanel::left("history-list")
                    .exact_width(list_w)
                    .resizable(false)
                    .show_separator_line(false)
                    .frame(egui::Frame::default().inner_margin(egui::Margin {
                        left: 0,
                        right: 16,
                        top: 0,
                        bottom: 0,
                    }))
                    .show_inside(ui, |ui| self.history_list(ui));

                egui::CentralPanel::default()
                    .frame(egui::Frame::default())
                    .show_inside(ui, |ui| self.history_detail(ui));
            });
    }

    fn history_empty_state(&mut self, ui: &mut egui::Ui) {
        ui.add_space((ui.available_height() * 0.28).clamp(24.0, 220.0));
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
                theme::kbd(ui, self.key_label());
                ui.label(egui::RichText::new("and speak to dictate.").color(theme::TEXT_2));
            });
        });
    }

    fn history_list(&mut self, ui: &mut egui::Ui) {
        // Search input with a leading magnifier.
        egui::Frame::default()
            .fill(theme::SURFACE_2)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(egui::Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(
                        egui::RichText::new(icons::MAGNIFYING_GLASS)
                            .size(14.0)
                            .color(theme::MUTED),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .frame(false)
                            .hint_text(egui::RichText::new("Search").color(theme::MUTED))
                            .desired_width(f32::INFINITY),
                    );
                });
            });
        ui.add_space(10.0);

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

        let footer_h = 36.0;
        let list_h = (ui.available_height() - footer_h).max(60.0);
        egui::ScrollArea::vertical()
            .max_height(list_h)
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                if shown.is_empty() {
                    ui.add_space(16.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new("No matches")
                                .size(13.0)
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
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(theme::mono_upper(
                &format!("{} transcripts", shown.len()),
                10.0,
                theme::MUTED,
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.confirm_clear {
                    if theme::button_with(ui, theme::Variant::Destructive, None, "Delete all", true)
                        .clicked()
                    {
                        let _ = history::clear();
                        self.reload_history();
                        self.confirm_clear = false;
                    }
                    if theme::button_with(ui, theme::Variant::Ghost, None, "Keep", true).clicked() {
                        self.confirm_clear = false;
                    }
                } else if theme::button_with(ui, theme::Variant::Ghost, None, "Clear all", true)
                    .clicked()
                {
                    self.confirm_clear = true;
                }
            });
        });
    }

    /// One list row: mono timestamp + duration, then a 2-line clamped
    /// preview. Returns true when clicked.
    fn history_row(&self, ui: &mut egui::Ui, ts: u64, text: &str, dur: f32) -> bool {
        let selected = Some(ts) == self.selected;
        let resp = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .id_salt(ts)
                    .sense(egui::Sense::click()),
                |ui| {
                    let hovered = ui.rect_contains_pointer(ui.max_rect());
                    let fill = if selected {
                        theme::SURFACE_2
                    } else if hovered {
                        theme::SURFACE
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    egui::Frame::default()
                        .fill(fill)
                        .corner_radius(egui::CornerRadius::same(8))
                        .inner_margin(egui::Margin {
                            left: 14,
                            right: 12,
                            top: 10,
                            bottom: 10,
                        })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 4.0;
                            ui.spacing_mut().interact_size.y = 16.0;
                            ui.horizontal(|ui| {
                                ui.label(theme::mono_upper(
                                    &list_time(ts),
                                    10.5,
                                    if selected { theme::TEXT_2 } else { theme::MUTED },
                                ));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(theme::mono_upper(
                                            &format!("{dur:.1}s"),
                                            10.5,
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
                                    font_id: egui::FontId::proportional(13.0),
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
                egui::CornerRadius::same(8),
                egui::Stroke::new(1.0, theme::BORDER),
                egui::StrokeKind::Inside,
            );
            let r = resp.rect;
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(r.left(), r.top() + 8.0),
                    egui::vec2(2.0, r.height() - 16.0),
                ),
                egui::CornerRadius::same(1),
                theme::ACCENT,
            );
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    }

    /// Copy and Delete (with inline confirm), in reading order. When `rtl` the
    /// row lays out right to left, so the buttons are added in reverse.
    fn detail_actions(
        &mut self,
        ui: &mut egui::Ui,
        ts: u64,
        rtl: bool,
        copy: &mut bool,
        delete: &mut bool,
    ) {
        ui.spacing_mut().item_spacing.x = 8.0;
        let flash = self
            .copied
            .is_some_and(|at| at.elapsed() < Duration::from_millis(1500));
        let confirming = self.confirm_delete == Some(ts);

        // 0 = copy, 1 = delete group
        let order = if rtl { [1, 0] } else { [0, 1] };
        for part in order {
            if part == 0 {
                if flash {
                    theme::button_with(
                        ui,
                        theme::Variant::Outline,
                        Some(icons::CHECK),
                        "Copied",
                        true,
                    );
                    ui.ctx().request_repaint_after(Duration::from_millis(200));
                } else if theme::button_with(
                    ui,
                    theme::Variant::Outline,
                    Some(icons::COPY),
                    "Copy",
                    true,
                )
                .clicked()
                {
                    *copy = true;
                }
            } else if confirming {
                let cancel = |s: &mut Self, ui: &mut egui::Ui| {
                    if theme::button_with(ui, theme::Variant::Ghost, None, "Cancel", true)
                        .clicked()
                    {
                        s.confirm_delete = None;
                    }
                };
                let confirm = |ui: &mut egui::Ui, delete: &mut bool| {
                    if theme::button_with(
                        ui,
                        theme::Variant::Destructive,
                        Some(icons::TRASH),
                        "Confirm delete",
                        true,
                    )
                    .clicked()
                    {
                        *delete = true;
                    }
                };
                if rtl {
                    cancel(self, ui);
                    confirm(ui, delete);
                } else {
                    confirm(ui, delete);
                    cancel(self, ui);
                }
            } else if theme::button_with(
                ui,
                theme::Variant::Ghost,
                Some(icons::TRASH),
                "Delete",
                true,
            )
            .clicked()
            {
                self.confirm_delete = Some(ts);
            }
        }
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
        let h = ui.available_height();

        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(h - 44.0);
            ui.spacing_mut().interact_size.y = 20.0;
            let wide = ui.available_width() > 440.0;
            ui.horizontal(|ui| {
                ui.label(theme::mono_upper(
                    &detail_time(entry.ts),
                    11.0,
                    theme::MUTED,
                ));
                if wide {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.detail_actions(ui, entry.ts, true, &mut do_copy, &mut do_delete);
                    });
                }
            });
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                theme::badge(ui, &format!("{:.1}s spoken", entry.dur_s), theme::Tone::Neutral);
                theme::badge(ui, &format!("{words} words"), theme::Tone::Neutral);
                theme::badge(
                    ui,
                    &format!("{:.2}s inference", entry.infer_s),
                    theme::Tone::Neutral,
                );
            });
            if !wide {
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    self.detail_actions(ui, entry.ts, false, &mut do_copy, &mut do_delete)
                });
            }
            ui.add_space(14.0);
            theme::hairline(ui);
            ui.add_space(14.0);

            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    if entry.text.trim().is_empty() {
                        ui.label(
                            egui::RichText::new("Nothing was said in this one.")
                                .size(15.0)
                                .italics()
                                .color(theme::MUTED),
                        );
                        return;
                    }
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&entry.text)
                                .size(15.0)
                                .line_height(Some(22.5))
                                .color(theme::FG),
                        )
                        .wrap(),
                    );
                });
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

/// 1px rule between rows inside a card, with 14px of air on each side.
fn row_sep(ui: &mut egui::Ui) {
    ui.add_space(14.0);
    theme::hairline(ui);
    ui.add_space(14.0);
}

impl App {
    fn settings_page(&mut self, ui: &mut egui::Ui) {
        theme::page_header(
            ui,
            "Settings",
            "Everything runs on this Mac. Changes apply after the daemon restarts.",
        );
        ui.add_space(20.0);
        self.engine_card(ui);
        ui.add_space(16.0);
        self.hotkey_card(ui);
        ui.add_space(16.0);
        self.output_card(ui);
    }

    fn cleanup_page(&mut self, ui: &mut egui::Ui) {
        theme::page_header(
            ui,
            "Text cleanup",
            "Tidy what the model heard before it is typed. Nothing leaves this Mac.",
        );
        ui.add_space(20.0);
        self.cleanup_card(ui);
        ui.add_space(16.0);
        if self.cleanup.has_problems() {
            self.problems_card(ui);
            ui.add_space(16.0);
        }
        self.preview_card(ui);
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
        let full = ui.available_width();
        let left_w = (full - 230.0).max(full * 0.5);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_max_width(left_w);
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(
                    egui::RichText::new(label)
                        .font(theme::medium(14.0))
                        .color(theme::FG),
                );
                if !desc.is_empty() {
                    ui.label(egui::RichText::new(desc).size(12.5).color(theme::MUTED));
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
            theme::card_header(
                ui,
                "Engine",
                "The speech model that turns your voice into text, on this Mac.",
            );
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(16.0);
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
            row_sep(ui);

            let readout = format!("{} · {} MB download", selected.ram_hint(), selected.download_mb());
            if downloading {
                let dl = self.dl.as_ref().unwrap();
                let frac = if dl.total > 0 {
                    dl.done as f32 / dl.total as f32
                } else {
                    0.0
                };
                Self::setting_row(ui, "Status", &readout, |ui| {
                    theme::badge(ui, "Downloading", theme::Tone::Neutral);
                });
                ui.add_space(12.0);
                theme::progress(ui, frac);
                ui.add_space(8.0);
                if let Some(e) = &dl.error {
                    ui.label(egui::RichText::new(e).size(12.5).color(theme::RED));
                } else {
                    ui.label(
                        egui::RichText::new(format!(
                            "{:.0}% · {:.0} / {:.0} MB · {}",
                            frac * 100.0,
                            dl.done as f64 / 1e6,
                            dl.total as f64 / 1e6,
                            if dl.file.is_empty() { "preparing" } else { &dl.file }
                        ))
                        .font(egui::FontId::monospace(11.5))
                        .color(theme::MUTED),
                    );
                }
            } else if complete {
                Self::setting_row(ui, "Status", &readout, |ui| {
                    theme::badge(ui, "Ready", theme::Tone::Accent);
                });
            } else {
                Self::setting_row(ui, "Status", &readout, |ui| {
                    if theme::button_with(
                        ui,
                        theme::Variant::Outline,
                        Some(icons::DOWNLOAD_SIMPLE),
                        &format!("Download ({} MB)", selected.download_mb()),
                        true,
                    )
                    .clicked()
                    {
                        do_download = Some(selected);
                    }
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
            theme::card_header(ui, "Hotkey", "The key you hold to dictate.");
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(16.0);
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
            ui.add_space(14.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(egui::RichText::new("Hold").size(13.0).color(theme::MUTED));
                theme::kbd(ui, config::key_label(&self.cfg.key));
                ui.label(
                    egui::RichText::new("and speak. Release to type.")
                        .size(13.0)
                        .color(theme::MUTED),
                );
            });
            crate::permissions::fn_key_notice(ui, &self.cfg.key);
        });
    }

    fn output_card(&mut self, ui: &mut egui::Ui) {
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_header(ui, "Output", "How your words reach the screen.");
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(16.0);
            Self::setting_row(ui, "Live typing", "Words appear while you speak", |ui| {
                theme::toggle(ui, &mut self.cfg.streaming);
            });
            row_sep(ui);
            Self::setting_row(
                ui,
                "Recording indicator",
                "Floating pill while dictating",
                |ui| {
                    theme::toggle(ui, &mut self.cfg.overlay);
                },
            );
            row_sep(ui);
            Self::setting_row(ui, "Keep history", "Log transcriptions locally", |ui| {
                theme::toggle(ui, &mut self.cfg.history);
            });
            row_sep(ui);
            Self::setting_row(ui, "Start on login", "Launch the daemon with your session", |ui| {
                theme::toggle(ui, &mut self.autostart_on);
            });
        });
    }

    fn about_page(&mut self, ui: &mut egui::Ui) {
        theme::page_header(ui, "About", "");
        ui.add_space(20.0);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                theme::logo(ui, 44.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(
                            egui::RichText::new(crate::app_name())
                                .font(theme::semibold(18.0))
                                .color(theme::FG),
                        );
                        theme::badge(
                            ui,
                            &format!("v{}", env!("CARGO_PKG_VERSION")),
                            theme::Tone::Neutral,
                        );
                    });
                    ui.label(
                        egui::RichText::new(
                            "Push-to-talk dictation that runs entirely on your machine.",
                        )
                        .size(13.0)
                        .color(theme::TEXT_2),
                    );
                });
            });
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if theme::button_with(
                    ui,
                    theme::Variant::Outline,
                    Some(icons::GITHUB_LOGO),
                    "GitHub",
                    false,
                )
                .clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(GITHUB_URL));
                }
                if theme::button_with(ui, theme::Variant::Outline, Some(icons::GLOBE), "Website", false)
                    .clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(SITE_URL));
                }
            });
        });
        ui.add_space(16.0);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_header(ui, "Privacy", "");
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(
                    "No account, no telemetry, no cloud. Audio and text never leave this machine.",
                )
                .size(13.5)
                .color(theme::TEXT_2),
            );
            ui.add_space(14.0);
            theme::section_label(ui, "Config file");
            ui.add_space(2.0);
            // path stays lowercase: it is case-sensitive
            ui.label(
                egui::RichText::new(config::config_path().display().to_string())
                    .font(egui::FontId::monospace(11.5))
                    .color(theme::MUTED),
            );
        });
    }
}

// ------------------------------------------------------------ text cleanup

/// Transforms that are wired into the chain but are still no-op stubs, named by
/// `Transform::name()`. They are listed in the panel under "not available yet"
/// so a `config.toml` that already enables one is visible — but they must never
/// appear in the chain readout or gate the live-typing caveat, or the panel
/// tells the user a cleanup is running that is not. Delete a name here when its
/// transform lands: #48 -> self_correct, #46 -> numbers.
const PENDING: [&str; 2] = ["self_correct", "numbers"];

/// How many recent dictations the preview replays through the chain.
const PREVIEW_SCAN: usize = 40;
/// How many changed dictations it shows at once.
const PREVIEW_SHOW: usize = 3;
/// Unchanged words kept either side of a change in a shown diff.
const PREVIEW_CONTEXT: usize = 8;
/// Longest utterance the preview will diff, counted on *both* sides. The diff
/// is quadratic in words, and while an utterance is bounded by how long a
/// person can hold a key, what a snippet expands it into is bounded by nothing
/// at all. Something this long is not a readable preview anyway; it still
/// counts toward "would change", which only needs `apply`.
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
                && after.split_whitespace().count() <= PREVIEW_MAX_WORDS
            {
                samples.push(Sample {
                    ts: e.ts,
                    pieces: trim_to_changes(&word_diff(before, &after), PREVIEW_CONTEXT),
                });
            }
        }

        Self {
            fp: PolishFingerprint::of(cfg),
            // Not `polish.names()`: that includes the transforms that are still
            // no-op stubs, so a config with `[polish.numbers] enabled = true`
            // rendered "RUNS NUMBERS …" and the live-typing caveat three lines
            // under a row saying it does nothing. Naming a stub in the chain is
            // the one way this panel can make a user believe a stub is working.
            chain: polish
                .names()
                .into_iter()
                .filter(|n| !PENDING.contains(n))
                .collect(),
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
        Change::Added => f.color = theme::ACCENT,
        Change::Elided => f.color = theme::MUTED,
    }
    f
}

/// Paints one replayed dictation: removed words struck through in `RED`, added
/// words in `ACCENT`, everything else quiet.
fn diff_label(ui: &mut egui::Ui, pieces: &[(Change, String)]) {
    let font = egui::FontId::proportional(15.0);
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
            theme::card_header(
                ui,
                "Cleanup rules",
                "Each rule runs on your words, in this order, before they are typed.",
            );
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.add_space(10.0);

            Self::setting_row(
                ui,
                "Custom dictionary",
                "Names, jargon and acronyms, spelled the way you write them",
                |ui| {
                    theme::toggle(ui, &mut self.cfg.polish.dictionary.enabled);
                },
            );
            Self::file_readout(ui, &self.cleanup.dictionary, "rule", "rules");
            row_sep(ui);

            Self::setting_row(
                ui,
                "Snippets",
                "Say a trigger, get the text you saved for it",
                |ui| {
                    theme::toggle(ui, &mut self.cfg.polish.snippets.enabled);
                },
            );
            Self::file_readout(ui, &self.cleanup.snippets, "entry", "entries");
            row_sep(ui);

            Self::setting_row(
                ui,
                "Spoken commands",
                "Turns \"new paragraph\" and \"bullet point\" into what they describe",
                |ui| {
                    theme::toggle(ui, &mut self.cfg.polish.spoken.enabled);
                },
            );
            if self.cfg.polish.spoken.enabled {
                ui.add_space(12.0);
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

            row_sep(ui);
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

            row_sep(ui);
            theme::section_label(ui, "Not available yet");
            ui.add_space(8.0);
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
                row_sep(ui);
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
            theme::card_header(ui, "Problems", "Found in your own rule files.");
            ui.add_space(10.0);
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
            theme::card_header(
                ui,
                "Cleanup preview",
                "Your own recent dictations, replayed through the rules above.",
            );
            ui.add_space(10.0);
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
                    if theme::button_with(
                        ui,
                        theme::Variant::Outline,
                        Some(icons::ARROWS_CLOCKWISE),
                        "Recheck",
                        true,
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
                        theme::kbd(ui, self.key_label());
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
                    ui.label(egui::RichText::new("added").small().color(theme::ACCENT));
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
        // fillers needs its two fields moved *separately*. Comparing default
        // against light() moves both at once, which passes even if the
        // fingerprint ignores one of them — and `level` alone is exactly what
        // #74 will move when Light -> Medium becomes selectable. A fingerprint
        // blind to it would leave the preview showing a stale answer.
        let on_off = FillersConfig {
            enabled: true,
            level: FillerLevel::Off,
        };
        let on_light = FillersConfig {
            enabled: true,
            level: FillerLevel::Light,
        };
        let off_light = FillersConfig {
            enabled: false,
            level: FillerLevel::Light,
        };
        let with = |f: &FillersConfig| PolishConfig {
            fillers: f.clone(),
            ..Default::default()
        };
        assert_ne!(
            PolishFingerprint::of(&with(&on_off)),
            PolishFingerprint::of(&with(&on_light)),
            "fillers.level does not reach the preview"
        );
        assert_ne!(
            PolishFingerprint::of(&with(&off_light)),
            PolishFingerprint::of(&with(&on_light)),
            "fillers.enabled does not reach the preview"
        );
        assert_ne!(
            PolishFingerprint::of(&base),
            PolishFingerprint::of(&light()),
            "fillers does not reach the preview"
        );
    }

    /// The chain readout and the live-typing caveat must never name a transform
    /// that is still a no-op. A `config.toml` enabling one used to render
    /// "RUNS NUMBERS ..." three lines under a row saying it does nothing.
    #[test]
    fn a_pending_transform_never_reaches_the_chain_readout() {
        let cfg = PolishConfig {
            self_correct: wc_text::SelfCorrectConfig { enabled: true },
            numbers: wc_text::NumbersConfig { enabled: true },
            ..Default::default()
        };
        let c = Cleanup::build(&cfg, &[entry(1, "twenty five people", None)]);
        assert!(
            c.chain.is_empty(),
            "a stub reached the chain readout: {:?}",
            c.chain
        );
        assert_eq!(c.changed, 0, "a stub cannot change a dictation");
    }

    /// The count says "would change", so an entry the chain leaves alone must
    /// not be counted or shown. Nothing covered this: the other tests use an
    /// empty chain or empty text, both of which short-circuit earlier.
    #[test]
    fn a_dictation_the_chain_leaves_alone_is_scanned_but_not_changed() {
        let entries = [entry(1, "nothing here needs removing", None)];
        let c = Cleanup::build(&light(), &entries);
        assert!(!c.chain.is_empty(), "the chain must actually be running");
        assert_eq!((c.scanned, c.changed), (1, 0));
        assert!(c.samples.is_empty(), "an unchanged entry has nothing to show");
    }

    /// A very long dictation is still counted, but diffing it would flood the
    /// panel — so it is bounded on both sides.
    #[test]
    fn a_very_long_dictation_is_counted_but_not_diffed() {
        let long = std::iter::repeat_n("um word", PREVIEW_MAX_WORDS)
            .collect::<Vec<_>>()
            .join(" ");
        let c = Cleanup::build(&light(), &[entry(1, &long, None)]);
        assert_eq!(c.changed, 1, "it still counts");
        assert!(c.samples.is_empty(), "but it is too long to diff");
    }

    /// The picker must read `wc-text`'s list, not its own copy — `Medium` is
    /// deliberately unreachable pending #74, and a literal here would resurrect
    /// it silently. Asserting on `SELECTABLE` alone cannot catch that, since
    /// `wc-text` already tests its own constant.
    #[test]
    fn the_level_picker_offers_exactly_what_wc_text_allows() {
        let offered = FillerLevel::SELECTABLE.to_vec();
        assert_eq!(offered, vec![FillerLevel::Off, FillerLevel::Light]);
        assert!(
            !offered.contains(&FillerLevel::Medium),
            "Medium is gated behind #74"
        );
        // The picker iterates SELECTABLE directly; if it ever grows its own
        // list this count is the thing that will disagree.
        assert_eq!(
            offered.len(),
            FillerLevel::SELECTABLE.len(),
            "the picker and wc-text must offer the same levels"
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
