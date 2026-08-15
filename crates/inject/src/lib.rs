//! Types transcribed text into the focused window.
//!
//! X11: enigo → XTEST, works everywhere. Wayland: enigo tries libei/portal,
//! flaky — the uinput virtual-keyboard cascade lands post-MVP (SCOPE.md §3).
//!
//! # Taking text back
//!
//! [`Injector::replace_last`] removes text this injector typed and puts
//! something else in its place: the foundation for undo (#42), marked
//! self-correction (#48) and rewriting under live streaming (#50).
//!
//! Every decision it makes — how many Backspace presses, what to retype, what
//! not to touch — lives in [`plan`], which is pure and never sees a display
//! server. The platform code here is only a translator: it turns a
//! [`plan::Action`] into the local idea of a keystroke and does no arithmetic
//! of its own. That split is the point. Synthetic input cannot be observed
//! from a test runner with no compositor, so the part that can be *got wrong*
//! is kept where it can be tested exhaustively, and a new backend inherits all
//! of it by implementing three small methods ([`plan::KeyboardSink`]).
//!
//! ## Two units, on purpose
//!
//! `replace_last` counts **chars**, because that is what a caller can compute
//! for free and cannot get wrong. The keyboard counts **grapheme clusters**,
//! because that is what one Backspace removes. Converting between them needs
//! the characters themselves, so the injector keeps a bounded record of what it
//! typed ([`plan::Typed`]) and resolves the count against that. One consequence
//! is worth stating plainly: **a replace never sends more backspaces than the
//! text it knows it typed**, so an over-large request stops at our own text
//! instead of eating the user's.
//!
//! ## No pasteboard path yet
//!
//! [`plan`] knows the 200-char paste threshold and is tested at its boundary,
//! but every backend here reports [`plan::Capabilities::TYPING_ONLY`], so no
//! [`plan::Action::Paste`] is ever emitted. Nothing in this crate has ever
//! touched the clipboard. Adding it means setting the user's clipboard aside,
//! synthesising ⌘V/Ctrl+V — a modifier chord, i.e. the exact bug class this
//! crate exists to dodge — and getting it wrong in terminals, where paste is
//! Ctrl+Shift+V. That is its own change with its own manual testing (#68), not
//! something to smuggle in under "replace". The planner is ready for it: set
//! `paste` in a backend's capabilities and implement
//! [`plan::KeyboardSink::send_paste`].

pub mod plan;
mod unicode;

// Linux-only at runtime, compiled everywhere so its tests run on both CI
// runners — the X11 path is otherwise untestable from a macOS machine.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod x11keys;

use anyhow::{Context, Result};
use enigo::{Enigo, Settings};
// macOS types via CGEvent instead, so the Keyboard trait is unused there.
#[cfg(not(target_os = "macos"))]
use enigo::Keyboard;

use plan::{Capabilities, KeyboardSink, PlanOpts, Typed};

pub use plan::{Action, Plan, PASTE_THRESHOLD, TYPED_MEMORY_CHARS};

/// Largest string a single `CGEventKeyboardSetUnicodeString` is documented to
/// carry, in UTF-16 code units. Units, not chars: one emoji is two.
#[cfg(target_os = "macos")]
const MACOS_EVENT_UTF16_UNITS: usize = 20;

/// macOS virtual keycode for the Backspace key (`kVK_Delete`).
#[cfg(target_os = "macos")]
const MACOS_BACKSPACE: u16 = 0x33;

