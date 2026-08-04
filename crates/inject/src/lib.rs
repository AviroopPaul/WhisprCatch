//! Types transcribed text into the focused window.
//!
//! X11: enigo → XTEST, works everywhere. Wayland: in-process uinput virtual
//! keyboard with a layout-aware keymap (SCOPE.md §1 cascade — XTEST never
//! reaches Wayland-native windows). If /dev/uinput isn't writable we fall
//! back to XTEST so XWayland windows still work.

use anyhow::{Context, Result};
use enigo::{Enigo, Keyboard, Settings};

#[cfg(target_os = "linux")]
pub mod layouts;
#[cfg(target_os = "linux")]
mod uinput;

/// Name of the uinput device the Wayland backend creates.
///
/// Public because the daemon has to hand it to the hotkey listener: kernel
/// injection is indistinguishable from real typing, so an evdev reader that
/// doesn't skip this device feeds our own keystrokes back into the PTT state
/// machine. Defined here because this crate creates the device — anyone who
/// needs to recognise it should ask, not re-spell it.
pub const VIRTUAL_KEYBOARD_NAME: &str = "whisper-catch virtual keyboard";

enum Backend {
    #[cfg(target_os = "linux")]
    Uinput(uinput::UinputKeyboard),
    Enigo(Enigo),
}

pub struct Injector {
    backend: Backend,
    #[cfg(target_os = "linux")]
    x11: Option<x11rb::rust_connection::RustConnection>,
}

impl Injector {
    /// `layout` is the user's explicit XKB layout choice (`"gb"`,
    /// `"us+dvorak"`); `None` detects the session's layout. Only the Wayland
    /// uinput backend consults it — XTEST and macOS type Unicode directly.
    pub fn new(layout: Option<&str>) -> Result<Self> {
        let backend = Self::pick_backend(layout)?;
        #[cfg(target_os = "linux")]
        let x11 = x11rb::connect(None).ok().map(|(c, _)| c);
        Ok(Self {
            backend,
            #[cfg(target_os = "linux")]
            x11,
        })
    }

    #[cfg(target_os = "linux")]
    fn pick_backend(layout: Option<&str>) -> Result<Backend> {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            match uinput::UinputKeyboard::new(layout) {
                Ok(kb) => {
                    log::info!("injector: uinput virtual keyboard (Wayland)");
                    return Ok(Backend::Uinput(kb));
                }
                Err(e) => log::warn!(
                    "injector: uinput unavailable ({e:#}); falling back to XTEST — \
                     text will only reach XWayland windows"
                ),
            }
        }
        Ok(Backend::Enigo(Self::new_enigo()?))
    }

    #[cfg(not(target_os = "linux"))]
    fn pick_backend(_layout: Option<&str>) -> Result<Backend> {
        Ok(Backend::Enigo(Self::new_enigo()?))
    }

    fn new_enigo() -> Result<Enigo> {
        Enigo::new(&Settings::default())
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("initializing input injection")
    }

    pub fn type_text(&mut self, text: &str) -> Result<()> {
        match &mut self.backend {
            #[cfg(target_os = "linux")]
            Backend::Uinput(kb) => kb.type_text(text).context("typing text (uinput)"),
            Backend::Enigo(enigo) => enigo
                .text(text)
                .map_err(|e| anyhow::anyhow!("{e}"))
                .context("typing text"),
        }
    }

    /// Fakes a release of a physically-held key at the input-stack level,
    /// so text injected *while the PTT modifier is held* doesn't turn into
    /// modifier+letter shortcuts. The kernel-level evdev listener still sees
    /// the real release later. Best-effort.
    #[cfg(target_os = "linux")]
    pub fn lift_key(&mut self, evdev_code: u16) {
        if let Backend::Uinput(kb) = &mut self.backend {
            kb.lift_key(evdev_code);
            return;
        }
        use x11rb::protocol::xtest::ConnectionExt as _;
        if let Some(conn) = &self.x11 {
            let keycode = (evdev_code + 8) as u8;
            // 3 = KeyRelease
            let _ = conn.xtest_fake_input(3, keycode, x11rb::CURRENT_TIME, 0u32, 0, 0, 0);
            let _ = x11rb::connection::Connection::flush(conn);
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn lift_key(&mut self, _evdev_code: u16) {}
}
