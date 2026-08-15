//! Floating recording indicator — a small always-on-top pill at the bottom
//! center of the screen while dictating. Runs as its own process
//! (`whisper-catch overlay`): the daemon spawns it on key-down, writes a
//! line to its stdin when transcription starts, and closes stdin (EOF) to
//! dismiss it.
//!
//! Look per docs/DESIGN.md: dark translucent rounded-full pill with a subtle
//! ring. Listening = 4-bar waveform + label. Transcribing = amber spinner +
//! label.

use std::io::BufRead;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use eframe::egui;

use crate::theme;

// Sized to the wider of the two states ("Transcribing…"), now that the LED and
// the elapsed timer are gone.
const W: f32 = 152.0;
const H: f32 = 40.0;
/// Gap between the pill and the bottom edge of the screen.
const BOTTOM_GAP: f32 = 64.0;

/// Bottom-centre of the display to show the pill on, in the *global* desktop
/// coordinate space that `OuterPosition` uses.
///
/// egui reports the monitor's size but not its origin, which is only safe when
/// that origin is (0,0). On macOS a second display routinely sits at a negative
/// origin — e.g. main display 1440x900 at (0,0) with an external 1920x1080 at
/// (-260,-1080) — so centring against the *external* monitor's size and posting
/// it as a global position drops the pill into the dead space between screens,
/// where it is never visible.
///
/// The pill should follow the user, so pick the display the pointer is on
/// rather than always the main one, and fall back to main if that fails.
#[cfg(target_os = "macos")]
fn pill_position(_ctx: &egui::Context) -> Option<(f32, f32)> {
    use core_graphics::display::CGDisplay;

    let bounds = active_display_bounds().unwrap_or_else(|| CGDisplay::main().bounds());
    Some((
        bounds.origin.x as f32 + (bounds.size.width as f32 - W) / 2.0,
        bounds.origin.y as f32 + bounds.size.height as f32 - H - BOTTOM_GAP,
    ))
}

