//! The floating pill: a small always-on-top capsule at the bottom centre of
//! the screen. Runs as its own long-lived process (`whisper-catch overlay`)
//! spawned once by the daemon and driven over stdin, one line per message:
//!
//! * `show`: the key went down; expand and start listening
//! * `l <rms>`: the current mic level, ~30 times a second while listening
//! * `t`: the key came up; show the transcribing state
//! * `hide`: back to the idle capsule
//! * EOF: the daemon is gone; quit
//!
//! Look per docs/DESIGN.md §B5: idle is a 40×9 black capsule with a grey
//! ring, always visible, so the user can see dictation is one key away.
//! Listening expands it to 112×32 with an orange LED and a live waveform;
//! transcribing swaps the waveform for three orange dots.

use std::collections::VecDeque;
use std::io::BufRead;
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32};

use crate::theme;

/// The window is sized to the expanded pill plus room for its ring; the
/// idle capsule is drawn inside it, bottom-centred, and everything else is
/// transparent and click-through.
const WIN_W: f32 = 132.0;
const WIN_H: f32 = 40.0;
const IDLE: egui::Vec2 = egui::vec2(40.0, 9.0);
const ACTIVE: egui::Vec2 = egui::vec2(112.0, 32.0);
/// Gap between the pill and the top of the Dock (or the bottom of the
/// screen when the Dock is hidden or on the side).
const GAP: f32 = 6.0;
/// Bars in the waveform.
const BARS: usize = 9;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Idle,
    Listening,
    Transcribing,
}

struct Shared {
    mode: Mode,
    /// Normalised levels, newest first.
    levels: VecDeque<f32>,
    quit: bool,
}

/// Maps an RMS level to 0..1 on a log scale: room tone sits near 0, normal
/// speech around 0.6 to 0.9.
fn normalise(rms: f32) -> f32 {
    ((rms.max(1e-5).log10() + 3.3) / 2.3).clamp(0.0, 1.0)
}

/// Dev-only: `WC_OVERLAY=idle|listening|transcribing` pins the pill in one
/// state with a synthetic waveform, for screenshots (with `WC_SHOT`).
fn forced_mode() -> Option<Mode> {
    match std::env::var("WC_OVERLAY").ok()?.as_str() {
        "idle" => Some(Mode::Idle),
        "listening" => Some(Mode::Listening),
        "transcribing" => Some(Mode::Transcribing),
        _ => None,
    }
}

