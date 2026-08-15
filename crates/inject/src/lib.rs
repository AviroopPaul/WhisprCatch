//! Types transcribed text into the focused window.
//!
//! X11: enigo → XTEST, works everywhere. Wayland: enigo tries libei/portal,
//! flaky — the uinput virtual-keyboard cascade lands post-MVP (SCOPE.md §3).

use anyhow::{Context, Result};
use enigo::{Enigo, Settings};
// macOS types via CGEvent instead, so the Keyboard trait is unused there.
#[cfg(not(target_os = "macos"))]
use enigo::Keyboard;

pub struct Injector {
    /// Kept on macOS too: constructing it is our Accessibility permission check,
    /// and it produces the actionable "does not have the permission to simulate
    /// input" error at startup. Typing goes through CGEvent — see `type_text`.
    #[allow(dead_code)]
    enigo: Enigo,
    #[cfg(target_os = "macos")]
    source: core_graphics::event_source::CGEventSource,
    #[cfg(target_os = "linux")]
    x11: Option<x11rb::rust_connection::RustConnection>,
}

impl Injector {
    pub fn new() -> Result<Self> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("initializing input injection")?;
        // Private state, not HIDSystemState: a source tied to the HID system
        // carries the live hardware modifier state, which is exactly what we are
        // trying not to inherit while the push-to-talk key is held.
        #[cfg(target_os = "macos")]
        let source = core_graphics::event_source::CGEventSource::new(
            core_graphics::event_source::CGEventSourceStateID::Private,
        )
        .map_err(|()| anyhow::anyhow!("creating CGEventSource"))
        .context("initializing input injection")?;
        #[cfg(target_os = "linux")]
        let x11 = x11rb::connect(None).ok().map(|(c, _)| c);
        Ok(Self {
            enigo,
            #[cfg(target_os = "macos")]
            source,
            #[cfg(target_os = "linux")]
            x11,
        })
    }

    #[cfg(not(target_os = "macos"))]
    pub fn type_text(&mut self, text: &str) -> Result<()> {
        self.enigo
            .text(text)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("typing text")
    }

    /// macOS: post the text ourselves with the modifier flags forced empty.
    ///
    /// A posted CGEvent otherwise inherits the *physical* keyboard state, so
    /// text injected while the push-to-talk key is held arrives as ⌘/⌥/⌃+letter
    /// — the receiving app runs shortcuts instead of inserting characters, and
    /// the keystrokes are silently lost (the post still reports success). There
    /// is no macOS equivalent of the X11 `lift_key` trick; you cannot
    /// fake-release a physically held modifier.
    ///
    /// Two things are required, and clearing the flags alone is not enough:
    ///
    ///   * post to the **session** tap, not `HID`. An event posted at the HID
    ///     level is re-annotated with the live hardware modifier state further
    ///     down the chain, which puts ⌘ straight back on.
    ///   * build it from a **private** event source, so it carries no hardware
    ///     state to begin with.
    #[cfg(target_os = "macos")]
    pub fn type_text(&mut self, text: &str) -> Result<()> {
        use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};

        // NB: writing the focused element's AXSelectedText was tried here and is
        // not usable — AXUIElementSetAttributeValue returns success while
        // inserting nothing, so there is no way to know when to fall back.

        // CGEventKeyboardSetUnicodeString gets unreliable with long strings;
        // chunk well under any limit. Chunk by chars so UTF-8 stays intact.
        const CHUNK: usize = 16;
        let chars: Vec<char> = text.chars().collect();
        for chunk in chars.chunks(CHUNK) {
            let s: String = chunk.iter().collect();
            for down in [true, false] {
                let event = CGEvent::new_keyboard_event(self.source.clone(), 0, down)
                    .map_err(|()| anyhow::anyhow!("creating keyboard event"))?;
                event.set_string(&s);
                event.set_flags(CGEventFlags::CGEventFlagNull);
                event.post(CGEventTapLocation::Session);
            }
        }
        Ok(())
    }

    /// Fakes a release of a physically-held key at the display-server level,
    /// so text injected *while the PTT modifier is held* doesn't turn into
    /// modifier+letter shortcuts. The kernel-level evdev listener still sees
    /// the real release later. Best-effort; no-op off X11.
    #[cfg(target_os = "linux")]
    pub fn lift_key(&mut self, evdev_code: u16) {
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
