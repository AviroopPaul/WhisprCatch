//! The floating pill: a small always-on-top capsule at the bottom centre of
//! the screen. Runs as its own long-lived process (`whisper-catch overlay`)
//! spawned once by the daemon and driven over stdin, one line per message:
//!
//! * `show`: dictation started; expand and start listening
//! * `l <rms>`: the current mic level, ~30 times a second while listening
//! * `t`: dictation ended; show the transcribing state
//! * `hide`: back to the idle capsule
//! * EOF: the daemon is gone; quit
//!
//! and writes `toggle` on stdout when the user clicks to start or finish a
//! dictation.
//!
//! Look per docs/DESIGN.md §B5: idle is a 40×9 black capsule with a grey
//! ring, always visible. Under the pointer it opens into hover controls, a
//! mic button (click: dictate hands-free) and a gear (Settings), with a label
//! above whichever is hovered. Listening expands it to 92×30 with a live
//! waveform; transcribing swaps the waveform for three orange dots.

use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32};
use egui_phosphor::regular as icons;

use crate::theme;

/// The window holds the pill, the hover controls and their label above.
/// Everything outside what is drawn is transparent, and the window itself is
/// click-through; the clickable part is a separate native panel (`mac::Hit`).
const WIN_W: f32 = 248.0;
const WIN_H: f32 = 96.0;
const IDLE: egui::Vec2 = egui::vec2(40.0, 9.0);
const ACTIVE: egui::Vec2 = egui::vec2(92.0, 30.0);
/// The capsule as a mic button, under the pointer.
const MIC: egui::Vec2 = egui::vec2(72.0, 32.0);
/// The gear button beside it.
const GEAR_D: f32 = 32.0;
const GEAR_GAP: f32 = 6.0;
/// Gap between the pill and the top of the Dock (or the bottom of the
/// screen when the Dock is hidden or on the side).
const GAP: f32 = 6.0;
/// Bars in the waveform.
const BARS: usize = 9;

/// Centre line of the pill, in window points (top-left origin).
fn pill_centre_y(height: f32) -> f32 {
    WIN_H - 4.0 - height / 2.0
}

fn mic_rect() -> egui::Rect {
    egui::Rect::from_center_size(egui::pos2(WIN_W / 2.0, pill_centre_y(MIC.y)), MIC)
}

/// The round button `slot` places to the right of the mic (0 is nearest).
fn side_button_rect(slot: usize) -> egui::Rect {
    let mic = mic_rect();
    let step = GEAR_D + GEAR_GAP;
    egui::Rect::from_center_size(
        egui::pos2(
            mic.right() + GEAR_GAP + GEAR_D / 2.0 + step * slot as f32,
            mic.center().y,
        ),
        egui::vec2(GEAR_D, GEAR_D),
    )
}

/// The Notes button, between the mic and the gear. Only exists when the
/// Catcher's Notes button is on.
fn notes_rect() -> egui::Rect {
    side_button_rect(0)
}

/// The gear moves out one slot when the Notes button takes the first.
fn gear_rect(notes: bool) -> egui::Rect {
    side_button_rect(usize::from(notes))
}

fn active_rect() -> egui::Rect {
    egui::Rect::from_center_size(egui::pos2(WIN_W / 2.0, pill_centre_y(ACTIVE.y)), ACTIVE)
}

/// What the native click target covers in each state, in window points.
/// Idle: a 64×22 patch over the tiny capsule, so the rest stays click-through.
/// Hovered: the mic, the Notes button when on, and the gear. Listening: the pill, which a click finishes.
fn target_rect(state: Target, notes: bool) -> egui::Rect {
    match state {
        Target::Idle => egui::Rect::from_min_max(
            egui::pos2(WIN_W / 2.0 - 32.0, WIN_H - 22.0),
            egui::pos2(WIN_W / 2.0 + 32.0, WIN_H),
        ),
        Target::Controls => egui::Rect::from_min_max(
            egui::pos2(mic_rect().left() - 6.0, mic_rect().top() - 6.0),
            egui::pos2(gear_rect(notes).right() + 6.0, WIN_H),
        ),
        Target::Pill => active_rect().expand(4.0),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Idle,
    Controls,
    Pill,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Idle,
    Listening,
    Transcribing,
}

/// What is under the pointer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Zone {
    None,
    Mic,
    Notes,
    Gear,
    /// The listening pill.
    Pill,
}

