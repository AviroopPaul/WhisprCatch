//! `whisper-catch note [--id <id>]`: a small always-on-top note taker, so
//! held-fn dictation can type straight into it. One window at a time, with a
//! tab per note opened in it, a collapsible sidebar of recent notes, and a
//! Copy button. Drawn by hand: no system title bar, our own rounded body.

use std::time::{Duration, Instant};

use anyhow::Result;
use eframe::egui;
use egui_phosphor::regular as icons;

use crate::notes::{valid_id, NoteMeta, Store};
use crate::notes_ui::Editor;
use crate::theme;

const COMPACT: egui::Vec2 = egui::vec2(530.0, 430.0);
const LARGE: egui::Vec2 = egui::vec2(860.0, 620.0);
const RADIUS: f32 = 16.0;
const BAR_H: f32 = 48.0;
const INSET: f32 = 8.0;
const SIDE_W: f32 = 170.0;
const RAIL_W: f32 = 52.0;
const ROW_H: f32 = 38.0;
const COPIED_FOR: Duration = Duration::from_millis(1500);
const RELOAD_EVERY: Duration = Duration::from_millis(1500);

pub fn run(id: Option<String>) -> Result<()> {
    if let Some(id) = &id {
        anyhow::ensure!(valid_id(id), "not a note id: {id}");
    }
    // Single instance. The lock dies with the process. Captures skip it.
    let _lock = if std::env::var_os("WC_SHOT").is_some() {
        None
    } else {
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(std::env::temp_dir().join("whisper-catch-note.lock"))?;
        if fs2::FileExt::try_lock_exclusive(&f).is_err() {
            log::info!("a note window is already open");
            return Ok(());
        }
        Some(f)
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(COMPACT)
            .with_min_inner_size([400.0, 300.0])
            .with_title("Note")
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top(),
        centered: true,
        persist_window: false,
        ..Default::default()
    };
    let key = crate::config::load()
        .map(|c| crate::config::key_label(&c.key).to_string())
        .unwrap_or_else(|_| "fn".into());
    eframe::run_native(
        "Note",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            theme::install_fonts(&cc.egui_ctx);
            let store = Store::default();
            let mut tabs = vec![match id {
                Some(id) => Editor::open(store.clone(), &id),
                None => Editor::new_note(store.clone()),
            }];
            // Dev-only capture hooks: `WC_NOTE_TABS=id,id` opens more tabs,
            // `WC_NOTE_COLLAPSED=1` starts with the sidebar collapsed.
            if let Ok(ids) = std::env::var("WC_NOTE_TABS") {
                for id in ids.split(',').filter(|i| valid_id(i)) {
                    tabs.push(Editor::open(store.clone(), id));
                }
            }
            if std::env::var_os("WC_NOTE_COLLAPSED").is_some() {
                set_collapsed(&cc.egui_ctx, true);
            }
            let list = store.list();
            Ok(Box::new(NoteApp {
                store,
                tabs,
                active: 0,
                list,
                loaded: Instant::now(),
                search: String::new(),
                key,
                large: false,
                copied: None,
                first: true,
                shot: crate::shot::Shot::from_env(),
            }) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| anyhow::anyhow!("note window failed: {e}"))
}

/// The sidebar's collapsed state lives in egui memory: this session only.
fn collapsed(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::new("note-collapsed")))
        .unwrap_or(false)
}

fn set_collapsed(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("note-collapsed"), on));
}

struct NoteApp {
    store: Store,
    tabs: Vec<Editor>,
    active: usize,
    list: Vec<NoteMeta>,
    loaded: Instant,
    search: String,
    key: String,
    large: bool,
    copied: Option<Instant>,
    first: bool,
    shot: crate::shot::Shot,
}

