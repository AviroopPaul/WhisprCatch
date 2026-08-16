//! First-run setup wizard: Welcome → keyboard permission (via polkit, no
//! terminal) → model download with progress → Done. Runs before the daemon
//! starts. Fixed-size centered window, one high-emphasis action per screen;
//! the window itself is the card — whitespace does the work.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use eframe::egui;
use wc_models::ModelId;

use crate::theme;

pub enum Outcome {
    /// Setup finished; start the daemon.
    Ready,
    /// User closed the window before finishing.
    Cancelled,
}

enum Step {
    Welcome,
    Permission {
        granting: bool,
        rx: Option<Receiver<Result<(), String>>>,
        error: Option<String>,
    },
    Download {
        started: bool,
        rx: Option<Receiver<DlMsg>>,
        file: String,
        done: u64,
        total: u64,
        error: Option<String>,
    },
    Done,
}

enum DlMsg {
    Progress { file: String, done: u64, total: u64 },
    Finished,
    Failed(String),
}

#[cfg(target_os = "macos")]
pub fn need_setup(model: ModelId) -> bool {
    // macOS permission grants only register after an app relaunch, so we never
    // hard-gate first-run on them (that traps the user in the wizard). The
    // wizard guides granting; Settings › Permissions manages it afterwards.
    !model.spec().is_complete(&wc_core::models_dir())
}

#[cfg(not(target_os = "macos"))]
pub fn need_setup(model: ModelId) -> bool {
    !wc_hotkey::keyboard_accessible() || !model.spec().is_complete(&wc_core::models_dir())
}

/// Opening size of the wizard window, in points.
const WINDOW_W: f32 = 560.0;
const WINDOW_H: f32 = 640.0;

/// Blocks on the wizard window; returns when setup is complete or abandoned.
pub fn run(model: ModelId, key_label: &str) -> Result<Outcome> {
    let outcome = Arc::new(Mutex::new(Outcome::Cancelled));
    let out = outcome.clone();
    let key = key_label.to_string();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WINDOW_W, WINDOW_H])
            .with_min_inner_size([WINDOW_W, WINDOW_H])
            .with_resizable(false),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        &format!("{} Setup", crate::app_name()),
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            theme::install_fonts(&cc.egui_ctx);
            Ok(Box::new(Wizard::new(out, model, key)) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| anyhow::anyhow!("setup window failed: {e}"))?;

    Ok(Arc::try_unwrap(outcome)
        .map(|m| m.into_inner().unwrap())
        .unwrap_or(Outcome::Cancelled))
}

struct Wizard {
    step: Step,
    outcome: Arc<Mutex<Outcome>>,
    model: ModelId,
    key_label: String,
    /// Cleared after the opening size has been asserted once.
    needs_size: bool,
    shot: crate::shot::Shot,
}

impl Wizard {
    fn new(outcome: Arc<Mutex<Outcome>>, model: ModelId, key_label: String) -> Self {
        Self {
            step: forced_step().unwrap_or(Step::Welcome),
            outcome,
            model,
            key_label,
            needs_size: true,
            shot: crate::shot::Shot::from_env(),
        }
    }
}

/// Dev-only: `WC_WIZARD_STEP=welcome|permission|download|done` opens the
/// wizard on that step, so design work and screenshots don't depend on the
/// machine's real permission and model state. The forced download step is
/// inert — `started: true` with no receiver means nothing is fetched.
fn forced_step() -> Option<Step> {
    match std::env::var("WC_WIZARD_STEP").ok()?.as_str() {
        "welcome" => Some(Step::Welcome),
        "permission" => Some(Step::Permission {
            granting: false,
            rx: None,
            error: None,
        }),
        "download" => Some(Step::Download {
            started: true,
            rx: None,
            file: "encoder_model.int8.onnx".into(),
            done: 431_000_000,
            total: 660_000_000,
            error: None,
        }),
        "done" => Some(Step::Done),
        _ => None,
    }
}

impl Step {
    fn first_download(model: ModelId) -> Step {
        if model.spec().is_complete(&wc_core::models_dir()) {
            Step::Done
        } else {
            Step::Download {
                started: false,
                rx: None,
                file: String::new(),
                done: 0,
                total: model.spec().total_size(),
                error: None,
            }
        }
    }