pub fn run() -> anyhow::Result<()> {
    let forced = forced_mode();
    let shared = Arc::new(Mutex::new(Shared {
        mode: forced.unwrap_or(Mode::Idle),
        levels: VecDeque::with_capacity(BARS),
        quit: false,
    }));

    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WIN_W, WIN_H])
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
    // *regular* application, which activates itself on launch and becomes
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

    let st = shared.clone();
    eframe::run_native(
        "WhisprCatch Overlay",
        options,
        Box::new(move |cc| {
            theme::install_fonts(&cc.egui_ctx);
            if forced.is_none() {
                let ctx = cc.egui_ctx.clone();
                let s = st.clone();
                std::thread::spawn(move || read_commands(s, ctx));
            }
            Ok(Box::new(Overlay {
                shared: st,
                forced: forced.is_some(),
                bars: [0.0; BARS],
                placed_at: None,
                next_place_check: 0.0,
                configured: false,
                shot: crate::shot::Shot::from_env(),
            }) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| anyhow::anyhow!("overlay failed: {e}"))
}

fn read_commands(s: Arc<Mutex<Shared>>, ctx: egui::Context) {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let mut g = s.lock().unwrap();
        match line.trim() {
            "show" => {
                g.mode = Mode::Listening;
                g.levels.clear();
            }
            "t" => g.mode = Mode::Transcribing,
            "hide" => g.mode = Mode::Idle,
            l if l.starts_with("l ") => {
                if let Ok(rms) = l[2..].trim().parse::<f32>() {
                    g.levels.push_front(normalise(rms));
                    g.levels.truncate(BARS);
                }
            }
            _ => {}
        }
        drop(g);
        ctx.request_repaint();
    }
    s.lock().unwrap().quit = true;
    ctx.request_repaint();
}

struct Overlay {
    shared: Arc<Mutex<Shared>>,
    forced: bool,
    /// Displayed bar heights (0..1), eased toward their targets every frame.
    bars: [f32; BARS],
    /// Where the window was last moved to, so it is only moved on a change.
    placed_at: Option<(f32, f32)>,
    /// `ctx` time of the next "which display is the pointer on?" check.
    next_place_check: f64,
    /// macOS window behaviour applied (all Spaces, above full-screen apps).
    configured: bool,
    shot: crate::shot::Shot,
}

impl eframe::App for Overlay {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0] // fully transparent backdrop
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.shot.tick(ctx);
        let (mode, levels, quit) = {
            let g = self.shared.lock().unwrap();
            (g.mode, g.levels.clone(), g.quit)
        };
        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let now = ctx.input(|i| i.time);

        #[cfg(target_os = "macos")]
        if !self.configured {
            self.configured = mac::configure_window();
        }

        // Follow the pointer's display, and the Dock: checked twice a second,
        // and on every key-down so the pill opens where the user is looking.
        let check_now = now >= self.next_place_check
            || (mode == Mode::Listening && self.bars.iter().all(|b| *b == 0.0));
        if check_now {
            self.next_place_check = now + 0.5;
            if let Some(pos) = target_position(ctx) {
                if self.placed_at != Some(pos) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos.into()));
                    log::debug!("overlay placed at ({:.0},{:.0})", pos.0, pos.1);
                    self.placed_at = Some(pos);
                }
            }
        }

        // expansion: 0 = idle capsule, 1 = full pill
        let open = ctx.animate_bool_with_time_and_easing(
            egui::Id::new("pill-open"),
            mode != Mode::Idle,
            0.22,
            egui::emath::easing::cubic_out,
        );

        // waveform targets: newest level in the centre, older ones rippling out
        let synthetic = self.forced && mode == Mode::Listening;
        for (i, bar) in self.bars.iter_mut().enumerate() {
            let k = (i as i32 - (BARS as i32 / 2)).unsigned_abs() as usize;
            let raw = if synthetic {
                (0.55 + 0.45 * ((now * 5.0 + k as f64 * 0.9).sin() as f32)).abs()
            } else {
                levels.get(k).copied().unwrap_or(0.0)
            };
            let target = if mode == Mode::Listening {
                raw * (1.0 - 0.11 * k as f32)
            } else {
                0.0
            };
            *bar += (target - *bar) * 0.35;
            if bar.abs() < 0.002 {
                *bar = 0.0;
            }
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| paint(ui, mode, open, &self.bars, now));

        let animating = open > 0.0 && open < 1.0;
        if mode != Mode::Idle || animating || self.bars.iter().any(|b| *b > 0.0) {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let (a, b) = (egui::Rgba::from(a), egui::Rgba::from(b));
    Color32::from(a * (1.0 - t) + b * t)
}

fn paint(ui: &mut egui::Ui, mode: Mode, open: f32, bars: &[f32; BARS], now: f64) {
    let win = ui.max_rect();
    let size = IDLE + (ACTIVE - IDLE) * open;
    let rect = egui::Rect::from_center_size(
        egui::pos2(win.center().x, win.bottom() - 4.0 - size.y / 2.0),
        size,
    );
    let r = size.y / 2.0;
    let p = ui.painter();

    // idle: near-black with a light grey ring, readable on any wallpaper;
    // open: deeper black with a faint ring, so the content carries it
    let fill = mix(
        Color32::from_rgba_unmultiplied(16, 16, 16, 220),
        Color32::from_rgba_unmultiplied(8, 8, 8, 242),
        open,
    );
    let ring = mix(
        Color32::from_rgba_unmultiplied(190, 190, 190, 150),
        Color32::from_rgba_unmultiplied(255, 255, 255, 34),
        open,
    );
    p.rect_filled(rect, r, fill);
    p.rect_stroke(
        rect,
        r,
        egui::Stroke::new(1.0 + 0.25 * (1.0 - open), ring),
        egui::StrokeKind::Inside,
    );

    // contents fade in over the second half of the expansion
    let a = ((open - 0.5) * 2.0).clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let cy = rect.center().y;
    match mode {
        Mode::Listening | Mode::Idle => {
            // recording LED, 2s pulse
            let pulse = 0.55 + 0.45 * (0.5 + 0.5 * (now * std::f64::consts::PI).cos() as f32);
            let led = egui::pos2(rect.left() + 16.0, cy);
            p.circle_filled(led, 6.0, theme::ACCENT.gamma_multiply(0.22 * a * pulse));
            p.circle_filled(led, 3.5, theme::ACCENT.gamma_multiply(a * pulse));

            // waveform, centred in the space right of the LED
            let (w, gap) = (3.0, 4.0);
            let total = BARS as f32 * w + (BARS - 1) as f32 * gap;
            let x0 = rect.left() + 30.0 + ((rect.width() - 30.0 - 14.0) - total) / 2.0;
            for (i, v) in bars.iter().enumerate() {
                let h = 3.0 + 17.0 * v;
                let x = x0 + i as f32 * (w + gap) + w / 2.0;
                p.rect_filled(
                    egui::Rect::from_center_size(egui::pos2(x, cy), egui::vec2(w, h)),
                    w / 2.0,
                    theme::FG.gamma_multiply(a * (0.55 + 0.45 * v.min(1.0))),
                );
            }
        }
        Mode::Transcribing => {
            // three orange dots, pulsing in sequence
            for k in 0..3 {
                let phase = ((now * 2.4 - k as f64 * 0.22).rem_euclid(1.0)) as f32;
                let lift = (phase * std::f32::consts::TAU).sin().max(0.0);
                let c = egui::pos2(rect.center().x + (k as f32 - 1.0) * 12.0, cy - 2.5 * lift);
                p.circle_filled(
                    c,
                    3.2,
                    theme::ACCENT.gamma_multiply(a * (0.45 + 0.55 * lift)),
                );
            }
        }
    }
}