pub struct Injector {
    /// Kept on macOS too: constructing it is our Accessibility permission check,
    /// and it produces the actionable "does not have the permission to simulate
    /// input" error at startup. Typing goes through CGEvent — see `type_raw`.
    #[allow(dead_code)]
    enigo: Enigo,
    #[cfg(target_os = "macos")]
    source: core_graphics::event_source::CGEventSource,
    #[cfg(target_os = "linux")]
    x11: Option<x11rb::rust_connection::RustConnection>,
    /// What we have put on screen, so `replace_last` can turn a char count back
    /// into text. Bounded; see [`plan::Typed`].
    typed: Typed,
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
            typed: Typed::new(),
        })
    }

    // The platform bodies below are `_raw`: they put characters on screen and
    // nothing else. Everything that has to happen *around* them — updating the
    // record of what we typed, planning a replace — sits in the public methods
    // further down, so a new backend only has to add cases here.

    #[cfg(not(target_os = "macos"))]
    fn type_raw(&mut self, text: &str) -> Result<()> {
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
    fn type_raw(&mut self, text: &str) -> Result<()> {
        use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};

        // NB: writing the focused element's AXSelectedText was tried here and is
        // not usable — AXUIElementSetAttributeValue returns success while
        // inserting nothing, so there is no way to know when to fall back.

        // The limit is per event and counted in UTF-16 units, so chunking by
        // chars would send a 32-unit event for 16 emoji.
        for chunk in unicode::utf16_chunks(text, MACOS_EVENT_UTF16_UNITS) {
            for down in [true, false] {
                let event = CGEvent::new_keyboard_event(self.source.clone(), 0, down)
                    .map_err(|()| anyhow::anyhow!("creating keyboard event"))?;
                event.set_string(chunk);
                event.set_flags(CGEventFlags::CGEventFlagNull);
                event.post(CGEventTapLocation::Session);
            }
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    fn backspace_raw(&mut self, count: usize) -> Result<()> {
        for _ in 0..count {
            self.enigo
                .key(enigo::Key::Backspace, enigo::Direction::Click)
                .map_err(|e| anyhow::anyhow!("{e}"))
                .context("sending backspace")?;
        }
        Ok(())
    }

    /// Post Backspace as key-down/key-up pairs with the flags forced empty, for
    /// the same reason `type_raw` does — an inherited ⌥ turns every press into
    /// a delete-word.
    #[cfg(target_os = "macos")]
    fn backspace_raw(&mut self, count: usize) -> Result<()> {
        use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};

        for _ in 0..count {
            for down in [true, false] {
                let event = CGEvent::new_keyboard_event(self.source.clone(), MACOS_BACKSPACE, down)
                    .map_err(|()| anyhow::anyhow!("creating keyboard event"))
                    .context("sending backspace")?;
                event.set_flags(CGEventFlags::CGEventFlagNull);
                event.post(CGEventTapLocation::Session);
            }
        }
        Ok(())
    }

    /// X11: release every modifier keycode the server reports as down, before
    /// we send anything.
    ///
    /// The push-to-talk key is usually a modifier (Right-Ctrl by default) and
    /// evdev is listen-only, so we cannot consume it. Sending Backspace while
    /// Ctrl is still held is not a no-op: in most editors Ctrl/⌥+Backspace
    /// deletes a whole word, so a five-press correction can take out a
    /// sentence. Lifting first is what stops that.
    ///
    /// Best-effort by design. On Wayland there is no XTEST connection and no
    /// way to fake a release, so this reports success having done nothing —
    /// failing here would make `replace_last` unusable on Wayland, which is
    /// worse than the risk it guards against.
    #[cfg(target_os = "linux")]
    fn lift_held_modifiers(&mut self) -> Result<()> {
        use x11rb::connection::Connection as _;
        use x11rb::protocol::xproto::ConnectionExt as _;
        use x11rb::protocol::xtest::ConnectionExt as _;

        let Some(conn) = &self.x11 else {
            return Ok(());
        };
        let held = (|| -> Result<Vec<u8>> {
            let keymap = conn.query_keymap()?.reply()?;
            let modmap = conn.get_modifier_mapping()?.reply()?;
            Ok(x11keys::modifiers_down(&keymap.keys, &modmap.keycodes))
        })();
        let held = match held {
            Ok(held) => held,
            Err(e) => {
                log::debug!("could not read modifier state: {e:#}");
                return Ok(());
            }
        };
        for keycode in held {
            // 3 = KeyRelease
            let _ = conn.xtest_fake_input(3, keycode, x11rb::CURRENT_TIME, 0u32, 0, 0, 0);
        }
        let _ = conn.flush();
        Ok(())
    }

    /// macOS has no equivalent of the X11 fake-release: you cannot tell the
    /// window server that a physically held key is up. The protection comes
    /// instead from every event we post carrying `CGEventFlagNull` and coming
    /// from a private event source — which `type_raw` and `backspace_raw`
    /// already do, so by the time this is called there is nothing to lift.
    #[cfg(not(target_os = "linux"))]
    fn lift_held_modifiers(&mut self) -> Result<()> {
        Ok(())
    }

    /// Type `text` at the cursor.
    pub fn type_text(&mut self, text: &str) -> Result<()> {
        match self.type_raw(text) {
            Ok(()) => {
                self.typed.record(text);
                Ok(())
            }
            // A failed type may have landed in part. Anything we thought we
            // knew about the screen is a guess now, and guessing is how a later
            // replace deletes someone's paragraph.
            Err(e) => {
                self.typed.forget();
                Err(e)
            }
        }
    }

    /// Take back the last `n_chars` chars this injector typed, and put
    /// `new_text` there instead.
    ///
    /// `n_chars` counts chars (`str::chars`) and is measured from the end of
    /// what *we* typed, not from the cursor — which may have moved without our
    /// knowing. It is clamped to the recorded text, so an over-large count
    /// removes everything we know we typed and stops; text the user wrote
    /// themselves is never touched.
    ///
    /// Only what actually changed is sent. After typing `"hello world"`,
    /// `replace_last(5, "there")` sends five backspaces and types `"there"`,
    /// and replacing text with itself sends nothing at all.
    ///
    /// On macOS this refuses while Secure Input is on; see
    /// [`secure_input_active`]. On any failure the record is dropped, so a
    /// later replace cannot be counted against a screen we are unsure of.
    pub fn replace_last(&mut self, n_chars: usize, new_text: &str) -> Result<()> {
        if secure_input_active() {
            self.typed.forget();
            anyhow::bail!(
                "secure input is enabled (a password field has focus); \
                 refusing to replace text"
            );
        }
        let opts = PlanOpts::from_capabilities(self.capabilities());
        let plan = self.typed.plan_replace(n_chars, new_text, opts);
        if plan.is_empty() {
            return Ok(());
        }
        if let Err(e) = plan.run(self) {
            self.typed.forget();
            return Err(e).context("replacing typed text");
        }
        Ok(())
    }

    /// How many chars [`Injector::replace_last`] can still reach back over.
    pub fn replaceable_chars(&self) -> usize {
        self.typed.known_chars()
    }

    /// Forget what we typed, so the next replace types instead of deleting.
    ///
    /// Callers should do this whenever the cursor may have left our text — a
    /// focus change, a click, a keystroke of the user's own. The injector can
    /// see none of those.
    pub fn forget_typed(&mut self) {
        self.typed.forget();
    }

    /// Fakes a release of a physically-held key at the display-server level,
    /// so text injected *while the PTT modifier is held* doesn't turn into
    /// modifier+letter shortcuts. The kernel-level evdev listener still sees
    /// the real release later. Best-effort; no-op off X11.
    ///
    /// Takes a specific key because the push-to-talk key need not be a modifier
    /// at all. `lift_held_modifiers` is the broader version used before a
    /// replace, and needs no keycode.
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

/// True when macOS Secure Event Input is on: a password field has focus, or an
/// app left it enabled. The OS drops synthetic keystrokes while it is, so a
/// replace would half-land at best — SCOPE.md §3 flags detecting it.
///
/// A replace refuses outright rather than trying. Deleting text and then
/// failing to type its replacement is worse than doing nothing, and the target
/// is by definition a field the user does not want a dictation daemon writing
/// into. Always false off macOS.
pub fn secure_input_active() -> bool {
    #[cfg(target_os = "macos")]
    {
        // `Boolean IsSecureEventInputEnabled(void)` lives in HIToolbox, which
        // ships as part of the Carbon umbrella framework. No crate needed.
        #[link(name = "Carbon", kind = "framework")]
        extern "C" {
            fn IsSecureEventInputEnabled() -> u8;
        }
        unsafe { IsSecureEventInputEnabled() != 0 }
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// The whole platform surface a [`plan::Plan`] needs. A backend that can do
/// these three things gets `replace_last` for free, with all of its counting
/// and its safety rules already tested.
impl KeyboardSink for Injector {
    /// Keystrokes only, on every platform: there is no clipboard path in this
    /// crate, so the planner must never choose one. See the crate docs.
    fn capabilities(&self) -> Capabilities {
        Capabilities::TYPING_ONLY
    }

    fn lift_modifiers(&mut self) -> Result<()> {
        self.lift_held_modifiers()
    }

    fn send_backspaces(&mut self, count: usize) -> Result<()> {
        self.backspace_raw(count)
    }

    fn send_text(&mut self, text: &str) -> Result<()> {
        self.type_raw(text)
    }
}
