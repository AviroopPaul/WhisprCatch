//! Global push-to-talk hotkey via raw evdev (Linux).
//!
//! Listen-only: never grabs the device, so a key also bound in the
//! compositor will fire both — pick a low-conflict key (default Right-Ctrl).
//! Requires read access to /dev/input/event* (`input` group membership).

use std::sync::mpsc::{self, Receiver};
use std::thread;

use anyhow::{bail, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PttEvent {
    Pressed,
    Released,
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
    pub fn listen(key: PttKey, exclude_devices: &[&str]) -> Result<Receiver<PttEvent>> {
        let code = key.code();
        let devices: Vec<(std::path::PathBuf, Device)> = evdev::enumerate()
            // Devices the caller synthesises input on: their events must never
            // feed the PTT state machine. The daemon passes the injector's
            // virtual keyboard — this crate deliberately doesn't know what that
            // is, so renaming the device can't silently break the guard.
            .filter(|(_, d)| !exclude_devices.contains(&d.name().unwrap_or("")))
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
        for (path, mut dev) in devices {
            log::info!("listening on {} ({})", path.display(), dev.name().unwrap_or("?"));
            let tx = tx.clone();
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
                        }
                    }
                }
            });
        }
        Ok(rx)
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
    /// `exclude_devices` exists for parity with the Linux signature and is
    /// ignored: a CGEventTap observes the event stream, not input devices.
    pub fn listen(key: PttKey, _exclude_devices: &[&str]) -> Result<Receiver<PttEvent>> {
        let (tx, rx) = mpsc::channel();
        let (keycode, mask) = key.mac();

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

        Ok(rx)
    }
}

#[cfg(target_os = "macos")]
pub use macos::listen;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn listen(_key: PttKey) -> Result<Receiver<PttEvent>> {
    bail!("hotkey listener not implemented for this platform yet")
}
