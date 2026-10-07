//! Permission checklist shared by the setup wizard and Settings › Permissions.
//!
//! macOS needs three privacy grants (Microphone, Accessibility, Input
//! Monitoring) plus one setting when the hotkey is fn ("Press fn key to: Do
//! Nothing", or holding fn opens the emoji picker). Each row shows its live
//! state and the one action that fixes it:
//!
//! * Microphone: the system prompt, asked right here instead of mid-dictation.
//! * Accessibility / Input Monitoring: register the app with the system (so it
//!   is already in the list), open the exact pane, and float a small panel
//!   beside System Settings holding the app icon, which the user drags into
//!   the list. That last step is the one people get stuck on without it.
//! * fn: write the preference, with Keyboard Settings as the fallback.
//!
//! macOS caches some grants per process (CLAUDE.md, "macOS signing"), so a
//! grant can read as missing until the daemon restarts. Nothing here gates on
//! these checks; the restart button is the escape hatch.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use eframe::egui;
use egui_phosphor::regular as icons;

use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mic {
    /// Never asked: the prompt will appear when we ask.
    NotAsked,
    Denied,
    Granted,
}

/// One snapshot of everything the checklist shows. Refreshed at most once a
/// second: `defaults` is a process spawn, the rest are cheap syscalls.
#[derive(Clone, Copy, Debug)]
pub struct Status {
    pub mic: Mic,
    pub accessibility: bool,
    pub input_monitoring: bool,
    /// Whether the configured hotkey is fn.
    pub uses_fn: bool,
    /// fn is free for dictation ("Press fn key to: Do Nothing").
    pub fn_free: bool,
}

impl Status {
    pub fn all_granted(&self) -> bool {
        self.mic == Mic::Granted
            && self.accessibility
            && self.input_monitoring
            && (!self.uses_fn || self.fn_free)
    }
}

static CACHE: Mutex<Option<(Instant, Status)>> = Mutex::new(None);

/// Live status, cached for a second.
pub fn status() -> Status {
    let mut c = CACHE.lock().unwrap();
    if let Some((at, s)) = *c {
        if at.elapsed() < Duration::from_secs(1) {
            return s;
        }
    }
    let s = probe();
    *c = Some((Instant::now(), s));
    s
}

/// Drop the cache so the next frame re-reads (after an action).
fn invalidate() {
    *CACHE.lock().unwrap() = None;
}

#[cfg(target_os = "macos")]
fn probe() -> Status {
    let uses_fn = crate::config::load().map(|c| c.key == "fn").unwrap_or(true);
    Status {
        mic: mac::mic_status(),
        accessibility: wc_hotkey::keyboard_accessible(),
        input_monitoring: wc_hotkey::input_monitoring_granted(),
        uses_fn,
        fn_free: !uses_fn || mac::fn_usage() == Some(0),
    }
}

#[cfg(not(target_os = "macos"))]
fn probe() -> Status {
    let ok = wc_hotkey::keyboard_accessible();
    Status {
        mic: Mic::Granted,
        accessibility: ok,
        input_monitoring: ok,
        uses_fn: false,
        fn_free: true,
    }
}

/// True when every grant the app needs reads as granted right now.
pub fn all_granted() -> bool {
    status().all_granted()
}

// ------------------------------------------------------------------ rows

struct Row {
    icon: &'static str,
    title: &'static str,
    why: &'static str,
    ok: bool,
    action: Action,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    None,
    AskMic,
    OpenMicPane,
    Accessibility,
    InputMonitoring,
    FreeFn,
}

fn rows(s: &Status) -> Vec<Row> {
    let mut v = vec![
        Row {
            icon: icons::MICROPHONE,
            title: "Microphone",
            why: "Hears you while you hold the key. Stays on this Mac.",
            ok: s.mic == Mic::Granted,
            action: match s.mic {
                Mic::NotAsked => Action::AskMic,
                Mic::Denied => Action::OpenMicPane,
                Mic::Granted => Action::None,
            },
        },
        Row {
            icon: icons::CURSOR_TEXT,
            title: "Accessibility",
            why: "Types the transcript into the app you are using.",
            ok: s.accessibility,
            action: Action::Accessibility,
        },
        Row {
            icon: icons::KEYBOARD,
            title: "Input Monitoring",
            why: "Notices the hotkey, from any app.",
            ok: s.input_monitoring,
            action: Action::InputMonitoring,
        },
    ];
    if s.uses_fn {
        v.push(Row {
            icon: icons::GLOBE,
            title: "fn key",
            why: "Sets \"Press fn key to\" to Do Nothing.",
            ok: s.fn_free,
            action: Action::FreeFn,
        });
    }
    v
}