/// Bounds of the display containing the mouse pointer.
#[cfg(target_os = "macos")]
fn active_display_bounds() -> Option<core_graphics::geometry::CGRect> {
    use core_graphics::display::CGDisplay;
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState).ok()?;
    let point = CGEvent::new(source).ok()?.location();

    for id in CGDisplay::active_displays().ok()? {
        let b = CGDisplay::new(id).bounds();
        let inside = point.x >= b.origin.x
            && point.x < b.origin.x + b.size.width
            && point.y >= b.origin.y
            && point.y < b.origin.y + b.size.height;
        if inside {
            log::info!("overlay: pointer on display {id} at {:?}", b.origin);
            return Some(b);
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn pill_position(ctx: &egui::Context) -> Option<(f32, f32)> {
    ctx.input(|i| i.viewport().monitor_size)
        .map(|size| ((size.x - W) / 2.0, size.y - H - BOTTOM_GAP))
}

const STATE_LISTENING: u8 = 0;
const STATE_TRANSCRIBING: u8 = 1;
const STATE_DONE: u8 = 2;

pub fn run() -> anyhow::Result<()> {
    let state = Arc::new(AtomicU8::new(STATE_LISTENING));

    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([W, H])
            .with_decorations(false)
            .with_transparent(true)
            .with_window_level(egui::WindowLevel::AlwaysOnTop)
            .with_taskbar(false)
            .with_resizable(false)
            // never take focus: keystrokes must keep flowing to the app the
            // user is dictating into (focus steal = streamed text lost)
            .with_active(false)
            .with_mouse_passthrough(true)
            .with_window_type(egui::X11WindowType::Notification),
        ..Default::default()
    };

    // The pill must never take keyboard focus: the daemon streams text into
    // whatever the user is dictating into, and a focus steal sends every
    // streamed keystroke to the overlay instead, where it is silently dropped.
    //
    // `with_active(false)` above is not sufficient on macOS. The daemon spawns
    // this process with a bare fork/exec rather than through LaunchServices, so
    // the bundle's `LSUIElement` is never applied and the process starts as a
    // *regular* application — which activates itself on launch and becomes
    // frontmost. Setting the policy explicitly is what LaunchServices would
    // otherwise have done for us.
    #[cfg(target_os = "macos")]
    {
        options.event_loop_builder = Some(Box::new(|builder| {
            use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
            builder
                .with_activation_policy(ActivationPolicy::Accessory)
                .with_activate_ignoring_other_apps(false);
        }));
    }

    let st = state.clone();
    eframe::run_native(
        "WhisprCatch",
        options,
        Box::new(move |cc| {
            theme::install_fonts(&cc.egui_ctx);
            // stdin watcher: any line -> transcribing, EOF -> done
            let ctx = cc.egui_ctx.clone();
            let s = st.clone();
            std::thread::spawn(move || {
                let stdin = std::io::stdin();
                for line in stdin.lock().lines() {
                    if line.is_err() {
                        break;
                    }
                    s.store(STATE_TRANSCRIBING, Ordering::Relaxed);
                    ctx.request_repaint();
                }
                s.store(STATE_DONE, Ordering::Relaxed);
                ctx.request_repaint();
            });
            Ok(Box::new(Overlay {
                state: st,
                position_frames: 0,
                shot: crate::shot::Shot::from_env(),
            }) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| anyhow::anyhow!("overlay failed: {e}"))
}

struct Overlay {
    state: Arc<AtomicU8>,
    /// Re-assert the position for the first frames — some WMs override the
    /// first move request, leaving the pill wherever it was initially placed.
    position_frames: u32,
    shot: crate::shot::Shot,
}

impl eframe::App for Overlay {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0] // fully transparent backdrop
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.shot.tick(ctx);
        if self.state.load(Ordering::Relaxed) == STATE_DONE {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if self.position_frames < 10 {
            if let Some((x, y)) = pill_position(ctx) {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition([x, y].into()));
                if self.position_frames == 0 {
                    log::info!(
                        "overlay at global ({x:.0},{y:.0}); egui monitor_size {:?}",
                        ctx.input(|i| i.viewport().monitor_size)
                    );
                }
                self.position_frames += 1;
            } else if self.position_frames == 0 {
                log::warn!("overlay: display bounds unknown, using WM placement");
                self.position_frames = 10;
            }
        }

        let transcribing = self.state.load(Ordering::Relaxed) == STATE_TRANSCRIBING;
        let t = ctx.input(|i| i.time);

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let painter = ui.painter();
                // pill: zinc-950 at ~92%, subtle white ring
                painter.rect_filled(
                    rect,
                    H / 2.0,
                    egui::Color32::from_rgba_unmultiplied(9, 9, 11, 235),
                );
                painter.rect_stroke(
                    rect.shrink(0.5),
                    H / 2.0,
                    egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 45)),
                    egui::StrokeKind::Inside,
                );

                let cy = rect.center().y;

                if transcribing {
                    // amber spinner (1s linear) + label
                    let spin = egui::pos2(rect.left() + 22.0, cy);
                    let a0 = (t % 1.0) as f32 * std::f32::consts::TAU;
                    let pts: Vec<egui::Pos2> = (0..=20)
                        .map(|i| {
                            let a = a0 + i as f32 / 20.0 * std::f32::consts::TAU * 0.72;
                            spin + egui::vec2(a.cos() * 6.5, a.sin() * 6.5)
                        })
                        .collect();
                    painter.add(egui::Shape::line(
                        pts,
                        egui::Stroke::new(2.0, theme::AMBER),
                    ));
                    painter.text(
                        egui::pos2(rect.left() + 40.0, cy),
                        egui::Align2::LEFT_CENTER,
                        "Transcribing…",
                        theme::medium(13.0),
                        theme::FG,
                    );
                } else {
                    // 4-bar waveform
                    for k in 0..4 {
                        let phase = t * 6.3 + k as f64 * 1.7;
                        let h = 4.0 + 10.0 * phase.sin().abs() as f32;
                        let x = rect.left() + 22.0 + k as f32 * 7.0;
                        painter.rect_filled(
                            egui::Rect::from_center_size(egui::pos2(x, cy), egui::vec2(3.0, h)),
                            1.5,
                            theme::RED,
                        );
                    }
                    painter.text(
                        egui::pos2(rect.left() + 54.0, cy),
                        egui::Align2::LEFT_CENTER,
                        "Listening",
                        theme::medium(13.0),
                        theme::FG,
                    );
                }
            });

        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
