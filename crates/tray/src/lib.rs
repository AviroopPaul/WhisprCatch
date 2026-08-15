//! System tray (StatusNotifierItem via ksni on Linux; NSStatusItem via
//! tray-icon on macOS). GNOME needs the AppIndicator extension; app stays
//! usable without a tray.
//!
//! Menu composition per docs/DESIGN.md: status header (state + model +
//! hotkey), Listening toggle, Open History / Preferences…, divider, Quit.
//! Native menus can't be themed, so the design language shows up in
//! structure and icon states (idle outline / recording red) instead.

use std::sync::Arc;

use anyhow::Result;
use wc_core::state::AppState;

/// Static facts the tray shows in its header (resolved at daemon startup).
#[derive(Clone, Debug, Default)]
pub struct TrayInfo {
    /// Human model label, e.g. "Parakeet 0.6B".
    pub model: String,
    /// Human hotkey label, e.g. "Right Alt".
    pub hotkey: String,
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::sync::atomic::Ordering;
    use ksni::blocking::{Handle, TrayMethods};
    use ksni::menu::{CheckmarkItem, MenuItem, StandardItem};

    pub struct WcTray {
        state: Arc<AppState>,
        info: TrayInfo,
        /// Own binary path, resolved at startup — current_exe() goes stale
        /// ("… (deleted)") once a package upgrade replaces the binary.
        exe: std::path::PathBuf,
    }

    impl ksni::Tray for WcTray {
        fn id(&self) -> String {
            "whisper-catch".into()
        }

        fn icon_name(&self) -> String {
            if self.state.recording.load(Ordering::Relaxed) {
                "media-record-symbolic".into()
            } else if self.state.is_enabled() {
                // Own app icon; SNI resolves it from hicolor when installed via
                // the .deb. Running uninstalled, GNOME falls back to a generic
                // glyph — acceptable for dev runs.
                "whisper-catch".into()
            } else {
                "microphone-sensitivity-muted-symbolic".into()
            }
        }