/// Top-left of the overlay window in the global desktop coordinates that
/// `OuterPosition` takes.
///
/// macOS: bottom-centre of the *visible* frame of the display the pointer is
/// on, so the pill sits just above the Dock and follows the user across
/// displays. egui alone gets this wrong twice: it reports a monitor's size but
/// not its origin (a second display routinely sits at a negative origin), and
/// it knows nothing about the Dock.
#[cfg(target_os = "macos")]
fn target_position(_ctx: &egui::Context) -> Option<(f32, f32)> {
    mac::target_position()
}

#[cfg(not(target_os = "macos"))]
fn target_position(ctx: &egui::Context) -> Option<(f32, f32)> {
    ctx.input(|i| i.viewport().monitor_size)
        .map(|size| ((size.x - WIN_W) / 2.0, size.y - WIN_H - GAP - 40.0))
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2_app_kit::{NSApplication, NSEvent, NSScreen, NSWindowCollectionBehavior};
    use objc2_foundation::MainThreadMarker;

    use super::{GAP, WIN_H, WIN_W};

    pub fn target_position() -> Option<(f32, f32)> {
        let mtm = MainThreadMarker::new()?;
        let screens = NSScreen::screens(mtm);
        // Cocoa's global space has its origin at the bottom-left of the
        // *first* screen (the one with the menu bar); winit's is top-left.
        let primary = screens.firstObject()?;
        let primary_h = primary.frame().size.height;
        let mouse = NSEvent::mouseLocation();
        let screen = screens
            .iter()
            .find(|s| {
                let f = s.frame();
                mouse.x >= f.origin.x
                    && mouse.x < f.origin.x + f.size.width
                    && mouse.y >= f.origin.y
                    && mouse.y < f.origin.y + f.size.height
            })
            .unwrap_or(primary);
        let vf = screen.visibleFrame();
        let x = vf.origin.x + (vf.size.width - WIN_W as f64) / 2.0;
        let top = primary_h - (vf.origin.y + GAP as f64 + WIN_H as f64);
        Some((x.round() as f32, top.round() as f32))
    }

    /// Puts the pill on every Space and over full-screen apps, above the
    /// Dock's level, without a shadow (the transparent window's shadow would
    /// outline the whole rectangle, not the capsule). Returns false until the
    /// window exists.
    pub fn configure_window() -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let app = NSApplication::sharedApplication(mtm);
        let windows = app.windows();
        if windows.is_empty() {
            return false;
        }
        for w in windows.iter() {
            w.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::Stationary
                    | NSWindowCollectionBehavior::FullScreenAuxiliary
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );
            // NSStatusWindowLevel: above the Dock and full-screen apps.
            w.setLevel(25);
            w.setHasShadow(false);
            w.setIgnoresMouseEvents(true);
        }
        log::info!("overlay: joined all Spaces");
        true
    }
}

#[cfg(test)]
mod tests {
    use super::normalise;

    #[test]
    fn levels_map_into_the_unit_range_and_keep_their_order() {
        let samples = [0.0, 1e-6, 1e-4, 1e-3, 0.01, 0.05, 0.1, 0.5, 1.0, 4.0];
        let mapped: Vec<f32> = samples.iter().map(|s| normalise(*s)).collect();
        assert!(mapped.iter().all(|v| (0.0..=1.0).contains(v)), "{mapped:?}");
        assert!(mapped.windows(2).all(|w| w[0] <= w[1]), "{mapped:?}");
    }

    /// Room tone must read as a flat line and speech as a clear movement,
    /// or the waveform tells the user nothing.
    #[test]
    fn silence_is_flat_and_speech_moves() {
        assert!(normalise(0.0003) < 0.1);
        assert!(normalise(0.05) > 0.6);
    }
}