    fn index(&self) -> usize {
        match self {
            Step::Welcome => 0,
            Step::Permission { .. } => 1,
            Step::Download { .. } => 2,
            Step::Done => 3,
        }
    }
}

/// Linux: grants keyboard access via polkit (GUI password prompt):
/// input-group membership for permanence + ACLs so it works immediately.
#[cfg(target_os = "linux")]
fn grant_keyboard_access() -> Result<(), String> {
    let user = std::env::var("USER").map_err(|_| "cannot determine username".to_string())?;
    let script = r#"set -e
usermod -aG input "$1"
setfacl -m "u:$1:r" /dev/input/event* 2>/dev/null || true
if [ -e /dev/uinput ]; then setfacl -m "u:$1:rw" /dev/uinput 2>/dev/null || true; fi"#;
    let status = std::process::Command::new("pkexec")
        .args(["sh", "-c", script, "sh", &user])
        .status()
        .map_err(|e| format!("could not run pkexec: {e}"))?;
    if !status.success() {
        return Err("authorization was cancelled or failed".into());
    }
    if !wc_hotkey::keyboard_accessible() {
        return Err(
            "access granted, but not active yet. Log out and back in, then reopen the app"
                .into(),
        );
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn grant_keyboard_access() -> Result<(), String> {
    Err("keyboard access setup is not implemented on this platform".into())
}

/// Open one System Settings privacy pane.
#[cfg(target_os = "macos")]
fn open_pane(pane: &str) {
    let _ = std::process::Command::new("open")
        .arg(format!("x-apple.systempreferences:com.apple.preference.security?{pane}"))
        .status();
}

/// The three grants macOS needs, in the order the user meets them.
/// `granted` is None where the system gives us no way to ask (Microphone is
/// prompted on first capture), so the row shows "on first use" instead of a
/// wrong answer.
#[cfg(target_os = "macos")]
fn permissions() -> [(&'static str, &'static str, Option<bool>, &'static str); 3] {
    [
        (
            "Accessibility",
            "Lets it type the transcript into the focused app",
            Some(wc_hotkey::keyboard_accessible()),
            "Privacy_Accessibility",
        ),
        (
            "Input Monitoring",
            "Lets it notice the push-to-talk key going down",
            Some(wc_hotkey::input_monitoring_granted()),
            "Privacy_ListenEvent",
        ),
        (
            "Microphone",
            "Asked the first time you dictate",
            None,
            "Privacy_Microphone",
        ),
    ]
}

/// One row of the permission checklist: status dot, name, why it is needed,
/// and its own button to the exact pane. Returns true if the button was hit.
#[cfg(target_os = "macos")]
fn perm_row(ui: &mut egui::Ui, name: &str, why: &str, granted: Option<bool>) -> bool {
    let mut clicked = false;
    egui::Frame::default()
        .fill(theme::SURFACE)
        .stroke(egui::Stroke::new(
            1.0,
            if granted == Some(true) {
                theme::tint_strong(theme::MINT)
            } else {
                theme::BORDER
            },
        ))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let dot = match granted {
                    Some(true) => theme::MINT,
                    Some(false) => theme::MUTED,
                    None => theme::AMBER,
                };
                theme::led(ui, dot, false);
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(
                        egui::RichText::new(name)
                            .font(theme::medium(13.5))
                            .color(theme::FG),
                    );
                    ui.label(
                        egui::RichText::new(why)
                            .size(11.5)
                            .color(theme::MUTED),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    match granted {
                        Some(true) => {
                            ui.label(theme::mono_upper("on", 10.5, theme::MINT));
                        }
                        Some(false) => {
                            clicked = theme::ghost_button(ui, "Open").clicked();
                        }
                        None => {
                            ui.label(theme::mono_upper("on first use", 10.5, theme::MUTED));
                        }
                    }
                });
            });
        });
    clicked
}

// ---------------------------------------------------------------------------
// Painted UI pieces (no icon fonts needed; crisp at any DPI).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum StepIcon {
    Mic,
    Keyboard,
    Download,
    Check,
}