/// Width kept free on the right of a row for its badge or button.
const CONTROL_W: f32 = 116.0;

/// One checklist row, laid out by hand in a single rect: egui's horizontal
/// centring places the plate before it knows how tall the text is, which
/// leaves the plate riding high next to a wrapped description.
fn row_ui(ui: &mut egui::Ui, row: &Row) {
    let w = ui.available_width();
    let text_x = 36.0 + 14.0;
    let text_w = (w - text_x - CONTROL_W).max(120.0);
    let title = ui.fonts(|f| f.layout_no_wrap(row.title.into(), theme::medium(14.0), theme::fg()));
    let why = ui.fonts(|f| {
        f.layout(
            row.why.into(),
            egui::FontId::proportional(12.5),
            theme::muted(),
            text_w,
        )
    });
    let text_h = title.size().y + 3.0 + why.size().y;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, text_h.max(40.0)), egui::Sense::hover());
    let cy = rect.center().y;

    paint_plate(
        ui.painter(),
        egui::Rect::from_min_size(egui::pos2(rect.left(), cy - 18.0), egui::vec2(36.0, 36.0)),
        row.icon,
        row.ok,
    );
    let ty = cy - text_h / 2.0;
    let p = ui.painter();
    p.galley(
        egui::pos2(rect.left() + text_x, ty),
        title.clone(),
        theme::fg(),
    );
    p.galley(
        egui::pos2(rect.left() + text_x, ty + title.size().y + 3.0),
        why,
        theme::muted(),
    );

    let ctrl = egui::Rect::from_min_max(egui::pos2(rect.right() - CONTROL_W, rect.top()), rect.max);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(ctrl)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            if row.ok {
                theme::badge(
                    ui,
                    &format!("{}  Granted", icons::CHECK),
                    theme::Tone::Accent,
                );
                return;
            }
            let label = match row.action {
                Action::AskMic => "Allow",
                Action::OpenMicPane => "Open Settings",
                Action::FreeFn => "Fix it",
                _ => "Grant",
            };
            if theme::button_with(ui, theme::Variant::Primary, None, label, true).clicked() {
                run_action(row.action);
            }
        },
    );
}

fn paint_plate(p: &egui::Painter, rect: egui::Rect, icon: &str, ok: bool) {
    let (fill, ring, ink) = if ok {
        (
            theme::tint(theme::accent_ink()),
            theme::tint_strong(theme::accent_ink()),
            theme::accent_ink(),
        )
    } else {
        (theme::surface_2(), theme::border(), theme::text_2())
    };
    p.rect_filled(rect, 9.0, fill);
    p.rect_stroke(
        rect,
        9.0,
        egui::Stroke::new(1.0, ring),
        egui::StrokeKind::Inside,
    );
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(18.0),
        ink,
    );
}

fn run_action(a: Action) {
    #[cfg(target_os = "macos")]
    match a {
        Action::None => {}
        Action::AskMic => mac::request_mic(),
        Action::OpenMicPane => mac::open_pane("Privacy_Microphone"),
        Action::Accessibility => {
            // The system prompt also adds the app to the list (switched off),
            // so the drag is a fallback rather than the only way in.
            wc_hotkey::request_accessibility();
            mac::open_pane("Privacy_Accessibility");
            mac::show_drag_helper("accessibility");
        }
        Action::InputMonitoring => {
            mac::request_input_monitoring();
            mac::open_pane("Privacy_ListenEvent");
            mac::show_drag_helper("input-monitoring");
        }
        Action::FreeFn => {
            if !mac::set_fn_do_nothing() {
                mac::open_keyboard_settings();
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = a;
    invalidate();
}

/// The full checklist plus the restart escape hatch (Settings › Permissions).
/// Self-contained; polls and repaints on its own.
pub fn panel(ui: &mut egui::Ui) {
    checklist(ui);

    #[cfg(target_os = "macos")]
    {
        ui.add_space(14.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(
                    "Granted something just now? macOS applies Input Monitoring \
                     only after the app restarts.",
                )
                .size(12.5)
                .color(theme::muted()),
            );
        });
        ui.add_space(6.0);
        if theme::button_with(
            ui,
            theme::Variant::Outline,
            Some(icons::ARROW_CLOCKWISE),
            &format!("Restart {}", crate::app_name()),
            true,
        )
        .clicked()
        {
            mac::restart_daemon();
        }
    }
}

/// One row per permission with its live state and the action that fixes it
/// (the setup wizard shows just this; it relaunches the app by itself).
pub fn checklist(ui: &mut egui::Ui) {
    #[cfg(target_os = "macos")]
    mac::refocus_after_settings(ui.ctx());
    let s = status();
    ui.ctx().request_repaint_after(Duration::from_secs(1));
    #[cfg(target_os = "macos")]
    if s.accessibility && s.input_monitoring {
        mac::hide_drag_helper();
    }

    let list = rows(&s);
    let done = list.iter().filter(|r| r.ok).count();
    ui.horizontal(|ui| {
        theme::badge(
            ui,
            &format!("{done} of {} ready", list.len()),
            if done == list.len() {
                theme::Tone::Accent
            } else {
                theme::Tone::Warn
            },
        );
    });
    ui.add_space(8.0);

    theme::card(ui).inner_margin(0.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        let n = list.len();
        for (i, row) in list.into_iter().enumerate() {
            egui::Frame::default()
                .inner_margin(egui::Margin::symmetric(16, 14))
                .show(ui, |ui| row_ui(ui, &row));
            if i + 1 < n {
                theme::hairline(ui);
            }
        }
    });
}