struct Shared {
    mode: Mode,
    /// Normalised levels, newest first.
    levels: VecDeque<f32>,
    quit: bool,
}

/// Pointer events from the native click target, read every frame.
#[derive(Default)]
pub struct PillUi {
    /// The pointer is over the target (entered and not yet left).
    pub inside: AtomicBool,
    /// A click landed on the target since the last frame.
    pub clicked: AtomicBool,
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

/// Dev-only: `WC_OVERLAY_HOVER=mic|notes|gear|pill` puts the pointer there.
/// `notes` also turns the Notes button on, whatever the config says.
fn forced_zone() -> Option<Zone> {
    match std::env::var("WC_OVERLAY_HOVER").ok()?.as_str() {
        "mic" | "1" => Some(Zone::Mic),
        "notes" => Some(Zone::Notes),
        "gear" => Some(Zone::Gear),
        "pill" => Some(Zone::Pill),
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

    let key = crate::config::load()
        .map(|c| crate::config::key_label(&c.key).to_string())
        .unwrap_or_else(|_| "fn".into());

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
                forced_zone: forced_zone(),
                notes: forced_zone() == Some(Zone::Notes) || notes_on(),
                cfg_mtime: crate::config::modified(),
                next_cfg_check: 0.0,
                bars: [0.0; BARS],
                placed_at: None,
                next_place_check: 0.0,
                configured: false,
                ui_state: Arc::new(PillUi::default()),
                key,
                zone: Zone::None,
                tip: Zone::None,
                #[cfg(target_os = "macos")]
                hit: None,
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

/// Asks the daemon to start (or finish) a dictation, as the hotkey would.
fn send_toggle() {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "toggle");
    let _ = out.flush();
}

struct Overlay {
    shared: Arc<Mutex<Shared>>,
    forced: bool,
    forced_zone: Option<Zone>,
    /// Show the Notes button on hover (`catcher_notes` in the config).
    notes: bool,
    /// `config.toml`'s mtime when `notes` was last read.
    cfg_mtime: Option<std::time::SystemTime>,
    /// `ctx` time of the next look at that mtime.
    next_cfg_check: f64,
    /// Displayed bar heights (0..1), eased toward their targets every frame.
    bars: [f32; BARS],
    /// Where the window was last moved to, so it is only moved on a change.
    placed_at: Option<(f32, f32)>,
    /// `ctx` time of the next "which display is the pointer on?" check.
    next_place_check: f64,
    /// macOS window behaviour applied (all Spaces, above full-screen apps).
    configured: bool,
    ui_state: Arc<PillUi>,
    /// The hotkey's label, for "Dictate fn".
    key: String,
    /// What the pointer is over this frame.
    zone: Zone,
    /// The last zone that had a label, kept while the label fades out.
    tip: Zone,
    /// The native click target (macOS), created once the window exists.
    #[cfg(target_os = "macos")]
    hit: Option<mac::Hit>,
    shot: crate::shot::Shot,
}

impl Overlay {
    /// The pointer in window points, if it is over the window.
    fn pointer(&self) -> Option<egui::Pos2> {
        #[cfg(target_os = "macos")]
        return self.placed_at.and_then(mac::pointer_in_window);
        #[cfg(not(target_os = "macos"))]
        None
    }

    fn zone_at(mode: Mode, notes: bool, p: egui::Pos2) -> Zone {
        match mode {
            Mode::Idle if gear_rect(notes).expand(3.0).contains(p) => Zone::Gear,
            Mode::Idle if notes && notes_rect().expand(3.0).contains(p) => Zone::Notes,
            Mode::Idle if target_rect(Target::Controls, notes).contains(p) => Zone::Mic,
            Mode::Listening if target_rect(Target::Pill, notes).contains(p) => Zone::Pill,
            _ => Zone::None,
        }
    }
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

        // The Catcher outlives Settings, so pick up `catcher_notes` from the
        // config file: a stat every 2s, a parse only when the mtime moved.
        if !self.forced && now >= self.next_cfg_check {
            self.next_cfg_check = now + 2.0;
            let mtime = crate::config::modified();
            if mtime != self.cfg_mtime {
                self.cfg_mtime = mtime;
                self.notes = notes_on();
            }
        }

        #[cfg(target_os = "macos")]
        if !self.configured {
            self.configured = mac::configure_window();
            if self.configured && !self.forced {
                self.hit = mac::Hit::new(ctx.clone(), self.ui_state.clone());
                if let (Some(hit), Some(pos)) = (&self.hit, self.placed_at) {
                    hit.place(pos);
                }
            }
        }

        // What is the pointer over? Entering the target wakes us; from then
        // on the pointer is read every frame, so the zone and the moment it
        // leaves are exact even if an exit event is missed.
        let inside = self.ui_state.inside.load(Ordering::Relaxed);
        self.zone = match self.forced_zone {
            Some(z) => z,
            None if inside => self
                .pointer()
                .map(|p| Self::zone_at(mode, self.notes, p))
                .unwrap_or(Zone::None),
            None => Zone::None,
        };
        if inside && self.zone == Zone::None && self.forced_zone.is_none() {
            self.ui_state.inside.store(false, Ordering::Relaxed);
        }

        // clicks: mic starts hands-free, gear opens Settings, the listening
        // pill finishes
        if self.ui_state.clicked.swap(false, Ordering::Relaxed) {
            let at = self.pointer().map(|p| Self::zone_at(mode, self.notes, p));
            match (mode, at) {
                (Mode::Idle, Some(Zone::Gear)) => open_settings(),
                (Mode::Idle, Some(Zone::Notes)) => open_note(),
                (Mode::Idle, _) => send_toggle(),
                (Mode::Listening, _) => send_toggle(),
                (Mode::Transcribing, _) => {}
            }
            log::debug!("pill click: {mode:?} at {at:?}");
        }

        // Follow the pointer's display, and the Dock: checked twice a second,
        // and on every key-down so the pill opens where the user is looking.
        // Not while hovered, so the controls cannot slide away mid-click.
        let check_now = (self.zone == Zone::None || self.placed_at.is_none())
            && (now >= self.next_place_check
                || (mode == Mode::Listening && self.bars.iter().all(|b| *b == 0.0)));
        if check_now {
            self.next_place_check = now + 0.5;
            if let Some(pos) = target_position(ctx) {
                if self.placed_at != Some(pos) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos.into()));
                    log::debug!("overlay placed at ({:.0},{:.0})", pos.0, pos.1);
                    self.placed_at = Some(pos);
                    #[cfg(target_os = "macos")]
                    if let Some(hit) = &self.hit {
                        hit.place(pos);
                    }
                }
            }
        }

        // the click target follows the state: tiny when idle, the controls
        // when hovered, the pill while listening
        #[cfg(target_os = "macos")]
        if let Some(hit) = &self.hit {
            hit.set_notes(self.notes);
            hit.cover(match mode {
                Mode::Idle if self.zone != Zone::None => Some(Target::Controls),
                Mode::Idle => Some(Target::Idle),
                Mode::Listening => Some(Target::Pill),
                Mode::Transcribing => None,
            });
        }

        let controls = ctx.animate_bool_with_time_and_easing(
            egui::Id::new("pill-controls"),
            mode == Mode::Idle && self.zone != Zone::None,
            0.18,
            egui::emath::easing::cubic_out,
        );
        let mic_hot =
            ctx.animate_bool_with_time(egui::Id::new("pill-mic"), self.zone == Zone::Mic, 0.12);
        let gear_hot =
            ctx.animate_bool_with_time(egui::Id::new("pill-gear"), self.zone == Zone::Gear, 0.12);
        let notes_hot =
            ctx.animate_bool_with_time(egui::Id::new("pill-notes"), self.zone == Zone::Notes, 0.12);
        if self.zone != Zone::None {
            self.tip = self.zone;
        }
        let tip =
            ctx.animate_bool_with_time(egui::Id::new("pill-tip"), self.zone != Zone::None, 0.14);

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

        let label = match self.tip {
            Zone::Mic => Some(("Dictate", Some(self.key.as_str()))),
            Zone::Notes => Some(("New note", None)),
            Zone::Gear => Some(("Settings", None)),
            Zone::Pill => Some(("Click to finish", None)),
            Zone::None => None,
        };
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let origin = ui.max_rect().min.to_vec2();
                paint_pill(ui, mode, open, controls, mic_hot, &self.bars, now);
                if controls > 0.0 {
                    if self.notes {
                        paint_side_button(ui, notes_rect(), origin, controls, notes_hot, icons::NOTE_PENCIL);
                    }
                    paint_side_button(ui, gear_rect(self.notes), origin, controls, gear_hot, icons::GEAR_SIX);
                }
                if let (Some((text, key)), true) = (label, tip > 0.0) {
                    let over = match self.tip {
                        Zone::Gear => gear_rect(self.notes),
                        Zone::Notes => notes_rect(),
                        Zone::Pill => active_rect(),
                        _ => mic_rect(),
                    };
                    paint_label(ui, over.translate(origin), tip, text, key);
                }
            });

        let animating = [open, controls, mic_hot, notes_hot, gear_hot, tip]
            .iter()
            .any(|v| *v > 0.0 && *v < 1.0);
        if mode != Mode::Idle
            || animating
            || self.zone != Zone::None
            || self.bars.iter().any(|b| *b > 0.0)
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}