impl eframe::App for NoteApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.first {
            self.first = false;
            #[cfg(target_os = "macos")]
            crate::permissions::mac::bring_to_front(ctx);
        }
        self.shot.tick(ctx);
        for t in &mut self.tabs {
            t.tick(ctx);
        }
        if self.loaded.elapsed() >= RELOAD_EVERY {
            self.reload();
        }
        ctx.request_repaint_after(RELOAD_EVERY);
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let full = ui.max_rect();
                ui.painter().rect_filled(full, RADIUS, theme::BG);
                ui.painter().rect_stroke(
                    full.shrink(0.5),
                    RADIUS,
                    egui::Stroke::new(1.0, theme::BORDER),
                    egui::StrokeKind::Inside,
                );
                let bar = egui::Rect::from_min_size(full.min, egui::vec2(full.width(), BAR_H));
                let drag = ui.interact(bar, egui::Id::new("note-drag"), egui::Sense::click_and_drag());
                if drag.drag_started_by(egui::PointerButton::Primary) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                self.top_bar(ui, bar, &mut close);

                let wide = if collapsed(ctx) { RAIL_W } else { SIDE_W };
                let below = egui::Rect::from_min_max(
                    egui::pos2(full.left() + INSET, bar.bottom() + 4.0),
                    egui::pos2(full.right() - INSET, full.bottom() - INSET),
                );
                let side = egui::Rect::from_min_size(below.min, egui::vec2(wide, below.height()));
                let card = egui::Rect::from_min_max(egui::pos2(side.right() + 6.0, below.top()), below.max);
                self.sidebar(ui, side);
                self.editor_card(ui, card);
            });

        if close {
            self.flush_all();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.flush_all();
    }
}

/// A square icon button with a hover fill and a tooltip.
fn icon_btn(ui: &mut egui::Ui, rect: egui::Rect, id: &str, icon: &str, tip: &str) -> egui::Response {
    let resp = ui.interact(rect, egui::Id::new(id), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 8.0, theme::SURFACE_2);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(18.0),
        if resp.hovered() { theme::FG } else { theme::TEXT_2 },
    );
    resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A sidebar row painted like the main window's nav items.
fn side_row(ui: &mut egui::Ui, rect: egui::Rect, id: egui::Id, icon: Option<&str>, label: &str, selected: bool) -> egui::Response {
    let resp = ui.interact(rect, id, egui::Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 8.0, theme::SURFACE_2);
    } else if resp.hovered() {
        p.rect_filled(rect, 8.0, theme::SURFACE);
    }
    let hot = selected || resp.hovered();
    let mut x = rect.left() + 12.0;
    if let Some(icon) = icon {
        p.text(
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            icon,
            egui::FontId::proportional(18.0),
            if hot { theme::FG } else { theme::TEXT_2 },
        );
        x += 28.0;
    }
    // A long title ends in "…" rather than being cut mid-letter.
    let color = if hot { theme::FG } else { theme::TEXT_2 };
    let mut job = egui::text::LayoutJob::simple_singleline(label.into(), theme::medium(14.0), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - 8.0 - x).max(0.0));
    let g = ui.fonts(|f| f.layout_job(job));
    p.galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

impl NoteApp {
    fn reload(&mut self) {
        self.list = self.store.list();
        self.loaded = Instant::now();
    }

    fn flush_all(&mut self) {
        for t in &mut self.tabs {
            t.flush();
        }
    }

    fn new_tab(&mut self) {
        self.tabs.push(Editor::new_note(self.store.clone()));
        self.active = self.tabs.len() - 1;
    }

    /// Opens a note in a tab, or focuses its tab if it is already open.
    fn open(&mut self, id: &str) {
        if let Some(i) = self.tabs.iter().position(|t| t.id == id) {
            self.active = i;
        } else {
            self.tabs.push(Editor::open(self.store.clone(), id));
            self.active = self.tabs.len() - 1;
        }
    }

    fn close_tab(&mut self, i: usize) {
        self.tabs[i].flush();
        self.tabs.remove(i);
        if self.tabs.is_empty() {
            self.new_tab();
        }
        self.active = self.active.min(self.tabs.len() - 1);
        self.reload();
    }

