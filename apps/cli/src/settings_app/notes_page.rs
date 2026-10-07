//! The Notes page: hero, recents list, and the editor view. Storage is
//! `crate::notes`, the editor is `crate::notes_ui`.

use std::time::{Duration, Instant};

use eframe::egui;
use egui_phosphor::regular as icons;

use super::{centered_col, list_time, App, PAGE_COL};
use crate::notes::{NoteMeta, Store};
use crate::notes_ui::Editor;
use crate::theme;

/// The list is re-read this often while it is on screen, so a note written
/// from the quick note window shows up.
const RELOAD_EVERY: Duration = Duration::from_secs(3);
const ROW_H: f32 = 60.0;

pub(super) struct NotesState {
    store: Store,
    list: Vec<NoteMeta>,
    loaded: Instant,
    search: String,
    searching: bool,
    editor: Option<Editor>,
}

impl NotesState {
    pub(super) fn new() -> Self {
        let store = Store::default();
        let list = store.list();
        // Dev-only capture hook: `WC_NOTE=<id>` opens that note in the editor.
        let editor = std::env::var("WC_NOTE")
            .ok()
            .filter(|id| crate::notes::valid_id(id))
            .map(|id| Editor::open(store.clone(), &id));
        Self {
            store,
            list,
            loaded: Instant::now(),
            search: String::new(),
            searching: false,
            editor,
        }
    }

    pub(super) fn reload(&mut self) {
        self.list = self.store.list();
        self.loaded = Instant::now();
    }

    /// Autosave for the open note, shown or not. Cheap; call every frame.
    pub(super) fn tick(&mut self, ctx: &egui::Context) {
        if let Some(e) = &mut self.editor {
            e.tick(ctx);
        }
    }

    pub(super) fn flush(&mut self) {
        if let Some(e) = &mut self.editor {
            e.flush();
        }
    }

    pub(super) fn editing(&self) -> bool {
        self.editor.is_some()
    }

    fn new_note(&mut self) {
        self.editor = Some(Editor::new_note(self.store.clone()));
    }

    fn close_editor(&mut self) {
        self.flush();
        self.editor = None;
        self.reload();
    }

    /// Notes whose title or text contains the search, or all of them.
    fn shown(&self) -> Vec<&NoteMeta> {
        let q = self.search.trim().to_lowercase();
        self.list
            .iter()
            .filter(|n| q.is_empty() || n.search.contains(&q))
            .collect()
    }
}