/// Stroke icon centered on a 72px mint-tinted plate. The icon carries the
/// accent rather than the plate, so the step reads as one bright object on
/// a dark field instead of a grey disc.
fn icon_plate(ui: &mut egui::Ui, icon: StepIcon) {
    let ink = theme::MINT;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(72.0, 72.0), egui::Sense::hover());
    let c = rect.center();
    let p = ui.painter();
    p.circle_filled(c, 36.0, theme::SURFACE);
    p.circle_stroke(c, 36.0, egui::Stroke::new(1.0, theme::tint_strong(theme::MINT)));
    let s = egui::Stroke::new(2.5, ink);
    match icon {
        StepIcon::Mic => {
            // capsule body
            p.rect_stroke(
                egui::Rect::from_center_size(egui::pos2(c.x, c.y - 6.0), egui::vec2(13.0, 22.0)),
                egui::CornerRadius::same(6),
                s,
                egui::StrokeKind::Middle,
            );
            // holder arc
            let arc: Vec<egui::Pos2> = (0..=24)
                .map(|i| {
                    let t = std::f32::consts::PI * i as f32 / 24.0;
                    egui::pos2(c.x + 12.5 * t.cos(), (c.y - 2.0) + 12.5 * t.sin())
                })
                .collect();
            p.add(egui::Shape::line(arc, s));
            // stem + base
            p.line_segment([egui::pos2(c.x, c.y + 10.5), egui::pos2(c.x, c.y + 16.0)], s);
            p.line_segment(
                [egui::pos2(c.x - 7.0, c.y + 16.0), egui::pos2(c.x + 7.0, c.y + 16.0)],
                s,
            );
            // recording LED next to the mic
            p.circle_filled(egui::pos2(c.x + 14.0, c.y - 14.0), 3.0, theme::RED);
        }
        StepIcon::Keyboard => {
            p.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(38.0, 26.0)),
                egui::CornerRadius::same(5),
                egui::Stroke::new(2.0, ink),
                egui::StrokeKind::Middle,
            );
            for y in [-6.0_f32, 0.0] {
                for k in -2..=2_i32 {
                    p.circle_filled(egui::pos2(c.x + k as f32 * 6.0, c.y + y), 1.5, ink);
                }
            }
            // space bar
            p.line_segment(
                [egui::pos2(c.x - 8.0, c.y + 6.5), egui::pos2(c.x + 8.0, c.y + 6.5)],
                egui::Stroke::new(2.5, ink),
            );
        }
        StepIcon::Download => {
            // arrow
            p.line_segment([egui::pos2(c.x, c.y - 15.0), egui::pos2(c.x, c.y + 3.0)], s);
            p.add(egui::Shape::line(
                vec![
                    egui::pos2(c.x - 7.0, c.y - 4.0),
                    egui::pos2(c.x, c.y + 3.5),
                    egui::pos2(c.x + 7.0, c.y - 4.0),
                ],
                s,
            ));
            // tray
            p.add(egui::Shape::line(
                vec![
                    egui::pos2(c.x - 13.0, c.y + 8.0),
                    egui::pos2(c.x - 13.0, c.y + 14.0),
                    egui::pos2(c.x + 13.0, c.y + 14.0),
                    egui::pos2(c.x + 13.0, c.y + 8.0),
                ],
                s,
            ));
        }
        StepIcon::Check => {
            p.circle_stroke(c, 16.0, egui::Stroke::new(2.5, theme::MINT));
            p.add(egui::Shape::line(
                vec![
                    egui::pos2(c.x - 7.5, c.y + 0.5),
                    egui::pos2(c.x - 2.5, c.y + 6.0),
                    egui::pos2(c.x + 8.0, c.y - 5.5),
                ],
                egui::Stroke::new(3.0, theme::MINT),
            ));
        }
    }
}

/// Four 8px dots: done = green fill, current = green ring, upcoming = raised.
fn step_dots(ui: &mut egui::Ui, current: usize) {
    let n = 4;
    let r = 4.0;
    let gap = 12.0;
    let w = n as f32 * 2.0 * r + (n - 1) as f32 * gap;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 2.0 * r + 4.0), egui::Sense::hover());
    let p = ui.painter();
    for i in 0..n {
        let c = egui::pos2(
            rect.left() + r + i as f32 * (2.0 * r + gap),
            rect.center().y,
        );
        if i < current {
            p.circle_filled(c, r, theme::MINT);
        } else if i == current {
            p.circle_stroke(c, r + 0.5, egui::Stroke::new(1.5, theme::MINT));
        } else {
            p.circle_filled(c, r, theme::SURFACE_2);
        }
    }
}

