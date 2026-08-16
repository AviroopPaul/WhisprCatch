//! Global push-to-talk hotkey via raw evdev (Linux).
//!
//! Listen-only: never grabs the device, so a key also bound in the
//! compositor will fire both — pick a low-conflict key (default Right-Ctrl).
//! Requires read access to /dev/input/event* (`input` group membership).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;

use anyhow::{bail, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PttEvent {
    Pressed,
    Released,
}

/// A running push-to-talk listener: the press/release stream, plus one bit of
/// context about everything else the keyboard did.
pub struct Listener {
    /// Press and release for the configured key, and nothing else.
    pub events: Receiver<PttEvent>,
    others: Arc<AtomicU64>,
}

impl Listener {
    /// How many times a key that is **not** the push-to-talk key has been
    /// pressed since the listener started.
    ///
    /// Sampled at press and compared at release, this answers exactly one
    /// question — *did the user type something of their own while holding the
    /// key?* — and it is the difference between a streaming replace that
    /// corrects our own words and one that deletes theirs.
    ///
    /// **This collects nothing that was not already in the process.** The
    /// Linux listener reads every `EV_KEY` event from every keyboard and
    /// filters in userspace; adding a `!=` to that loop is not new access, no
    /// keycode is retained, and the counter never leaves the process.
    ///
    /// It is **0 on macOS whenever the push-to-talk key is a modifier**, which
    /// is the default there: that tap subscribes to `FlagsChanged` only and
    /// genuinely cannot see other keys. Widening it to `KeyDown` would be new
    /// collection, so it is not done — a caller must treat 0 as "no
    /// information", never as "nothing happened".
    pub fn other_key_presses(&self) -> u64 {
        self.others.load(Ordering::Relaxed)
    }
}

/// Named keys we support as PTT triggers. Maps to evdev codes on Linux and to
/// virtual keycodes / modifier flags on macOS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PttKey {
    RightCtrl,
    LeftCtrl,
    RightAlt,
    LeftAlt,
    RightCommand,
    LeftCommand,
    Super,
    F13,
    ScrollLock,
}

impl PttKey {
    /// Kernel evdev key code (X11 keycode = this + 8).
    pub fn evdev_code(self) -> u16 {
        match self {
            Self::RightCtrl => 97,
            Self::LeftCtrl => 29,
            Self::RightAlt => 100,
            Self::LeftAlt => 56,
            Self::RightCommand => 126, // KEY_RIGHTMETA
            Self::LeftCommand | Self::Super => 125, // KEY_LEFTMETA
            Self::F13 => 183,
            Self::ScrollLock => 70,
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "rightctrl" | "rctrl" => Self::RightCtrl,
            "leftctrl" | "lctrl" => Self::LeftCtrl,
            "rightalt" | "ralt" | "rightoption" | "ropt" => Self::RightAlt,
            "leftalt" | "lalt" | "leftoption" | "lopt" => Self::LeftAlt,
            "rightcommand" | "rightcmd" | "rcmd" | "cmd" | "command" => Self::RightCommand,
            "leftcommand" | "leftcmd" | "lcmd" => Self::LeftCommand,
            "super" | "meta" | "win" => Self::Super,
            "f13" => Self::F13,
            "scrolllock" => Self::ScrollLock,
            other => bail!(
                "unknown PTT key '{other}' (try: rctrl, lctrl, ralt, lalt, rcmd, lcmd, super, f13, scrolllock)"
            ),
        })
    }
}

/// True when at least one keyboard-capable input device is readable —
/// i.e. hotkey listening will work without further permission setup.
#[cfg(target_os = "linux")]
pub fn keyboard_accessible() -> bool {
    evdev::enumerate().any(|(_, d)| {
        d.supported_keys()
            .map(|k| k.contains(evdev::KeyCode::KEY_A))
            .unwrap_or(false)
    })
}

/// macOS: keyboard access needs Accessibility (injection) + an event tap
/// (Input Monitoring). `AXIsProcessTrusted()` is our proxy for "granted".
#[cfg(target_os = "macos")]
pub fn keyboard_accessible() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }
    unsafe { AXIsProcessTrusted() }
}

/// macOS only: whether Input Monitoring is granted (what the CGEventTap needs).
/// This is a separate TCC permission from Accessibility — a user can grant one
/// and not the other, which looks like "the hotkey does nothing". No-op elsewhere.
#[cfg(target_os = "macos")]
pub fn input_monitoring_granted() -> bool {
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOHIDCheckAccess(request_type: u32) -> u32;
    }
    const K_IOHID_REQUEST_TYPE_LISTEN_EVENT: u32 = 1;
    const K_IOHID_ACCESS_TYPE_GRANTED: u32 = 0;
    unsafe { IOHIDCheckAccess(K_IOHID_REQUEST_TYPE_LISTEN_EVENT) == K_IOHID_ACCESS_TYPE_GRANTED }
}