        fn title(&self) -> String {
            "WhisprCatch".into()
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            let s = *self.state.stats.lock().unwrap();
            ksni::ToolTip {
                title: "WhisprCatch".into(),
                description: format!(
                    "{} words · {} utterances · {:.0}s audio",
                    s.words, s.utterances, s.audio_secs
                ),
                ..Default::default()
            }
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            // status header: state + model + hotkey (ksni rebuilds the menu on
            // every refresh(), so this stays live)
            let status = if self.state.recording.load(Ordering::Relaxed) {
                "Recording"
            } else if self.state.is_enabled() {
                "Ready"
            } else {
                "Paused"
            };
            vec![
                StandardItem {
                    label: format!("{status} · {}", self.info.model),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: format!("Hold {} to dictate", self.info.hotkey),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                CheckmarkItem {
                    label: "Listening".into(),
                    checked: self.state.is_enabled(),
                    activate: Box::new(|this: &mut Self| {
                        let now = !this.state.is_enabled();
                        this.state.enabled.store(now, Ordering::Relaxed);
                        log::info!("listening {}", if now { "enabled" } else { "disabled" });
                    }),
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                StandardItem {
                    label: "Open History".into(),
                    icon_name: "document-open-recent".into(),
                    activate: Box::new(|this: &mut Self| {
                        if let Err(e) =
                            std::process::Command::new(&this.exe).arg("settings").spawn()
                        {
                            log::error!("failed to open history: {e}");
                        }
                    }),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Preferences…".into(),
                    icon_name: "preferences-system".into(),
                    activate: Box::new(|this: &mut Self| {
                        if let Err(e) = std::process::Command::new(&this.exe)
                            .args(["settings", "--tab", "settings"])
                            .spawn()
                        {
                            log::error!("failed to open preferences: {e}");
                        }
                    }),
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                StandardItem {
                    label: "Quit WhisprCatch".into(),
                    icon_name: "application-exit".into(),
                    activate: Box::new(|_| std::process::exit(0)),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    /// Spawns the tray. Returns a handle used to refresh icon/menu after
    /// state changes. Fails gracefully when no StatusNotifierWatcher exists.
    pub fn spawn(state: Arc<AppState>, info: TrayInfo) -> Result<TrayHandle> {
        let exe = std::env::current_exe().unwrap_or_else(|_| "whisper-catch".into());
        let handle = WcTray { state, info, exe }
            .spawn()
            .map_err(|e| anyhow::anyhow!("tray unavailable: {e}"))?;
        Ok(TrayHandle(handle))
    }

    pub struct TrayHandle(Handle<WcTray>);

    impl TrayHandle {
        pub fn refresh(&self) {
            let _ = self.0.update(|_| {});
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::{spawn, TrayHandle};

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::path::PathBuf;

    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIconBuilder};

    /// The macOS tray owns the main thread (see [`run_main`]); there is no
    /// background handle to refresh. Kept so callers share one API shape.
    pub struct TrayHandle;

    impl TrayHandle {
        pub fn refresh(&self) {}
    }

    /// Not used on macOS — the tray must own the main run loop. Call
    /// [`run_main`] from the main thread instead and run dictation elsewhere.
    pub fn spawn(_state: Arc<AppState>, _info: TrayInfo) -> Result<TrayHandle> {
        anyhow::bail!("use wc_tray::run_main on macOS")
    }

    /// Builds the menu-bar item and runs the AppKit event loop. **Blocks
    /// forever** and MUST be called on the main thread. The dictation loop runs
    /// on another thread and shares `state` for the Listening toggle.
    pub fn run_main(state: Arc<AppState>, exe: PathBuf, info: TrayInfo) -> Result<()> {
        let mtm = MainThreadMarker::new()
            .ok_or_else(|| anyhow::anyhow!("run_main must be called on the main thread"))?;
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

        // header: model + hotkey (disabled rows; native menus can't be styled)
        let header = MenuItem::with_id("header", &info.model, false, None);
        let hint = MenuItem::with_id(
            "hint",
            format!("Hold {} to dictate", info.hotkey),
            false,
            None,
        );
        let toggle =
            CheckMenuItem::with_id("toggle", "Listening", true, state.is_enabled(), None);
        let history = MenuItem::with_id("history", "Open History", true, None);
        let prefs = MenuItem::with_id("prefs", "Preferences…", true, None);
        let quit = MenuItem::with_id("quit", "Quit WhisprCatch", true, None);

        let menu = Menu::new();
        menu.append(&header)?;
        menu.append(&hint)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&toggle)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&history)?;
        menu.append(&prefs)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit)?;

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("WhisprCatch")
            .with_icon(mic_icon())
            .with_icon_as_template(true)
            .build()?;
        // Keep the tray alive for the whole process.
        std::mem::forget(tray);

        // Menu clicks arrive on a global channel; handle them off the main loop.
        let st = state.clone();
        std::thread::spawn(move || {
            let rx = MenuEvent::receiver();
            while let Ok(ev) = rx.recv() {
                if ev.id == "toggle" {
                    let now = !st.is_enabled();
                    st.enabled.store(now, std::sync::atomic::Ordering::Relaxed);
                    log::info!("listening {}", if now { "enabled" } else { "disabled" });
                } else if ev.id == "history" {
                    if let Err(e) = std::process::Command::new(&exe).arg("settings").spawn() {
                        log::error!("failed to open history: {e}");
                    }
                } else if ev.id == "prefs" {
                    if let Err(e) = std::process::Command::new(&exe)
                        .args(["settings", "--tab", "settings"])
                        .spawn()
                    {
                        log::error!("failed to open preferences: {e}");
                    }
                } else if ev.id == "quit" {
                    std::process::exit(0);
                }
            }
        });

        app.run();
        Ok(())
    }

    /// An 18×18 push-to-talk mic glyph rendered as a template image (adapts
    /// to light/dark menu bars). The real app icon ships in the `.app` bundle.
    fn mic_icon() -> Icon {
        let (w, h) = (18u32, 18u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        let mut set = |x: i32, y: i32, a: u8| {
            if (0..w as i32).contains(&x) && (0..h as i32).contains(&y) {
                let i = ((y as u32 * w + x as u32) * 4) as usize;
                rgba[i + 3] = rgba[i + 3].max(a);
            }
        };
        // capsule body: rounded rect x∈[6.5,11.5], y∈[1.5,9.5]
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                // distance to capsule core segment (x=9, y from 4 to 7.5)
                let cy = fy.clamp(4.0, 7.5);
                let d = ((fx - 9.0).powi(2) + (fy - cy).powi(2)).sqrt();
                if d <= 2.6 {
                    set(x, y, 255);
                }
                // holder arc: ring r∈[4.6,5.8] around (9,8), lower half
                let dr = ((fx - 9.0).powi(2) + (fy - 8.0).powi(2)).sqrt();
                if fy >= 8.0 && (4.6..=5.8).contains(&dr) {
                    set(x, y, 255);
                }
            }
        }
        // stem + base
        for y in 14..16 {
            set(8, y, 255);
            set(9, y, 255);
        }
        for x in 6..12 {
            set(x, 16, 255);
        }
        Icon::from_rgba(rgba, w, h).expect("valid tray icon")
    }
}

#[cfg(target_os = "macos")]
pub use macos::{run_main, spawn, TrayHandle};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub struct TrayHandle;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl TrayHandle {
    pub fn refresh(&self) {}
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn spawn(_state: Arc<AppState>, _info: TrayInfo) -> Result<TrayHandle> {
    anyhow::bail!("tray not implemented for this platform yet")
}