/// Under the hotkey picker: when `key_slug` is "fn" and macOS still has the
/// Globe/fn key bound to the emoji picker or dictation, explain and offer the
/// one-click fix. Draws nothing otherwise.
pub fn fn_key_notice(ui: &mut egui::Ui, key_slug: &str) {
    #[cfg(target_os = "macos")]
    {
        if key_slug != "fn" {
            return;
        }
        mac::refocus_after_settings(ui.ctx());
        let free = mac::fn_usage_cached() == Some(0);
        ui.add_space(4.0);
        if free {
            ui.label(
                egui::RichText::new(format!("{}  fn is free for dictation", icons::CHECK))
                    .size(12.5)
                    .color(theme::accent_ink()),
            );
            return;
        }
        egui::Frame::default()
            .fill(theme::surface_2())
            .stroke(egui::Stroke::new(1.0, theme::tint_strong(theme::amber())))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new(icons::WARNING)
                            .size(14.0)
                            .color(theme::amber()),
                    );
                    ui.label(
                        egui::RichText::new(
                            "macOS still opens the emoji picker when you press fn. \
                             Set it to Do Nothing so holding fn dictates.",
                        )
                        .size(12.5)
                        .color(theme::fg()),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if theme::button_with(ui, theme::Variant::Primary, None, "Fix it", true)
                        .clicked()
                    {
                        if !mac::set_fn_do_nothing() {
                            mac::open_keyboard_settings();
                        }
                        invalidate();
                    }
                    if theme::button_with(
                        ui,
                        theme::Variant::Ghost,
                        None,
                        "Keyboard Settings",
                        true,
                    )
                    .clicked()
                    {
                        mac::open_keyboard_settings();
                    }
                });
            });
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (ui, key_slug);
}

/// `whisper-catch drag-helper <pane>`: the floating drag-to-grant panel.
pub fn drag_helper(pane: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    return mac::drag_helper::run(pane);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pane;
        anyhow::bail!("the drag helper is macOS only")
    }
}

/// The `.app` bundle this binary runs from, if it runs from one.
pub fn bundle_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension()? == "app").then(|| app.to_path_buf())
}