/// Centered column for step copy — max 380px so lines stay readable.
fn step_body(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let w = ui.available_width().min(380.0);
    ui.allocate_ui_with_layout(
        egui::vec2(w, 0.0),
        egui::Layout::top_down(egui::Align::Center),
        add,
    );
}

/// Step title: the website's two-tone display headline.
fn title(ui: &mut egui::Ui, roman: &str, italic: &str) {
    theme::display(ui, roman, italic, 31.0);
}

fn body_text(text: &str) -> egui::RichText {
    egui::RichText::new(text).size(13.5).color(theme::TEXT_2)
}

/// The one high-emphasis button per screen: the website's mint CTA.
fn primary_button(ui: &mut egui::Ui, text: &str, min: egui::Vec2) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(text)
                .font(theme::medium(14.0))
                .color(theme::ON_MINT),
        )
        .fill(theme::MINT)
        .stroke(egui::Stroke::NONE)
        .corner_radius(egui::CornerRadius::same(10))
        .min_size(min),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Green status chip, mono uppercase (welcome privacy line). Painted at
/// exact content size — a Frame would stretch to the column width here.
fn status_chip(ui: &mut egui::Ui, text: &str) {
    let galley = ui.fonts(|f| {
        f.layout_no_wrap(text.to_uppercase(), theme::mono_medium(10.5), theme::MINT)
    });
    let pad = egui::vec2(10.0, 5.0);
    let (rect, _) =
        ui.allocate_exact_size(galley.size() + pad * 2.0, egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, egui::CornerRadius::same(4), theme::tint(theme::MINT));
    p.galley(rect.min + pad, galley, theme::MINT);
}

/// Error state: tinted panel, error text, optional recovery hint.
fn error_box(ui: &mut egui::Ui, msg: &str, hint: Option<&str>) {
    egui::Frame::default()
        .fill(theme::tint(theme::RED))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.colored_label(theme::RED, msg);
            if let Some(h) = hint {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(h).small().color(theme::MUTED));
            }
        });
}

/// "Hold ⟨key⟩ and speak. Release to type." with the amber hotkey chip.
fn hotkey_line(ui: &mut egui::Ui, key_label: &str) {
    let body = egui::TextStyle::Body.resolve(ui.style());

    let pre = ui.fonts(|f| f.layout_no_wrap("Hold".into(), body.clone(), theme::TEXT_2));
    let post =
        ui.fonts(|f| f.layout_no_wrap("and speak. Release to type.".into(), body, theme::TEXT_2));
    let key = ui.fonts(|f| {
        f.layout_no_wrap(
            key_label.to_uppercase(),
            theme::mono_medium(12.0),
            theme::AMBER,
        )
    });

    let pad = egui::vec2(10.0, 6.0);
    let chip_size = key.size() + pad * 2.0;
    let gap = 8.0;
    let total = egui::vec2(
        pre.size().x + gap + chip_size.x + gap + post.size().x,
        chip_size.y + 3.0,
    );
    let (rect, _) = ui.allocate_exact_size(total, egui::Sense::hover());
    let p = ui.painter();
    let cy = rect.center().y;
    let mut x = rect.left();

    p.galley(egui::pos2(x, cy - pre.size().y / 2.0), pre.clone(), theme::TEXT_2);
    x += pre.size().x + gap;

    let chip = egui::Rect::from_min_size(
        egui::pos2(x, cy - chip_size.y / 2.0 - 1.0),
        chip_size,
    );
    p.rect_filled(chip, egui::CornerRadius::same(5), theme::tint(theme::AMBER));
    p.rect_stroke(
        chip,
        egui::CornerRadius::same(5),
        egui::Stroke::new(1.0, theme::RING),
        egui::StrokeKind::Inside,
    );
    // the "key" bottom edge
    p.line_segment(
        [
            egui::pos2(chip.left() + 5.0, chip.bottom() + 1.5),
            egui::pos2(chip.right() - 5.0, chip.bottom() + 1.5),
        ],
        egui::Stroke::new(2.0, theme::RING),
    );
    p.galley(chip.min + pad, key, theme::AMBER);
    x += chip_size.x + gap;

    p.galley(egui::pos2(x, cy - post.size().y / 2.0), post, theme::TEXT_2);
}

