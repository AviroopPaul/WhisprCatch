//! The note editor, shared by the Notes page and the quick note window.
//!
//! `body` draws the text area, `controls` the status and the actions, and
//! `tick` (call it every frame, shown or not) does the debounced autosave.

use std::time::{Duration, Instant};

use eframe::egui;
use egui_phosphor::regular as icons;

use crate::notes::{self, Store};
use crate::theme;

/// Quiet time after the last keystroke before the note is written.
const DEBOUNCE: Duration = Duration::from_millis(600);
const HINT: &str = "Hold fn and talk, or start typing.";

pub struct Editor {
    store: Store,
    pub id: String,
    pub text: String,
    /// What is on disk (or empty when there is no file yet).
    saved: String,
    last_edit: Option<Instant>,
    want_focus: bool,
    confirm_delete: bool,
    error: Option<String>,
}

impl Editor {
    pub fn new_note(store: Store) -> Self {
        let id = store.create();
        Self::with(store, id, String::new())
    }

    pub fn open(store: Store, id: &str) -> Self {
        let text = store.load(id);
        Self::with(store, id.to_string(), text)
    }

    fn with(store: Store, id: String, text: String) -> Self {
        Self {
            store,
            id,
            saved: text.clone(),
            text,
            last_edit: None,
            want_focus: true,
            confirm_delete: false,
            error: None,
        }
    }

    pub fn title(&self) -> String {
        notes::title_of(&self.text)
    }

    fn dirty(&self) -> bool {
        self.text != self.saved
    }

    /// Writes the note now if it changed.
    pub fn flush(&mut self) {
        if !self.dirty() {
            return;
        }
        match self.store.save(&self.id, &self.text) {
            Ok(()) => {
                self.saved = self.text.clone();
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Could not save: {e}")),
        }
        self.last_edit = None;
    }

    /// The debounced autosave. Cheap; call every frame.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if !self.dirty() {
            return;
        }
        let quiet = self.last_edit.map_or(DEBOUNCE, |t| t.elapsed());
        if quiet >= DEBOUNCE {
            self.flush();
        } else {
            ctx.request_repaint_after(DEBOUNCE - quiet);
        }
    }

    /// The text area: no frame, FG text at 15, filling the space it is given.
    pub fn body(&mut self, ui: &mut egui::Ui) {
        self.body_with(ui, 15.0, None);
    }

    /// [`body`](Self::body) at `size`. With `key` (the hotkey's label) the
    /// empty note shows a keycap and "to dictate" instead of the hint text.
    pub fn body_with(&mut self, ui: &mut egui::Ui, size: f32, key: Option<&str>) {
        let min = egui::vec2(ui.available_width(), ui.available_height().max(120.0));
        let id = egui::Id::new(("note-editor", &self.id));
        let font = egui::FontId::proportional(size);
        let out = egui::ScrollArea::vertical()
            .id_salt(("note-scroll", &self.id))
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.text)
                        .id(id)
                        .frame(false)
                        .font(font)
                        .text_color(theme::FG)
                        .hint_text(egui::RichText::new(if key.is_some() { "" } else { HINT }).color(theme::MUTED))
                        .desired_width(f32::INFINITY)
                        .min_size(min)
                        .lock_focus(true),
                )
            });
        let resp = out.inner;
        if let (Some(key), true) = (key, self.text.is_empty()) {
            // painted over the first line so the keycap can render
            let at = egui::Rect::from_min_size(
                resp.rect.min + egui::vec2(0.0, 1.0),
                egui::vec2(220.0, size * 1.5),
            );
            let mut cap = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(at)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            cap.spacing_mut().item_spacing.x = 8.0;
            theme::kbd(&mut cap, key);
            cap.label(
                egui::RichText::new("to dictate")
                    .size(size)
                    .color(theme::MUTED),
            );
        }
        if resp.changed() {
            self.last_edit = Some(Instant::now());
        }
        if self.want_focus {
            self.want_focus = false;
            resp.request_focus();
            // an existing note opens with the caret at its end
            let n = self.text.chars().count();
            if n > 0 {
                if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), id) {
                    st.cursor
                        .set_char_range(Some(egui::text::CCursorRange::one(
                            egui::text::CCursor::new(n),
                        )));
                    st.store(ui.ctx(), id);
                }
            }
            ui.ctx().request_repaint();
        }
    }

    /// Save state and the Show in Finder / Delete actions, laid out
    /// right-to-left in the row it is given. Returns true once the note has
    /// been deleted.
    pub fn controls(&mut self, ui: &mut egui::Ui) -> bool {
        let mut deleted = false;
        if self.confirm_delete {
            if theme::button_with(ui, theme::Variant::Destructive, None, "Delete", true).clicked()
            {
                let _ = self.store.delete(&self.id);
                self.text.clear();
                self.saved.clear();
                self.last_edit = None;
                self.confirm_delete = false;
                deleted = true;
            }
            if theme::button_with(ui, theme::Variant::Ghost, None, "Keep", true).clicked() {
                self.confirm_delete = false;
            }
            ui.label(
                egui::RichText::new("Delete this note?")
                    .size(13.0)
                    .color(theme::TEXT_2),
            );
            return deleted;
        }
        if theme::button_with(ui, theme::Variant::Ghost, Some(icons::TRASH), "", true)
            .on_hover_text("Delete note")
            .clicked()
            && !self.text.is_empty()
        {
            self.confirm_delete = true;
        }
        let reveal = if cfg!(target_os = "macos") {
            "Show in Finder"
        } else {
            "Open folder"
        };
        if theme::button_with(ui, theme::Variant::Ghost, Some(icons::FOLDER_OPEN), reveal, true)
            .on_hover_text("The notes are plain .md files in this folder")
            .clicked()
        {
            crate::settings_app::open_folder(self.store.dir());
        }
        let (text, color) = match (&self.error, self.dirty(), self.text.is_empty()) {
            (Some(e), _, _) => (e.as_str(), theme::RED),
            (None, true, _) => ("Saving", theme::MUTED),
            (None, false, false) => ("Saved", theme::MUTED),
            (None, false, true) => ("", theme::MUTED),
        };
        ui.label(egui::RichText::new(text).size(12.5).color(color));
        deleted
    }
}