/// Starts the app again from its bundle once this process has gone, so a
/// fresh process picks up grants macOS cached as missing in this one. False
/// when not running from a bundle (a dev build), where there is nothing to
/// relaunch and the caller should carry on in-process.
#[cfg(target_os = "macos")]
pub fn relaunch_after_exit() -> bool {
    let Some(app) = bundle_path() else {
        return false;
    };
    let pid = std::process::id();
    // wait for this pid to exit (and drop the instance lock), then reopen
    let script = format!(
        "while kill -0 {pid} 2>/dev/null; do sleep 0.2; done; open \"{}\"",
        app.display()
    );
    std::process::Command::new("/bin/sh")
        .args(["-c", &script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

#[cfg(target_os = "macos")]
pub mod mac {
    use super::*;
    use std::process::Command;

    /// Open one System Settings privacy pane.
    pub fn open_pane(pane: &str) {
        let _ = Command::new("open")
            .arg(format!(
                "x-apple.systempreferences:com.apple.preference.security?{pane}"
            ))
            .status();
        watch_settings();
    }

    pub fn open_keyboard_settings() {
        let _ = Command::new("open")
            .arg("x-apple.systempreferences:com.apple.Keyboard-Settings.extension")
            .status();
        watch_settings();
    }

    // ------------------------------------------------------------- focus

    /// Raises this process's window and makes it the active app. The app is
    /// an accessory (no Dock icon), so when System Settings closes macOS hands
    /// focus to the next *regular* app and our window ends up behind it.
    ///
    /// It also makes the process a *regular* app (Dock icon) for as long as
    /// the window is open. An accessory app is skipped when macOS picks who
    /// gets focus after a system prompt (Microphone) or System Settings goes
    /// away, so the window kept falling behind whatever else was open. The
    /// daemon drops back to accessory when its tray starts after setup
    /// (`wc_tray::run_main`); the main window is its own process and exits.
    pub fn bring_to_front(ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        if let Some(mtm) = objc2_foundation::MainThreadMarker::new() {
            let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
            app.setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Regular);
            #[allow(deprecated)] // `activate` needs macOS 14; we support 11
            app.activateIgnoringOtherApps(true);
        }
    }

    /// Set from a system prompt's completion handler (any thread); the next
    /// checklist frame brings the window back.
    static REFOCUS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    /// The checklist window's context, so a completion handler can wake it.
    static CHECKLIST_CTX: Mutex<Option<egui::Context>> = Mutex::new(None);

    fn request_refocus() {
        REFOCUS.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(ctx) = CHECKLIST_CTX.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }

    /// 0 = not watching, 1 = we opened System Settings and wait for its
    /// window, 2 = its window has been seen and we wait for it to close.
    static SETTINGS_WATCH: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    static WATCH_SINCE: Mutex<Option<Instant>> = Mutex::new(None);
    static WATCH_CHECKED: Mutex<Option<Instant>> = Mutex::new(None);

    fn watch_settings() {
        SETTINGS_WATCH.store(1, std::sync::atomic::Ordering::Relaxed);
        *WATCH_SINCE.lock().unwrap() = Some(Instant::now());
    }

    /// Called every frame by the checklist: once the System Settings window we
    /// opened has closed, bring our window back on top, whether or not the
    /// user changed anything there. Checked at most every 300ms.
    pub fn refocus_after_settings(ctx: &egui::Context) {
        use std::sync::atomic::Ordering;
        {
            let mut slot = CHECKLIST_CTX.lock().unwrap();
            if slot.is_none() {
                *slot = Some(ctx.clone());
            }
        }
        if REFOCUS.swap(false, Ordering::Relaxed) {
            bring_to_front(ctx);
        }
        let state = SETTINGS_WATCH.load(Ordering::Relaxed);
        if state == 0 {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(300));
        {
            let mut checked = WATCH_CHECKED.lock().unwrap();
            if checked.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
                return;
            }
            *checked = Some(Instant::now());
        }
        let open = settings_window().is_some();
        match (state, open) {
            (1, true) => SETTINGS_WATCH.store(2, Ordering::Relaxed),
            (1, false) => {
                // never appeared (or the user was very quick): give up quietly
                let stale = WATCH_SINCE
                    .lock()
                    .unwrap()
                    .is_none_or(|t| t.elapsed() > Duration::from_secs(15));
                if stale {
                    SETTINGS_WATCH.store(0, Ordering::Relaxed);
                }
            }
            (2, false) => {
                SETTINGS_WATCH.store(0, Ordering::Relaxed);
                super::invalidate();
                bring_to_front(ctx);
            }
            _ => {}
        }
    }

    /// Frame of the System Settings window on screen, in CG (top-left)
    /// coordinates. Matched by pid rather than by the localized app name.
    pub fn settings_window() -> Option<core_graphics::geometry::CGRect> {
        use core_foundation::base::{CFType, TCFType};
        use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
        use core_foundation::number::CFNumber;
        use core_foundation::string::CFString;
        use core_graphics::geometry::CGRect;
        use core_graphics::window::{
            copy_window_info, kCGNullWindowID, kCGWindowListExcludeDesktopElements,
            kCGWindowListOptionOnScreenOnly,
        };

        let pid = settings_pid()?;
        let list = copy_window_info(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            kCGNullWindowID,
        )?;
        for raw in list.iter() {
            let dict: CFDictionary<CFString, CFType> =
                unsafe { CFDictionary::wrap_under_get_rule(*raw as CFDictionaryRef) };
            let num = |k: &'static str| {
                dict.find(CFString::from_static_string(k))
                    .and_then(|v| v.downcast::<CFNumber>())
                    .and_then(|n| n.to_i64())
            };
            if num("kCGWindowOwnerPID") != Some(pid) || num("kCGWindowLayer") != Some(0) {
                continue;
            }
            let bounds = dict
                .find(CFString::from_static_string("kCGWindowBounds"))
                .and_then(|v| v.downcast::<CFDictionary>())?;
            let r = CGRect::from_dict_representation(&bounds)?;
            if r.size.width > 200.0 && r.size.height > 200.0 {
                return Some(r);
            }
        }
        None
    }

    fn settings_pid() -> Option<i64> {
        let apps = objc2_app_kit::NSRunningApplication::runningApplicationsWithBundleIdentifier(
            &objc2_foundation::NSString::from_str("com.apple.systempreferences"),
        );
        Some(apps.firstObject()?.processIdentifier() as i64)
    }

    /// Daemon side: whenever a grant is missing and System Settings is open,
    /// keep the drag helper running, however Settings was reached. macOS's own
    /// "would like to receive keystrokes" prompt opens Input Monitoring
    /// without going through our Grant button, and that is exactly where
    /// people got stuck without the icon to drag. The helper shows itself only
    /// beside a Settings window, names the list that still needs the app, and
    /// quits once both grants are in.
    pub fn watch_for_settings() {
        std::thread::spawn(|| {
            let mut helper: Option<std::process::Child> = None;
            // closed with its ✕ (or another helper already showing): leave it
            // closed until System Settings itself closes
            let mut dismissed = false;
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let missing =
                    !wc_hotkey::keyboard_accessible() || !wc_hotkey::input_monitoring_granted();
                let alive = helper
                    .as_mut()
                    .is_some_and(|c| matches!(c.try_wait(), Ok(None)));
                if helper.is_some() && !alive {
                    helper = None;
                    dismissed = true;
                }
                let settings_open = settings_pid().is_some();
                if !settings_open {
                    dismissed = false;
                }
                if missing && !alive && settings_open && !dismissed {
                    let Ok(exe) = std::env::current_exe() else {
                        return;
                    };
                    helper = Command::new(exe)
                        .args(["drag-helper", "auto"])
                        .stdin(std::process::Stdio::piped())
                        .spawn()
                        .ok();
                } else if !missing && alive {
                    if let Some(mut c) = helper.take() {
                        let _ = c.kill();
                        let _ = c.wait();
                    }
                }
            }
        });
    }

    // ------------------------------------------------------- Microphone

    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {
        static AVMediaTypeAudio: &'static objc2_foundation::NSString;
    }

    fn capture_device_class() -> Option<&'static objc2::runtime::AnyClass> {
        objc2::runtime::AnyClass::get(c"AVCaptureDevice")
    }

    pub fn mic_status() -> Mic {
        let Some(cls) = capture_device_class() else {
            return Mic::NotAsked;
        };
        // AVAuthorizationStatus: 0 not determined, 1 restricted, 2 denied, 3 authorized
        let s: isize =
            unsafe { objc2::msg_send![cls, authorizationStatusForMediaType: AVMediaTypeAudio] };
        match s {
            3 => Mic::Granted,
            0 => Mic::NotAsked,
            _ => Mic::Denied,
        }
    }

    /// Shows the system Microphone prompt now, so the first dictation is not
    /// the moment it appears (and eats the first utterance).
    pub fn request_mic() {
        let Some(cls) = capture_device_class() else {
            return;
        };
        let block = block2::RcBlock::new(|granted: objc2::runtime::Bool| {
            log::info!("microphone access: {}", granted.as_bool());
            super::invalidate();
            // the prompt took focus; give it back to the checklist window
            request_refocus();
        });
        unsafe {
            let _: () = objc2::msg_send![
                cls,
                requestAccessForMediaType: AVMediaTypeAudio,
                completionHandler: &*block
            ];
        }
    }

    // ------------------------------------------------- Input Monitoring

    /// Asks for Input Monitoring: shows the system prompt the first time and,
    /// either way, puts the app in the Input Monitoring list.
    pub fn request_input_monitoring() {
        #[link(name = "IOKit", kind = "framework")]
        extern "C" {
            fn IOHIDRequestAccess(request_type: u32) -> bool;
        }
        const K_IOHID_REQUEST_TYPE_LISTEN_EVENT: u32 = 1;
        unsafe { IOHIDRequestAccess(K_IOHID_REQUEST_TYPE_LISTEN_EVENT) };
    }

    // ------------------------------------------------------------- fn key

    /// `AppleFnUsageType`: 0 Do Nothing, 1 Change Input Source, 2 Show Emoji
    /// & Symbols, 3 Start Dictation. None when unset, which means the system
    /// default (not Do Nothing).
    pub fn fn_usage() -> Option<i64> {
        let out = Command::new("defaults")
            .args(["read", "com.apple.HIToolbox", "AppleFnUsageType"])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }

    static FN_CACHE: Mutex<Option<(Instant, Option<i64>)>> = Mutex::new(None);

    pub fn fn_usage_cached() -> Option<i64> {
        let mut c = FN_CACHE.lock().unwrap();
        if let Some((at, v)) = *c {
            if at.elapsed() < Duration::from_secs(1) {
                return v;
            }
        }
        let v = fn_usage();
        *c = Some((Instant::now(), v));
        v
    }

    /// Sets "Press fn key to: Do Nothing". Returns whether it reads back as
    /// set; the caller opens Keyboard Settings when it does not.
    pub fn set_fn_do_nothing() -> bool {
        let ok = Command::new("defaults")
            .args([
                "write",
                "com.apple.HIToolbox",
                "AppleFnUsageType",
                "-int",
                "0",
            ])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        *FN_CACHE.lock().unwrap() = None;
        ok && fn_usage() == Some(0)
    }

    // ------------------------------------------------------ drag helper

    static HELPER: Mutex<Option<std::process::Child>> = Mutex::new(None);

    /// Floats the drag-to-grant panel beside System Settings, replacing any
    /// helper already showing. Its stdin is a pipe to us, so it also goes away
    /// when this window closes.
    pub fn show_drag_helper(pane: &str) {
        hide_drag_helper();
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        match Command::new(exe)
            .args(["drag-helper", pane])
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => *HELPER.lock().unwrap() = Some(child),
            Err(e) => log::warn!("drag helper failed to start: {e}"),
        }
    }

    pub fn hide_drag_helper() {
        if let Some(mut c) = HELPER.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    // ------------------------------------------------------------ restart

    /// Settings › Permissions › Restart: stop the running daemon (its pid is
    /// in the instance lock) and start the app again once it has gone.
    pub fn restart_daemon() {
        let pid = std::fs::read_to_string(crate::instance_lock_path())
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok());
        let Some(app) = bundle_path() else {
            log::warn!("restart: not running from an app bundle");
            return;
        };
        let wait = match pid {
            Some(pid) => format!(
                "kill {pid} 2>/dev/null; while kill -0 {pid} 2>/dev/null; do sleep 0.2; done; "
            ),
            None => String::new(),
        };
        let script = format!("{wait}open \"{}\"", app.display());
        let _ = Command::new("/bin/sh")
            .args(["-c", &script])
            .stdin(std::process::Stdio::null())
            .spawn();
    }

    pub mod drag_helper {
        //! A small borderless panel that sits under the System Settings
        //! window and holds the app icon. Dragging the icon into the
        //! Accessibility or Input Monitoring list adds the app and switches it
        //! on, which is much easier than the "+" button and a file picker.
        //! Pure AppKit: egui cannot start a native file drag.

        use std::cell::RefCell;

        use objc2::rc::Retained;
        use objc2::runtime::{AnyObject, ProtocolObject};
        use objc2::{define_class, msg_send, sel, AllocAnyThread, DefinedClass, MainThreadOnly};
        use objc2_app_kit::{
            NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBox, NSBoxType,
            NSButton, NSColor, NSDragOperation, NSDraggingContext, NSDraggingItem,
            NSDraggingSession, NSDraggingSource, NSEvent, NSFont, NSImageView, NSPanel, NSScreen,
            NSTextField, NSTitlePosition, NSWindowCollectionBehavior, NSWindowStyleMask,
            NSWorkspace,
        };
        use objc2_foundation::{
            MainThreadMarker, NSArray, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
            NSTimer, NSURL,
        };

        const PANEL_W: f64 = 372.0;
        const PANEL_H: f64 = 88.0;

        pub struct Ivars {
            path: String,
        }

        define_class!(
            #[unsafe(super(NSImageView))]
            #[thread_kind = MainThreadOnly]
            #[name = "WCDragIcon"]
            #[ivars = Ivars]
            struct DragIcon;

            unsafe impl NSObjectProtocol for DragIcon {}

            unsafe impl NSDraggingSource for DragIcon {
                #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
                fn source_operation_mask(
                    &self,
                    _session: &NSDraggingSession,
                    _context: NSDraggingContext,
                ) -> NSDragOperation {
                    NSDragOperation::Copy | NSDragOperation::Generic | NSDragOperation::Link
                }
            }

            impl DragIcon {
                #[unsafe(method(mouseDown:))]
                fn mouse_down(&self, event: &NSEvent) {
                    self.begin_drag(event);
                }

                #[unsafe(method(acceptsFirstMouse:))]
                fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
                    true
                }
            }
        );

        impl DragIcon {
            fn new(mtm: MainThreadMarker, frame: NSRect, path: String) -> Retained<Self> {
                let this = Self::alloc(mtm).set_ivars(Ivars { path });
                unsafe { msg_send![super(this), initWithFrame: frame] }
            }

            fn begin_drag(&self, event: &NSEvent) {
                let url = NSURL::fileURLWithPath(&NSString::from_str(&self.ivars().path));
                let writer = ProtocolObject::from_ref(&*url);
                let item =
                    NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), writer);
                let image = self.image();
                unsafe {
                    item.setDraggingFrame_contents(
                        self.bounds(),
                        image.as_deref().map(|i| i as &AnyObject),
                    );
                }
                let items = NSArray::from_retained_slice(&[item]);
                self.beginDraggingSessionWithItems_event_source(
                    &items,
                    event,
                    ProtocolObject::from_ref(self),
                );
            }
        }

        fn label(
            mtm: MainThreadMarker,
            text: &str,
            size: f64,
            bold: bool,
            rgb: (f64, f64, f64),
        ) -> Retained<NSTextField> {
            let l = NSTextField::labelWithString(&NSString::from_str(text), mtm);
            let font = if bold {
                NSFont::boldSystemFontOfSize(size)
            } else {
                NSFont::systemFontOfSize(size)
            };
            l.setFont(Some(&font));
            l.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
                rgb.0, rgb.1, rgb.2, 1.0,
            )));
            l
        }

        /// Under the System Settings window when there is room, else tucked
        /// inside its bottom edge; bottom-centre of the screen when Settings is
        /// not open (yet).
        fn place(panel: &NSPanel, mtm: MainThreadMarker) {
            let screens = NSScreen::screens(mtm);
            let Some(primary) = screens.firstObject() else {
                return;
            };
            let primary_h = primary.frame().size.height;
            let origin = match super::settings_window() {
                Some(w) => {
                    let x = w.origin.x + (w.size.width - PANEL_W) / 2.0;
                    let below = w.origin.y + w.size.height + 12.0;
                    let vf = primary.visibleFrame();
                    let screen_bottom = primary_h - vf.origin.y;
                    let top = if below + PANEL_H <= screen_bottom {
                        below
                    } else {
                        w.origin.y + w.size.height - PANEL_H - 18.0
                    };
                    NSPoint::new(x, primary_h - top - PANEL_H)
                }
                None => {
                    let vf = primary.visibleFrame();
                    NSPoint::new(
                        vf.origin.x + (vf.size.width - PANEL_W) / 2.0,
                        vf.origin.y + 80.0,
                    )
                }
            };
            let cur = panel.frame().origin;
            if (cur.x - origin.x).abs() > 0.5 || (cur.y - origin.y).abs() > 0.5 {
                panel.setFrameOrigin(origin);
            }
        }

        /// The helper's second line, naming the list that still needs the app:
        /// `preferred` when that one is missing, else whichever is. None when
        /// both grants are in and the helper has nothing left to do.
        fn needs_text(preferred: &str) -> Option<String> {
            let ax = !wc_hotkey::keyboard_accessible();
            let im = !wc_hotkey::input_monitoring_granted();
            let list = match (ax, im) {
                (false, false) => return None,
                (true, true) if preferred == "Input Monitoring" => "Input Monitoring",
                (true, true) => {
                    return Some(
                        "Drop it on Accessibility, then on Input\nMonitoring. Each switches on for you."
                            .into(),
                    )
                }
                (true, false) => "Accessibility",
                (false, true) => "Input Monitoring",
            };
            Some(format!(
                "Drop it on the {list} list above. It is\nadded and switched on for you."
            ))
        }

        pub fn run(pane: &str) -> anyhow::Result<()> {
            let mtm = MainThreadMarker::new()
                .ok_or_else(|| anyhow::anyhow!("drag helper must run on the main thread"))?;

            // One helper at a time: the Grant buttons and the daemon's
            // watcher can both ask for one. The lock dies with the process.
            let lock = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(std::env::temp_dir().join("whisper-catch-helper.lock"))?;
            if fs2::FileExt::try_lock_exclusive(&lock).is_err() {
                return Ok(());
            }
            if needs_text("").is_none() {
                return Ok(()); // nothing to grant
            }
            let app = NSApplication::sharedApplication(mtm);
            app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

            // gone when the window that opened us goes (stdin is its pipe)
            std::thread::spawn(|| {
                let mut sink = Vec::new();
                let _ = std::io::Read::read_to_end(&mut std::io::stdin(), &mut sink);
                std::process::exit(0);
            });
            // and never outstays its welcome
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(600));
                std::process::exit(0);
            });

            let path = super::super::bundle_path()
                .or_else(|| std::env::current_exe().ok())
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let list_name = if pane == "input-monitoring" {
                "Input Monitoring"
            } else {
                "Accessibility"
            };

            let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(PANEL_W, PANEL_H));
            let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
                NSPanel::alloc(mtm),
                frame,
                NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                NSBackingStoreType::Buffered,
                false,
            );
            panel.setLevel(3); // NSFloatingWindowLevel: above System Settings
            panel.setOpaque(false);
            panel.setBackgroundColor(Some(&NSColor::clearColor()));
            panel.setHasShadow(true);
            panel.setMovableByWindowBackground(true);
            panel.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::FullScreenAuxiliary,
            );
            unsafe { panel.setReleasedWhenClosed(false) };

            // the card: #30353e, hairline, radius 14 (theme DARK surface / ring; it
            // floats over System Settings, so it stays dark in both modes)
            let card = NSBox::new(mtm);
            card.setBoxType(NSBoxType::Custom);
            card.setTitlePosition(NSTitlePosition::NoTitle);
            card.setCornerRadius(14.0);
            card.setBorderWidth(1.0);
            card.setFillColor(&NSColor::colorWithSRGBRed_green_blue_alpha(
                0.188, 0.208, 0.243, 0.98,
            ));
            card.setBorderColor(&NSColor::colorWithSRGBRed_green_blue_alpha(
                0.408, 0.424, 0.447, 1.0,
            ));
            card.setContentViewMargins(NSSize::new(0.0, 0.0));
            card.setFrame(frame);
            panel.setContentView(Some(&card));

            let icon = DragIcon::new(
                mtm,
                NSRect::new(NSPoint::new(14.0, 14.0), NSSize::new(60.0, 60.0)),
                path.clone(),
            );
            let ws = NSWorkspace::sharedWorkspace();
            let img = ws.iconForFile(&NSString::from_str(&path));
            icon.setImage(Some(&img));
            card.addSubview(&icon);

            let title = label(
                mtm,
                &format!("Drag {} into the list", crate::app_name()),
                13.5,
                true,
                (0.933, 0.933, 0.933),
            );
            title.setFrame(NSRect::new(
                NSPoint::new(86.0, 48.0),
                NSSize::new(250.0, 20.0),
            ));
            card.addSubview(&title);
            let sub = label(
                mtm,
                &needs_text(list_name).unwrap_or_default(),
                11.5,
                false,
                (0.722, 0.725, 0.737),
            );
            sub.setFrame(NSRect::new(
                NSPoint::new(86.0, 14.0),
                NSSize::new(270.0, 32.0),
            ));
            card.addSubview(&sub);

            // yellow "drag me" arrow beside the icon
            let arrow = label(mtm, "↑", 15.0, true, (1.0, 0.827, 0.412));
            arrow.setFrame(NSRect::new(
                NSPoint::new(66.0, 56.0),
                NSSize::new(16.0, 20.0),
            ));
            card.addSubview(&arrow);

            let close = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str("✕"),
                    Some(&app),
                    Some(sel!(terminate:)),
                    mtm,
                )
            };
            close.setBordered(false);
            close.setFrame(NSRect::new(
                NSPoint::new(PANEL_W - 30.0, PANEL_H - 28.0),
                NSSize::new(22.0, 20.0),
            ));
            card.addSubview(&close);

            place(&panel, mtm);

            // Follow the Settings window as it opens, moves or resizes; show
            // only while one is on screen; keep the text on whichever list
            // still needs the app, and leave once both grants are in.
            let held = RefCell::new((panel.clone(), sub.clone(), 0u32, String::new()));
            let block = block2::RcBlock::new(move |_t: std::ptr::NonNull<NSTimer>| {
                let Some(mtm) = MainThreadMarker::new() else {
                    return;
                };
                let mut h = held.borrow_mut();
                h.2 += 1;
                if h.2 % 4 == 1 {
                    match needs_text("") {
                        None => std::process::exit(0),
                        Some(text) if text != h.3 => {
                            h.1.setStringValue(&NSString::from_str(&text));
                            h.3 = text;
                        }
                        Some(_) => {}
                    }
                }
                if super::settings_window().is_some() {
                    place(&h.0, mtm);
                    h.0.orderFrontRegardless();
                } else {
                    h.0.orderOut(None);
                }
            });
            let _timer = unsafe {
                NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.25, true, &block)
            };

            app.run();
            Ok(())
        }
    }
}