    /// Logo, tabs and +, then expand and close on the right.
    fn top_bar(&mut self, ui: &mut egui::Ui, bar: egui::Rect, close: &mut bool) {
        let cy = bar.center().y;
        let logo = egui::Rect::from_center_size(egui::pos2(bar.left() + 30.0, cy), egui::vec2(24.0, 24.0));
        ui.put(logo, egui::Image::new((theme::logo_texture(ui.ctx()).id(), logo.size())));

        // right: close, then expand
        let b = 30.0;
        let close_r = egui::Rect::from_center_size(egui::pos2(bar.right() - 26.0, cy), egui::vec2(b, b));
        if icon_btn(ui, close_r, "note-close", icons::X, "Close").clicked() {
            *close = true;
        }
        let exp_r = close_r.translate(egui::vec2(-b - 6.0, 0.0));
        let (icon, tip) = if self.large {
            (icons::ARROWS_IN_SIMPLE, "Smaller")
        } else {
            (icons::ARROWS_OUT_SIMPLE, "Larger")
        };
        if icon_btn(ui, exp_r, "note-expand", icon, tip).clicked() {
            self.large = !self.large;
            let size = if self.large { LARGE } else { COMPACT };
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }

        // tabs
        let limit = exp_r.left() - 44.0;
        let mut x = bar.left() + 64.0;
        let mut activate = None;
        let mut close_tab = None;
        for (i, t) in self.tabs.iter().enumerate() {
            let active = i == self.active;
            let title = short(&t.title(), 16);
            let g = ui.fonts(|f| f.layout_no_wrap(title, theme::medium(14.0), theme::FG));
            let w = 12.0 + g.size().x + 8.0 + 16.0 + 8.0;
            if x + w > limit {
                break;
            }
            let rect = egui::Rect::from_min_max(egui::pos2(x, bar.top() + 8.0), egui::pos2(x + w, bar.bottom()));
            let resp = ui.interact(rect, egui::Id::new(("note-tab", i)), egui::Sense::click());
            let hot = active || resp.hovered();
            let p = ui.painter();
            p.galley(
                egui::pos2(rect.left() + 12.0, rect.center().y - g.size().y / 2.0),
                g,
                if hot { theme::FG } else { theme::TEXT_2 },
            );
            if active {
                p.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(rect.left() + 4.0, rect.bottom() - 2.0),
                        egui::pos2(rect.right() - 4.0, rect.bottom()),
                    ),
                    1.0,
                    theme::ACCENT,
                );
            }
            if resp.clicked() {
                activate = Some(i);
            }
            if hot {
                let xr = egui::Rect::from_center_size(
                    egui::pos2(rect.right() - 16.0, rect.center().y),
                    egui::vec2(18.0, 18.0),
                );
                if icon_btn(ui, xr, &format!("note-tab-x-{i}"), icons::X, "Close tab").clicked() {
                    close_tab = Some(i);
                }
            }
            x += w + 2.0;
        }
        let plus = egui::Rect::from_center_size(egui::pos2(x + 16.0, cy + 2.0), egui::vec2(b, b));
        if icon_btn(ui, plus, "note-plus", icons::PLUS, "New note").clicked() {
            self.new_tab();
        }
        if let Some(i) = close_tab {
            self.close_tab(i);
        } else if let Some(i) = activate {
            self.active = i;
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, side: egui::Rect) {
        let ctx = ui.ctx().clone();
        let rail = collapsed(&ctx);
        let row = |n: usize| {
            egui::Rect::from_min_size(
                egui::pos2(side.left(), side.top() + 4.0 + n as f32 * (ROW_H + 4.0)),
                egui::vec2(side.width(), ROW_H),
            )
        };
        let search_id = egui::Id::new("note-search");
        let mut want_search = false;
        let mut new = false;

        // bottom: Text cleanup
        let cleanup = egui::Rect::from_min_size(
            egui::pos2(side.left(), side.bottom() - ROW_H),
            egui::vec2(side.width(), ROW_H),
        );

        if rail {
            let btn = |r: egui::Rect| egui::Rect::from_center_size(r.center(), egui::vec2(36.0, 36.0));
            if icon_btn(ui, btn(row(0)), "rail-expand", icons::SIDEBAR_SIMPLE, "Show notes").clicked() {
                set_collapsed(&ctx, false);
            }
            if icon_btn(ui, btn(row(1)), "rail-new", icons::NOTE_PENCIL, "New note").clicked() {
                new = true;
            }
            if icon_btn(ui, btn(row(2)), "rail-search", icons::MAGNIFYING_GLASS, "Search notes").clicked() {
                set_collapsed(&ctx, false);
                want_search = true;
            }
            if icon_btn(ui, btn(cleanup), "rail-cleanup", icons::MAGIC_WAND, "Text cleanup").clicked() {
                open_cleanup();
            }
        } else {
            if side_row(ui, row(0), egui::Id::new("side-collapse"), Some(icons::SIDEBAR_SIMPLE), "Collapse notes", false).clicked() {
                set_collapsed(&ctx, true);
            }
            if side_row(ui, row(1), egui::Id::new("side-new"), Some(icons::NOTE_PENCIL), "New note", false).clicked() {
                new = true;
            }
            // search field
            let sr = row(2);
            ui.painter().text(
                egui::pos2(sr.left() + 12.0, sr.center().y),
                egui::Align2::LEFT_CENTER,
                icons::MAGNIFYING_GLASS,
                egui::FontId::proportional(17.0),
                theme::TEXT_2,
            );
            let field = egui::Rect::from_min_max(egui::pos2(sr.left() + 40.0, sr.top()), sr.max);
            let mut f = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(field)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            let resp = f.add(
                egui::TextEdit::singleline(&mut self.search)
                    .id(search_id)
                    .frame(false)
                    .hint_text(egui::RichText::new("Search notes...").color(theme::MUTED))
                    .font(theme::medium(14.0))
                    .desired_width(field.width()),
            );
            if ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new("note-focus-search"))).unwrap_or(false) {
                resp.request_focus();
            }
            let line_y = row(2).bottom() + 4.0;
            ui.painter().hline(
                side.left() + 6.0..=side.right() - 6.0,
                line_y,
                egui::Stroke::new(1.0, theme::BORDER),
            );

            // the list
            let list = egui::Rect::from_min_max(
                egui::pos2(side.left(), line_y + 6.0),
                egui::pos2(side.right(), cleanup.top() - 6.0),
            );
            let q = self.search.trim().to_lowercase();
            let shown: Vec<&NoteMeta> = self
                .list
                .iter()
                .filter(|n| q.is_empty() || n.search.contains(&q))
                .collect();
            let active_id = self.tabs.get(self.active).map(|t| t.id.clone());
            let mut open: Option<String> = None;
            if shown.is_empty() {
                ui.painter().text(
                    egui::pos2(list.center().x, list.top() + 34.0),
                    egui::Align2::CENTER_CENTER,
                    if self.list.is_empty() { "No notes yet" } else { "No matches" },
                    egui::FontId::proportional(14.0),
                    theme::MUTED,
                );
            } else {
                let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(list).layout(egui::Layout::top_down(egui::Align::Min)));
                lu.set_clip_rect(list);
                egui::ScrollArea::vertical()
                    .id_salt("note-list")
                    .auto_shrink(false)
                    .show(&mut lu, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for n in &shown {
                            let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), egui::Sense::hover());
                            let sel = active_id.as_deref() == Some(n.id.as_str());
                            let id = egui::Id::new(("note-item", &n.id));
                            if side_row(ui, r, id, None, &short(&n.title, 19), sel).clicked() {
                                open = Some(n.id.clone());
                            }
                        }
                    });
            }
            if let Some(id) = open {
                self.open(&id);
            }
            if side_row(ui, cleanup, egui::Id::new("side-cleanup"), Some(icons::MAGIC_WAND), "Text cleanup", false).clicked() {
                open_cleanup();
            }
        }
        if want_search {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("note-focus-search"), true));
        }
        if new {
            self.new_tab();
        }
    }

    /// The rounded editor card with the Copy pill floating bottom-right.
    fn editor_card(&mut self, ui: &mut egui::Ui, card: egui::Rect) {
        ui.painter().rect_filled(card, 14.0, theme::PANEL);
        ui.painter().rect_stroke(
            card,
            14.0,
            egui::Stroke::new(1.0, theme::BORDER),
            egui::StrokeKind::Inside,
        );
        let inner = egui::Rect::from_min_max(
            egui::pos2(card.left() + 18.0, card.top() + 14.0),
            egui::pos2(card.right() - 14.0, card.bottom() - 10.0),
        );
        let mut tu = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        tu.set_clip_rect(card.shrink(1.0));
        let key = self.key.clone();
        let Some(ed) = self.tabs.get_mut(self.active) else {
            return;
        };
        ed.body_with(&mut tu, 15.5, Some(&key));

        if ed.text.trim().is_empty() {
            return;
        }
        let pill = egui::Rect::from_min_max(
            egui::pos2(card.right() - 16.0 - 130.0, card.bottom() - 16.0 - 36.0),
            egui::pos2(card.right() - 16.0, card.bottom() - 16.0),
        );
        let mut pu = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(pill)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        let done = self.copied.is_some_and(|t| t.elapsed() < COPIED_FOR);
        let (icon, label) = if done {
            (icons::CHECK, "Copied")
        } else {
            (icons::COPY, "Copy")
        };
        if theme::button_with(&mut pu, theme::Variant::Light, Some(icon), label, false).clicked() {
            ui.ctx().copy_text(ed.text.clone());
            self.copied = Some(Instant::now());
        }
        if done {
            ui.ctx().request_repaint_after(COPIED_FOR);
        }
    }
}

/// Opens the main window on Text cleanup.
fn open_cleanup() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    match std::process::Command::new(exe)
        .args(["settings", "--tab", "cleanup"])
        .stdout(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => log::warn!("could not open Text cleanup: {e}"),
    }
}