impl App {
    pub(super) fn notes_page(&mut self, ui: &mut egui::Ui) {
        if self.notes.loaded.elapsed() >= RELOAD_EVERY {
            self.notes.reload();
        }
        ui.ctx().request_repaint_after(RELOAD_EVERY);

        let mut on = self.cfg.catcher_notes;
        let mut toggled = false;
        theme::page_header_with(ui, "Notes", Some("Beta"), |ui| {
            toggled = theme::toggle(ui, &mut on).changed();
            super::insights_page::info(ui, "Shows a Notes button when you hover the Catcher");
            ui.label(
                egui::RichText::new("Add to Catcher")
                    .size(14.0)
                    .color(theme::text_2()),
            );
        });
        if toggled {
            // Autosave writes it this frame; the Catcher re-reads the config.
            self.cfg.catcher_notes = on;
        }
        ui.add_space(20.0);

        let mut start = false;
        theme::hero(
            ui,
            "Catch a thought before it slips away",
            "A to-do list, a draft before you send it, an idea for later. Hold fn and talk \
             into a note. Notes stay on this Mac as plain text files.",
            |ui| {
                if theme::button(ui, theme::Variant::Light, "Start new note").clicked() {
                    start = true;
                }
            },
        );
        ui.add_space(28.0);

        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Recents")
                    .font(theme::semibold(17.0))
                    .color(theme::fg()),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                refresh |= theme::button_with(ui, theme::Variant::Ghost, Some(icons::ARROW_CLOCKWISE), "", true)
                    .on_hover_text("Refresh")
                    .clicked();
                start |= theme::button_with(ui, theme::Variant::Ghost, Some(icons::PLUS), "", true)
                    .on_hover_text("New note")
                    .clicked();
                if theme::button_with(ui, theme::Variant::Ghost, Some(icons::MAGNIFYING_GLASS), "", true)
                    .on_hover_text("Search notes")
                    .clicked()
                {
                    self.notes.searching = !self.notes.searching;
                    if !self.notes.searching {
                        self.notes.search.clear();
                    }
                }
            });
        });
        if self.notes.searching {
            ui.add_space(8.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.notes.search)
                    .hint_text("Search notes")
                    .desired_width(f32::INFINITY),
            );
        }
        ui.add_space(8.0);
        theme::hairline(ui);

        let mut open: Option<String> = None;
        let shown = self.notes.shown();
        if shown.is_empty() {
            ui.add_space(72.0);
            let msg = if self.notes.list.is_empty() {
                "No notes yet"
            } else {
                "No matching notes"
            };
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(msg).size(15.0).color(theme::muted()));
            });
            ui.add_space(72.0);
        }
        for (i, n) in shown.iter().enumerate() {
            if i > 0 {
                theme::hairline(ui);
            }
            if note_row(ui, n) {
                open = Some(n.id.clone());
            }
        }

        if refresh {
            self.notes.reload();
        }
        if start {
            self.notes.new_note();
        } else if let Some(id) = open {
            self.notes.editor = Some(Editor::open(self.notes.store.clone(), &id));
        }
    }

    /// The editor view: a fixed-height column, the text scrolls inside it.
    pub(super) fn note_editor_page(&mut self, ui: &mut egui::Ui) {
        ui.add_space(28.0);
        let w = (ui.available_width() - 80.0).clamp(200.0, PAGE_COL);
        let h = (ui.available_height() - 28.0).max(200.0);
        centered_col(ui, w, |ui| {
            ui.set_height(h);
            let mut back = false;
            let mut deleted = false;
            let Some(editor) = self.notes.editor.as_mut() else {
                return;
            };
            ui.horizontal(|ui| {
                if theme::button_with(ui, theme::Variant::Ghost, Some(icons::ARROW_LEFT), "Notes", true)
                    .clicked()
                {
                    back = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    deleted = editor.controls(ui);
                });
            });
            ui.add_space(10.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(editor.title())
                        .font(theme::semibold(24.0))
                        .color(theme::fg()),
                )
                .truncate(),
            );
            ui.add_space(12.0);
            let body_h = (ui.available_height()).max(120.0);
            theme::card(ui)
                .inner_margin(egui::Margin::symmetric(24, 18))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_height(body_h - 38.0);
                    editor.body(ui);
                });
            if back || deleted {
                self.notes.close_editor();
            }
        });
    }
}

/// One recents row: title, a one-line preview, and the time. True on click.
fn note_row(ui: &mut egui::Ui, n: &NoteMeta) -> bool {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_H),
        egui::Sense::hover(),
    );
    let resp = ui.interact(rect, ui.id().with(("note-row", &n.id)), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect.shrink2(egui::vec2(0.0, 2.0)), 8.0, theme::surface());
    }
    let time = list_time(n.modified);
    let time_g = ui.fonts(|f| {
        f.layout_no_wrap(time, egui::FontId::monospace(11.5), theme::muted())
    });
    let pad = 12.0;
    let right = rect.right() - pad;
    ui.painter().galley(
        egui::pos2(right - time_g.size().x, rect.center().y - time_g.size().y / 2.0),
        time_g.clone(),
        theme::muted(),
    );
    let left = egui::Rect::from_min_max(
        egui::pos2(rect.left() + pad, rect.top() + 9.0),
        egui::pos2(right - time_g.size().x - 16.0, rect.bottom() - 6.0),
    );
    let mut text = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    text.spacing_mut().item_spacing.y = 3.0;
    text.add(
        egui::Label::new(
            egui::RichText::new(&n.title)
                .font(theme::medium(14.5))
                .color(theme::fg()),
        )
        .truncate()
        .selectable(false),
    );
    if !n.preview.is_empty() {
        text.add(
            egui::Label::new(egui::RichText::new(&n.preview).size(13.0).color(theme::muted()))
                .truncate()
                .selectable(false),
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}