#[cfg(not(target_os = "macos"))]
pub fn input_monitoring_granted() -> bool {
    keyboard_accessible()
}

/// macOS only: show the system Accessibility prompt, which also registers
/// WhisprCatch in System Settings › Privacy › Accessibility so the user can
/// toggle it on. Returns the current trust state. No-op elsewhere.
#[cfg(target_os = "macos")]
pub fn request_accessibility() -> bool {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::string::{CFString, CFStringRef};

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        static kAXTrustedCheckOptionPrompt: CFStringRef;
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    }
    unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let opts = CFDictionary::from_CFType_pairs(&[(
            key.as_CFType(),
            CFBoolean::true_value().as_CFType(),
        )]);
        AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef())
    }
}

#[cfg(not(target_os = "macos"))]
pub fn request_accessibility() -> bool {
    keyboard_accessible()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn keyboard_accessible() -> bool {
    false
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use evdev::{Device, EventSummary, KeyCode};

    impl PttKey {
        fn code(self) -> KeyCode {
            match self {
                PttKey::RightCtrl => KeyCode::KEY_RIGHTCTRL,
                PttKey::LeftCtrl => KeyCode::KEY_LEFTCTRL,
                PttKey::RightAlt => KeyCode::KEY_RIGHTALT,
                PttKey::LeftAlt => KeyCode::KEY_LEFTALT,
                PttKey::RightCommand => KeyCode::KEY_RIGHTMETA,
                PttKey::LeftCommand | PttKey::Super => KeyCode::KEY_LEFTMETA,
                PttKey::F13 => KeyCode::KEY_F13,
                PttKey::ScrollLock => KeyCode::KEY_SCROLLLOCK,
            }
        }
    }

    /// Spawns one reader thread per keyboard-capable device; events funnel
    /// into a single channel. No hotplug rescan yet (MVP).
    pub fn listen(key: PttKey) -> Result<Listener> {
        let code = key.code();
        let devices: Vec<(std::path::PathBuf, Device)> = evdev::enumerate()
            .filter(|(_, d)| {
                d.supported_keys()
                    .map(|keys| keys.contains(code))
                    .unwrap_or(false)
            })
            .collect();

        if devices.is_empty() {
            bail!(
                "no readable input device supports {:?} — are you in the `input` group? \
                 (sudo usermod -aG input $USER, then re-login)",
                key
            );
        }

        let (tx, rx) = mpsc::channel();
        let others = Arc::new(AtomicU64::new(0));
        for (path, mut dev) in devices {
            log::info!("listening on {} ({})", path.display(), dev.name().unwrap_or("?"));
            let tx = tx.clone();
            let others = others.clone();
            thread::spawn(move || loop {
                let events = match dev.fetch_events() {
                    Ok(ev) => ev,
                    Err(e) => {
                        log::warn!("device read failed (unplugged?): {e}");
                        return;
                    }
                };
                for ev in events {
                    if let EventSummary::Key(_, c, value) = ev.destructure() {
                        if c == code {
                            // value: 1 = press, 0 = release, 2 = autorepeat (ignore)
                            let msg = match value {
                                1 => Some(PttEvent::Pressed),
                                0 => Some(PttEvent::Released),
                                _ => None,
                            };
                            if let Some(m) = msg {
                                if tx.send(m).is_err() {
                                    return;
                                }
                            }
                        } else if value == 1 {
                            // Somebody else's key went down. The keycode is
                            // already in this loop and is deliberately not
                            // looked at, kept or sent anywhere — only the
                            // count leaves this line. See `other_key_presses`.
                            others.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            });
        }
        Ok(Listener { events: rx, others })
    }
}

#[cfg(target_os = "linux")]
pub use linux::listen;

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use core_foundation::base::TCFType;
    use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
    use core_graphics::event::{
        CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
        CGEventType, CallbackResult, EventField,
    };

    // Re-enable a tap the system disabled (slow callback / user input). Not
    // exported by core-graphics, so we bind it directly.
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    }

    impl PttKey {
        /// (macOS virtual keycode, Some(modifier mask) for a modifier key or
        /// None for a normal key). Modifiers arrive as `FlagsChanged`; normal
        /// keys as `KeyDown`/`KeyUp`.
        fn mac(self) -> (i64, Option<CGEventFlags>) {
            match self {
                PttKey::RightCtrl => (62, Some(CGEventFlags::CGEventFlagControl)),
                PttKey::LeftCtrl => (59, Some(CGEventFlags::CGEventFlagControl)),
                PttKey::RightAlt => (61, Some(CGEventFlags::CGEventFlagAlternate)),
                PttKey::LeftAlt => (58, Some(CGEventFlags::CGEventFlagAlternate)),
                PttKey::RightCommand => (54, Some(CGEventFlags::CGEventFlagCommand)),
                PttKey::LeftCommand | PttKey::Super => {
                    (55, Some(CGEventFlags::CGEventFlagCommand))
                }
                PttKey::F13 => (105, None),
                PttKey::ScrollLock => (107, None), // F14 — no Scroll Lock on macOS
            }
        }
    }

    /// Installs a listen-only CGEventTap on a dedicated CFRunLoop thread and
    /// funnels press/release for `key` into a channel — mirroring the Linux
    /// evdev path so the daemon loop is identical across platforms.
    ///
    /// The set of event types below is **not** widened to observe other keys.
    /// With a modifier push-to-talk key — the default here — the tap sees
    /// `FlagsChanged` and nothing else, so `Listener::other_key_presses` stays
    /// 0 and the caller gets no information rather than wrong information.
    /// Subscribing to `KeyDown` to fill it in would put every keystroke on the
    /// machine through this callback, which is a trade this project does not
    /// make for a rare mis-splice.
    pub fn listen(key: PttKey) -> Result<Listener> {
        let (tx, rx) = mpsc::channel();
        let (keycode, mask) = key.mac();
        let others = Arc::new(AtomicU64::new(0));
        let others_cb = others.clone();

        thread::spawn(move || {
            let events = if mask.is_some() {
                vec![CGEventType::FlagsChanged]
            } else {
                vec![CGEventType::KeyDown, CGEventType::KeyUp]
            };

            // holds the mach-port pointer so the callback can re-enable itself
            let port: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
            let port_cb = port.clone();

            let tap = CGEventTap::new(
                CGEventTapLocation::HID,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::ListenOnly,
                events,
                move |_proxy, etype, event| {
                    let code = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
                    match etype {
                        CGEventType::FlagsChanged => {
                            if code == keycode {
                                if let Some(m) = mask {
                                    let down = event.get_flags().contains(m);
                                    let _ = tx.send(if down {
                                        PttEvent::Pressed
                                    } else {
                                        PttEvent::Released
                                    });
                                }
                            }
                        }
                        CGEventType::KeyDown => {
                            if code == keycode {
                                let _ = tx.send(PttEvent::Pressed);
                            } else {
                                // Only reachable when the push-to-talk key is
                                // *not* a modifier, where this tap already
                                // receives every KeyDown. Nothing new is
                                // subscribed to in order to count it.
                                others_cb.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        CGEventType::KeyUp => {
                            if code == keycode {
                                let _ = tx.send(PttEvent::Released);
                            }
                        }
                        CGEventType::TapDisabledByTimeout
                        | CGEventType::TapDisabledByUserInput => {
                            let p = port_cb.load(Ordering::Relaxed);
                            if p != 0 {
                                unsafe { CGEventTapEnable(p as *mut c_void, true) };
                                log::warn!("event tap re-enabled after {:?}", etype);
                            }
                        }
                        _ => {}
                    }
                    CallbackResult::Keep
                },
            );

            let tap = match tap {
                Ok(t) => t,
                Err(()) => {
                    log::error!(
                        "could not create the keyboard event tap — grant WhisprCatch \
                         Accessibility and Input Monitoring in System Settings › Privacy"
                    );
                    return;
                }
            };

            port.store(tap.mach_port().as_concrete_TypeRef() as usize, Ordering::Relaxed);
            let source = match tap.mach_port().create_runloop_source(0) {
                Ok(s) => s,
                Err(()) => {
                    log::error!("failed to create run-loop source for event tap");
                    return;
                }
            };
            CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopCommonModes });
            tap.enable();
            log::info!("macOS event tap installed for {:?} (keycode {})", key, keycode);
            CFRunLoop::run_current();
        });

        Ok(Listener { events: rx, others })
    }
}

#[cfg(target_os = "macos")]
pub use macos::listen;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn listen(_key: PttKey) -> Result<Listener> {
    bail!("hotkey listener not implemented for this platform yet")
}