// ---------------------------------------------------------------------------

impl eframe::App for Wizard {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // macOS hands the window back at whatever size it feels like unless
        // the size is asserted from inside the app; without this the wizard
        // opens at twice its designed size on a Retina display.
        if self.needs_size {
            self.needs_size = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                WINDOW_W, WINDOW_H,
            )));
        }
        self.shot.tick(ctx);
        // poll background work
        let mut advance_to_download = false;
        match &mut self.step {
            Step::Permission { granting, rx, error } => {
                if let Some(r) = rx {
                    if let Ok(res) = r.try_recv() {
                        *granting = false;
                        *rx = None;
                        match res {
                            Ok(()) => advance_to_download = true,
                            Err(e) => *error = Some(e),
                        }
                    }
                }
            }
            Step::Download { rx, file, done, total, error, .. } => {
                let mut finished = false;
                if let Some(r) = rx {
                    while let Ok(msg) = r.try_recv() {
                        match msg {
                            DlMsg::Progress { file: f, done: d, total: t } => {
                                *file = f;
                                *done = d;
                                *total = t;
                            }
                            DlMsg::Finished => finished = true,
                            DlMsg::Failed(e) => *error = Some(e),
                        }
                    }
                }
                if finished {
                    self.step = Step::Done;
                }
            }
            _ => {}
        }
        if advance_to_download {
            self.step = Step::first_download(self.model);
        }

        let step_idx = self.step.index();
        let mut next: Option<Step> = None;
        let model = self.model;

        // pinned action area — buttons live at a stable position near the bottom
        egui::TopBottomPanel::bottom("wizard-actions")
            .show_separator_line(false)
            .frame(
                egui::Frame::default()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin { left: 24, right: 24, top: 8, bottom: 40 }),
            )
            .show(ctx, |ui| {
                ui.set_min_height(48.0);
                ui.vertical_centered(|ui| match &mut self.step {
                    Step::Welcome => {
                        if primary_button(ui, "Get started", egui::vec2(220.0, 40.0)).clicked() {
                            // Show the checklist unless every switch we can read is
                            // already on.
                            #[cfg(target_os = "macos")]
                            let all_set = wc_hotkey::keyboard_accessible()
                                && wc_hotkey::input_monitoring_granted();
                            #[cfg(not(target_os = "macos"))]
                            let all_set = wc_hotkey::keyboard_accessible();
                            next = Some(if all_set {
                                Step::first_download(model)
                            } else {
                                Step::Permission {
                                    granting: false,
                                    rx: None,
                                    error: None,
                                }
                            });
                        }
                    }
                    Step::Permission { granting, rx, error } => {
                        // macOS: each row opens its own pane, so the action here is
                        // simply to move on. The grants are never a gate — macOS
                        // caches them per process, so one just granted still reads
                        // as denied until relaunch, and blocking would trap the user.
                        #[cfg(target_os = "macos")]
                        {
                            let _ = (&granting, &rx, &error);
                            if primary_button(ui, "Continue", egui::vec2(220.0, 40.0)).clicked() {
                                next = Some(Step::first_download(model));
                            }
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(
                                    "You can flip these on later in Settings › Permissions.",
                                )
                                .small()
                                .color(theme::MUTED),
                            );
                        }
                        #[cfg(not(target_os = "macos"))]
                        {
                            if *granting {
                                ui.add(egui::Spinner::new().size(20.0).color(theme::AMBER));
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new("Waiting for authorization…")
                                        .small()
                                        .color(theme::MUTED),
                                );
                            } else if primary_button(
                                ui,
                                "Grant keyboard access…",
                                egui::vec2(240.0, 40.0),
                            )
                            .clicked()
                            {
                                let (tx, r) = mpsc::channel();
                                *rx = Some(r);
                                *granting = true;
                                *error = None;
                                let ctx2 = ui.ctx().clone();
                                std::thread::spawn(move || {
                                    let _ = tx.send(grant_keyboard_access());
                                    ctx2.request_repaint();
                                });
                            }
                        }
                    }
                    Step::Download { .. } => {
                        ui.add_space(12.0);
                        ui.label(
                            egui::RichText::new(
                                "This happens once. Later launches start straight away.",
                            )
                            .small()
                            .color(theme::MUTED),
                        );
                    }
                    Step::Done => {
                        if primary_button(ui, "Start dictating", egui::vec2(220.0, 44.0)).clicked()
                        {
                            *self.outcome.lock().unwrap() = Outcome::Ready;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(24, 0)),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(32.0);
                    step_dots(ui, step_idx);
                    ui.add_space(10.0);
                    ui.label(theme::mono_upper(
                        &format!("step {} of 4", step_idx + 1),
                        10.0,
                        theme::MUTED,
                    ));
                    ui.add_space(24.0);
                    let icon = match &self.step {
                        Step::Welcome => StepIcon::Mic,
                        Step::Permission { .. } => StepIcon::Keyboard,
                        Step::Download { .. } => StepIcon::Download,
                        Step::Done => StepIcon::Check,
                    };
                    icon_plate(ui, icon);
                    ui.add_space(24.0);

                    match &mut self.step {
                        Step::Welcome => {
                            title(ui, "Welcome to ", crate::app_name());
                            ui.add_space(12.0);
                            step_body(ui, |ui| {
                                ui.label(body_text(
                                    "Push-to-talk dictation for your desktop. Hold a key, \
                                     speak, and your words are typed wherever your cursor is.",
                                ));
                            });
                            ui.add_space(16.0);
                            status_chip(ui, "Everything stays on this device");
                            ui.add_space(16.0);
                            step_body(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(
                                        "Two quick steps: keyboard access, then the speech \
                                         model. You won't see this window again.",
                                    )
                                    .small()
                                    .color(theme::MUTED),
                                );
                            });
                        }
                        Step::Permission { error, .. } => {
                            #[cfg(target_os = "macos")]
                            {
                                title(ui, "Three ", "permissions.");
                                ui.add_space(12.0);
                                step_body(ui, |ui| {
                                    ui.label(body_text(
                                        "macOS keeps these switches to itself. Flip each one \
                                         on for WhisprCatch and this list turns mint.",
                                    ));
                                });
                                ui.add_space(20.0);
                                // Wider than the prose column: these are controls, not copy.
                                let w = ui.available_width().min(400.0);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(w, 0.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.spacing_mut().item_spacing.y = 8.0;
                                        for (name, why, granted, pane) in permissions() {
                                            if perm_row(ui, name, why, granted) {
                                                // Accessibility is the one macOS will
                                                // prompt for; the others just need the pane.
                                                if pane == "Privacy_Accessibility" {
                                                    wc_hotkey::request_accessibility();
                                                }
                                                open_pane(pane);
                                            }
                                        }
                                    },
                                );
                            }
                            #[cfg(not(target_os = "macos"))]
                            {
                                title(ui, "Keyboard ", "access.");
                                ui.add_space(12.0);
                                step_body(ui, |ui| {
                                    ui.label(body_text(
                                        "To notice when you hold the dictation key, \
                                         WhisprCatch needs permission to read your keyboard. \
                                         You'll be asked for your password once.",
                                    ));
                                });
                            }
                            if let Some(e) = error {
                                ui.add_space(16.0);
                                let msg = e.clone();
                                step_body(ui, |ui| {
                                    error_box(
                                        ui,
                                        &msg,
                                        Some("Nothing was changed. You can try again below."),
                                    );
                                });
                            }
                        }
                        Step::Download { started, rx, file, done, total, error } => {
                            if !*started {
                                *started = true;
                                let (tx, r) = mpsc::channel();
                                *rx = Some(r);
                                let ctx2 = ui.ctx().clone();
                                std::thread::spawn(move || {
                                    let res = model.spec().ensure_with(
                                        &wc_core::models_dir(),
                                        &|f, d, t| {
                                            let _ = tx.send(DlMsg::Progress {
                                                file: f.to_string(),
                                                done: d,
                                                total: t,
                                            });
                                            ctx2.request_repaint();
                                        },
                                    );
                                    let _ = tx.send(match res {
                                        Ok(_) => DlMsg::Finished,
                                        Err(e) => DlMsg::Failed(format!("{e:#}")),
                                    });
                                    ctx2.request_repaint();
                                });
                            }
                            title(ui, "The speech ", "model.");
                            ui.add_space(12.0);
                            let dl_line = format!(
                                "Downloading {}, about {} MB, one time. Everything runs \
                                 locally; audio never leaves this machine.",
                                model.label(),
                                model.download_mb(),
                            );
                            step_body(ui, |ui| {
                                ui.label(body_text(&dl_line));
                            });
                            ui.add_space(24.0);
                            let frac = if *total > 0 {
                                *done as f32 / *total as f32
                            } else {
                                0.0
                            };
                            let mb_line = format!(
                                "{:.0}% · {:.0} / {:.0} MB · {}",
                                frac * 100.0,
                                *done as f64 / 1e6,
                                *total as f64 / 1e6,
                                if file.is_empty() { "preparing" } else { file.as_str() }
                            );
                            let err = error.clone();
                            step_body(ui, |ui| {
                                ui.add(
                                    egui::ProgressBar::new(frac)
                                        .desired_height(6.0)
                                        .fill(theme::MINT)
                                        .corner_radius(egui::CornerRadius::same(4)),
                                );
                                ui.add_space(8.0);
                                ui.label(theme::mono_upper(&mb_line, 10.0, theme::MUTED));
                                if let Some(e) = err {
                                    ui.add_space(16.0);
                                    error_box(
                                        ui,
                                        &e,
                                        Some(
                                            "Downloads resume where they left off. Close and \
                                             reopen the app to retry.",
                                        ),
                                    );
                                }
                            });
                        }
                        Step::Done => {
                            title(ui, "You're all ", "set.");
                            ui.add_space(16.0);
                            hotkey_line(ui, &self.key_label);
                            ui.add_space(20.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(300.0, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.spacing_mut().item_spacing.y = 8.0;
                                    for (dot, line) in [
                                        (theme::MINT, "Text lands wherever your cursor is."),
                                        (theme::RED, "A small pill shows while it listens."),
                                        (theme::AMBER, "History and settings live in the tray."),
                                    ] {
                                        ui.horizontal(|ui| {
                                            theme::led(ui, dot, false);
                                            ui.label(body_text(line));
                                        });
                                    }
                                },
                            );
                            ui.add_space(20.0);
                            ui.label(
                                egui::RichText::new(
                                    "Built for people who think faster than they type.",
                                )
                                .small()
                                .color(theme::MUTED)
                                .italics(),
                            );
                        }
                    }
                });
            });

        if let Some(s) = next {
            self.step = s;
            ctx.request_repaint();
        }

        match &self.step {
            Step::Permission { .. } | Step::Download { .. } => {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
            _ => {}
        }
    }
}

/// Small fatal-error window for desktop launches (no terminal to print to).
/// macOS surfaces failures via a notification instead (GUI must be main-thread).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn error_window(message: &str) {
    let msg = message.to_string();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([460.0, 240.0])
            .with_resizable(false),
        centered: true,
        ..Default::default()
    };
    let _ = eframe::run_native(
        &format!("{}: error", crate::app_name()),
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            theme::install_fonts(&cc.egui_ctx);
            Ok(Box::new(ErrorApp { msg }) as Box<dyn eframe::App>)
        }),
    );
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
struct ErrorApp {
    msg: String,
}

impl eframe::App for ErrorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::same(20)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    theme::led(ui, theme::RED, false);
                    ui.label(
                        egui::RichText::new("Something went wrong")
                            .font(theme::semibold(15.0))
                            .color(theme::FG),
                    );
                });
                ui.add_space(8.0);
                egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                    ui.label(egui::RichText::new(&self.msg).color(theme::TEXT_2));
                });
                ui.add_space(12.0);
                if ui.button("Close").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
    }
}