/// Whether the config asks for the Notes button. Unreadable means off.
fn notes_on() -> bool {
    crate::config::load().map(|c| c.catcher_notes).unwrap_or(false)
}

/// Opens the main window on its Settings page. The window brings itself to
/// the front on its first frame (settings_app).
fn open_settings() {
    spawn_self(&["settings", "--tab", "settings"], "Settings");
}

/// Opens the quick note window (single instance, brings itself to the front).
fn open_note() {
    spawn_self(&["note"], "a note");
}

fn spawn_self(args: &[&str], what: &str) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    match std::process::Command::new(exe)
        .args(args)
        // our stdout is the daemon's command pipe; keep the child off it
        .stdout(std::process::Stdio::null())
        .spawn()
    {
        // reap it when it closes; this process lives for the whole session
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => log::warn!("could not open {what}: {e}"),
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let (a, b) = (egui::Rgba::from(a), egui::Rgba::from(b));
    Color32::from(a * (1.0 - t) + b * t)
}

/// Near-black fill shared by the open pill, the controls and the label.
const CHROME: Color32 = Color32::from_rgba_premultiplied(8, 8, 8, 242);

/// The capsule: idle, morphing into the mic button (`controls`), or open for
/// a dictation (`open`).
fn paint_pill(
    ui: &mut egui::Ui,
    mode: Mode,
    open: f32,
    controls: f32,
    mic_hot: f32,
    bars: &[f32; BARS],
    now: f64,
) {
    let win = ui.max_rect();
    let idle = IDLE + (MIC - IDLE) * controls;
    let size = idle + (ACTIVE - idle) * open;
    let rect = egui::Rect::from_center_size(
        egui::pos2(win.center().x, win.min.y + pill_centre_y(size.y)),
        size,
    );
    let r = size.y / 2.0;
    let p = ui.painter();

    // idle: near-black with a light grey ring, readable on any wallpaper;
    // as a button or open: deeper black with a faint ring, the content leads
    let lift = controls.max(open);
    let fill = mix(
        Color32::from_rgba_unmultiplied(16, 16, 16, 220),
        CHROME,
        lift,
    );
    let fill = mix(
        fill,
        Color32::from_rgba_unmultiplied(30, 30, 30, 245),
        mic_hot * controls,
    );
    let ring = mix(
        Color32::from_rgba_unmultiplied(190, 190, 190, 150),
        Color32::from_rgba_unmultiplied(255, 255, 255, 38),
        lift,
    );
    let ring = mix(
        ring,
        Color32::from_rgba_unmultiplied(255, 255, 255, 80),
        mic_hot * controls,
    );
    p.rect_filled(rect, r, fill);
    p.rect_stroke(
        rect,
        r,
        egui::Stroke::new(1.0 + 0.25 * (1.0 - lift), ring),
        egui::StrokeKind::Inside,
    );

    // the mic glyph, while the capsule is a button
    let c = ((controls - 0.4) / 0.6).clamp(0.0, 1.0) * (1.0 - open);
    if c > 0.0 {
        p.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            icons::MICROPHONE,
            egui::FontId::proportional(17.0),
            mix(theme::FG, theme::ACCENT, mic_hot).gamma_multiply(c),
        );
    }

    // dictation contents fade in over the second half of the expansion
    let a = ((open - 0.5) * 2.0).clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let cy = rect.center().y;
    match mode {
        Mode::Listening | Mode::Idle => {
            let (w, gap) = (3.0, 4.0);
            let total = BARS as f32 * w + (BARS - 1) as f32 * gap;
            let x0 = rect.center().x - total / 2.0;
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

/// A round button beside the mic (Notes, Settings), scaling and fading in
/// with the controls.
fn paint_side_button(
    ui: &mut egui::Ui,
    at: egui::Rect,
    origin: egui::Vec2,
    t: f32,
    hot: f32,
    icon: &str,
) {
    let full = at.translate(origin);
    let rect = egui::Rect::from_center_size(full.center(), full.size() * (0.6 + 0.4 * t));
    let r = rect.width() / 2.0;
    let p = ui.painter();
    let fill = mix(
        CHROME,
        Color32::from_rgba_unmultiplied(30, 30, 30, 245),
        hot,
    );
    p.circle_filled(rect.center(), r, fill.gamma_multiply(t));
    p.circle_stroke(
        rect.center(),
        r - 0.5,
        egui::Stroke::new(
            1.0,
            mix(
                Color32::from_rgba_unmultiplied(255, 255, 255, 38),
                Color32::from_rgba_unmultiplied(255, 255, 255, 80),
                hot,
            )
            .gamma_multiply(t),
        ),
    );
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(16.0 * (0.6 + 0.4 * t)),
        mix(theme::TEXT_2, theme::ACCENT, hot).gamma_multiply(t),
    );
}

/// The label above a hovered control: "Dictate fn", "Settings", "Click to
/// finish". A pill of `CHROME` with the key in SemiBold, rising 4pt in.
fn paint_label(ui: &mut egui::Ui, over: egui::Rect, t: f32, text: &str, key: Option<&str>) {
    let p = ui.painter();
    let body = ui.fonts(|f| f.layout_no_wrap(text.into(), theme::medium(13.0), theme::FG));
    let key_g =
        key.map(|k| ui.fonts(|f| f.layout_no_wrap(k.into(), theme::semibold(13.0), theme::FG)));
    let gap = 6.0;
    let w = body.size().x + key_g.as_ref().map(|g| gap + g.size().x).unwrap_or(0.0) + 28.0;
    let h = 28.0;
    // Centred over its button, but kept inside the window: the gear sits near
    // the right edge once the Notes button is on, and the window clips.
    let bounds = ui.clip_rect().shrink(4.0);
    let x = (over.center().x - w / 2.0).clamp(bounds.left(), (bounds.right() - w).max(bounds.left()));
    let rect = egui::Rect::from_min_size(
        egui::pos2(x, over.top() - 8.0 - h + 4.0 * (1.0 - t)),
        egui::vec2(w, h),
    );
    p.rect_filled(rect, h / 2.0, CHROME.gamma_multiply(t));
    p.rect_stroke(
        rect,
        h / 2.0,
        egui::Stroke::new(
            1.0,
            Color32::from_rgba_unmultiplied(255, 255, 255, 38).gamma_multiply(t),
        ),
        egui::StrokeKind::Inside,
    );
    let mut x = rect.left() + 14.0;
    let cy = rect.center().y;
    p.galley(
        egui::pos2(x, cy - body.size().y / 2.0),
        body.clone(),
        theme::FG.gamma_multiply(t),
    );
    x += body.size().x + gap;
    if let Some(g) = key_g {
        p.galley(
            egui::pos2(x, cy - g.size().y / 2.0),
            g,
            theme::FG.gamma_multiply(t),
        );
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
    use std::cell::Cell;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    use eframe::egui;
    use objc2::rc::Retained;
    use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSBackingStoreType, NSColor, NSEvent, NSPanel, NSScreen, NSTrackingArea,
        NSTrackingAreaOptions, NSView, NSWindowCollectionBehavior, NSWindowStyleMask,
    };
    use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};

    use super::{target_rect, PillUi, Target, GAP, WIN_H, WIN_W};

    pub struct TargetIvars {
        ctx: egui::Context,
        ui: Arc<PillUi>,
    }

    define_class!(
        // A plain view that takes the clicks the (click-through) egui window
        // cannot. It lives in a non-activating panel, so clicking it never
        // takes focus from the app the user is typing in. It only reports
        // enter / leave / click; the overlay works out what was hit.
        #[unsafe(super(NSView))]
        #[thread_kind = MainThreadOnly]
        #[name = "WCPillTarget"]
        #[ivars = TargetIvars]
        struct PillTarget;

        impl PillTarget {
            #[unsafe(method(mouseDown:))]
            fn mouse_down(&self, _event: &NSEvent) {
                self.ivars().ui.clicked.store(true, Ordering::Relaxed);
                self.wake(true);
            }

            #[unsafe(method(acceptsFirstMouse:))]
            fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
                true
            }

            #[unsafe(method(mouseEntered:))]
            fn mouse_entered(&self, _event: &NSEvent) {
                self.wake(true);
            }

            #[unsafe(method(mouseExited:))]
            fn mouse_exited(&self, _event: &NSEvent) {
                self.wake(false);
            }
        }
    );

    impl PillTarget {
        fn new(
            mtm: MainThreadMarker,
            frame: NSRect,
            ctx: egui::Context,
            ui: Arc<PillUi>,
        ) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(TargetIvars { ctx, ui });
            let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    frame,
                    NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::ActiveAlways
                        | NSTrackingAreaOptions::InVisibleRect,
                    Some(&view),
                    None,
                )
            };
            view.addTrackingArea(&area);
            view
        }

        fn wake(&self, inside: bool) {
            let iv = self.ivars();
            iv.ui.inside.store(inside, Ordering::Relaxed);
            iv.ctx.request_repaint();
        }
    }

    pub struct Hit {
        panel: Retained<NSPanel>,
        /// Top-left of the pill window (winit coordinates), once placed.
        win: Cell<Option<(f32, f32)>>,
        covering: Cell<Option<Target>>,
        /// The Notes button is on, so the hovered controls are wider.
        notes: Cell<bool>,
    }

    impl Hit {
        /// A borderless, clear, non-activating panel on every Space, one
        /// level above the pill window (25), that takes clicks even where it
        /// is clear.
        pub fn new(ctx: egui::Context, ui: Arc<PillUi>) -> Option<Self> {
            let mtm = MainThreadMarker::new()?;
            let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(64.0, 22.0));
            let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
                NSPanel::alloc(mtm),
                frame,
                NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                NSBackingStoreType::Buffered,
                false,
            );
            panel.setOpaque(false);
            panel.setBackgroundColor(Some(&NSColor::clearColor()));
            panel.setHasShadow(false);
            panel.setHidesOnDeactivate(false);
            panel.setLevel(26);
            panel.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::Stationary
                    | NSWindowCollectionBehavior::FullScreenAuxiliary
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );
            // explicitly false: a clear window then still receives clicks in
            // its transparent area instead of passing them through
            panel.setIgnoresMouseEvents(false);
            unsafe { panel.setReleasedWhenClosed(false) };
            let view = PillTarget::new(mtm, frame, ctx, ui);
            panel.setContentView(Some(&view));
            Some(Self {
                panel,
                win: Cell::new(None),
                covering: Cell::new(None),
                notes: Cell::new(false),
            })
        }

        /// Widens or narrows the hovered controls' target to match.
        pub fn set_notes(&self, on: bool) {
            if self.notes.replace(on) != on {
                if let Some(t) = self.covering.take() {
                    self.cover(Some(t));
                }
            }
        }

        pub fn place(&self, win: (f32, f32)) {
            self.win.set(Some(win));
            if let Some(t) = self.covering.take() {
                self.cover(Some(t));
            }
        }

        /// Moves the target over `what` (window points), or hides it.
        pub fn cover(&self, what: Option<Target>) {
            if what == self.covering.get() {
                return;
            }
            let (Some(target), Some(win)) = (what, self.win.get()) else {
                self.panel.orderOut(None);
                self.covering.set(None);
                return;
            };
            let Some(primary_h) = primary_height() else {
                return;
            };
            let r = target_rect(target, self.notes.get());
            // window points (top-left origin) -> Cocoa screen points
            let bottom = primary_h - win.1 as f64 - WIN_H as f64;
            let frame = NSRect::new(
                NSPoint::new(
                    win.0 as f64 + r.left() as f64,
                    bottom + (WIN_H - r.bottom()) as f64,
                ),
                NSSize::new(r.width() as f64, r.height() as f64),
            );
            self.panel.setFrame_display(frame, false);
            self.panel.orderFrontRegardless();
            self.covering.set(Some(target));
        }
    }

    fn primary_height() -> Option<f64> {
        let mtm = MainThreadMarker::new()?;
        Some(NSScreen::screens(mtm).firstObject()?.frame().size.height)
    }

    /// The pointer in the pill window's points (top-left origin), for a
    /// window whose top-left is `win`, or None if it is outside the window.
    pub fn pointer_in_window(win: (f32, f32)) -> Option<egui::Pos2> {
        let m = NSEvent::mouseLocation();
        let primary_h = primary_height()?;
        let p = egui::pos2(
            (m.x - win.0 as f64) as f32,
            ((primary_h - m.y) - win.1 as f64) as f32,
        );
        let inside = (0.0..=WIN_W).contains(&p.x) && (0.0..=WIN_H).contains(&p.y);
        inside.then_some(p)
    }

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
    use super::*;

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

    /// Every drawn part and every click target must sit inside the window:
    /// anything outside is clipped, or clickable where nothing shows.
    #[test]
    fn controls_and_targets_fit_the_window() {
        let win = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(WIN_W, WIN_H));
        for notes in [false, true] {
            for r in [
                mic_rect(),
                gear_rect(notes),
                active_rect(),
                target_rect(Target::Idle, notes),
                target_rect(Target::Controls, notes),
                target_rect(Target::Pill, notes),
            ] {
                assert!(win.contains_rect(r), "{r:?} outside {win:?} (notes {notes})");
            }
        }
        assert!(win.contains_rect(notes_rect()));
        // a label (28 tall, 8 above its control) fits above the mic and the pill
        for r in [mic_rect(), active_rect()] {
            assert!(
                r.top() - 8.0 - 28.0 >= 0.0,
                "no room for a label above {r:?}"
            );
        }
    }

    /// The hover controls must include where the idle target was, or the
    /// pointer would land on nothing the moment the controls open.
    #[test]
    fn the_controls_cover_the_idle_target() {
        for notes in [false, true] {
            assert!(target_rect(Target::Controls, notes)
                .contains_rect(target_rect(Target::Idle, notes)));
        }
    }

    #[test]
    fn zones_resolve_by_mode() {
        let mic = mic_rect().center();
        let gear = gear_rect(false).center();
        assert_eq!(Overlay::zone_at(Mode::Idle, false, mic), Zone::Mic);
        assert_eq!(Overlay::zone_at(Mode::Idle, false, gear), Zone::Gear);
        assert_eq!(Overlay::zone_at(Mode::Listening, false, mic), Zone::Pill);
        assert_eq!(Overlay::zone_at(Mode::Transcribing, false, mic), Zone::None);
    }

    /// With the Notes button on, it sits between the mic and the gear, the
    /// gear moves out one slot, and nothing overlaps.
    #[test]
    fn the_notes_button_sits_between_the_mic_and_the_gear() {
        let (mic, notes, gear) = (mic_rect(), notes_rect(), gear_rect(true));
        assert!(mic.right() < notes.left() && notes.right() < gear.left());
        assert_eq!(gear_rect(false), notes, "off: the gear keeps its old place");
        assert_eq!(Overlay::zone_at(Mode::Idle, true, mic.center()), Zone::Mic);
        assert_eq!(Overlay::zone_at(Mode::Idle, true, notes.center()), Zone::Notes);
        assert_eq!(Overlay::zone_at(Mode::Idle, true, gear.center()), Zone::Gear);
        // off: the notes slot is the gear, and the Notes zone cannot occur
        assert_eq!(
            Overlay::zone_at(Mode::Idle, false, notes.center()),
            Zone::Gear
        );
        // only an idle Catcher has these buttons
        assert_eq!(
            Overlay::zone_at(Mode::Listening, true, notes.center()),
            Zone::None
        );
    }
}
