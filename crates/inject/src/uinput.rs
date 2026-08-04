//! Wayland text injection: an in-process uinput virtual keyboard with a
//! layout-aware keymap (the dotool model — SCOPE.md §1 "Text injection").
//!
//! XTEST only reaches XWayland windows; Wayland-native ones (GNOME Console,
//! Ghostty, …) never see it. Events written to /dev/uinput enter the kernel
//! input layer, so every compositor delivers them like real typing. The
//! compositor interprets our keycodes through the user's active XKB layout,
//! so we reverse that same layout (char → keycode + modifiers) instead of
//! assuming US like `ydotool type` does.
//!
//! Needs write access to /dev/uinput (the packaged udev rule + `input` group
//! grant this). If the device can't be created the caller falls back to XTEST.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use evdev::{
    uinput::VirtualDevice,
    AttributeSet, EventType, InputEvent, KeyCode,
};
use xkbcommon::xkb;

/// Modifier keys needed to reach a shift level, pressed around the base key.
#[derive(Clone, Copy, Default)]
struct Mods {
    shift: bool,
    altgr: bool,
}

/// How a single character is produced on the active layout.
#[derive(Clone, Copy)]
struct KeyCombo {
    code: u16,
    mods: Mods,
}

pub struct UinputKeyboard {
    device: VirtualDevice,
    keymap: HashMap<char, KeyCombo>,
}

/// Delay between injected key events. Compositors coalesce input in frames;
/// a small gap keeps ordering deterministic under load.
const KEY_DELAY: Duration = Duration::from_millis(2);

const PRESS: i32 = 1;
const RELEASE: i32 = 0;

impl UinputKeyboard {
    /// `layout` is the user's explicit XKB choice (`"gb"`, `"us+dvorak"`);
    /// `None` falls back to detecting the session's layout.
    pub fn new(layout: Option<&str>) -> Result<Self> {
        let keymap = build_reverse_keymap(layout).context("building reverse XKB keymap")?;

        let mut keys = AttributeSet::<KeyCode>::new();
        // Register the whole keyboard range: the reverse keymap covers typing,
        // but lift_key() must be able to release any configurable PTT key.
        for code in 1..=248 {
            keys.insert(KeyCode::new(code));
        }
        let device = VirtualDevice::builder()
            .context("opening /dev/uinput")?
            .name("whisper-catch virtual keyboard")
            .with_keys(&keys)
            .context("registering keys")?
            .build()
            .context("creating uinput device")?;

        // Give the compositor a moment to bind the new input device; events
        // sent before it attaches are dropped silently.
        std::thread::sleep(Duration::from_millis(200));

        Ok(Self { device, keymap })
    }

    pub fn type_text(&mut self, text: &str) -> Result<()> {
        for ch in text.chars().flat_map(ascii_fallback) {
            let combo = *self
                .keymap
                .get(&ch)
                .with_context(|| format!("no key for {ch:?} on the active layout"))?;
            self.tap(combo)?;
        }
        Ok(())
    }

    /// Releases a (physically held) key on the virtual device. The compositor
    /// tracks modifier state per seat, so a release from any device clears it;
    /// the eventual real release is a no-op.
    pub fn lift_key(&mut self, evdev_code: u16) {
        let _ = self.emit(evdev_code, RELEASE);
    }

    fn tap(&mut self, combo: KeyCombo) -> Result<()> {
        if combo.mods.shift {
            self.emit(KeyCode::KEY_LEFTSHIFT.code(), PRESS)?;
        }
        if combo.mods.altgr {
            self.emit(KeyCode::KEY_RIGHTALT.code(), PRESS)?;
        }
        self.emit(combo.code, PRESS)?;
        self.emit(combo.code, RELEASE)?;
        if combo.mods.altgr {
            self.emit(KeyCode::KEY_RIGHTALT.code(), RELEASE)?;
        }
        if combo.mods.shift {
            self.emit(KeyCode::KEY_LEFTSHIFT.code(), RELEASE)?;
        }
        Ok(())
    }

    fn emit(&mut self, code: u16, value: i32) -> Result<()> {
        self.device
            .emit(&[InputEvent::new(EventType::KEY.0, code, value)])
            .context("writing input event")?;
        std::thread::sleep(KEY_DELAY);
        Ok(())
    }
}

/// Transcripts occasionally carry typographic punctuation that no keyboard
/// layout can type directly; downgrade it rather than fail the whole line.
fn ascii_fallback(ch: char) -> impl Iterator<Item = char> {
    let mapped: &str = match ch {
        '\u{2018}' | '\u{2019}' => "'",
        '\u{201C}' | '\u{201D}' => "\"",
        '\u{2013}' | '\u{2014}' => "-",
        '\u{2026}' => "...",
        '\u{00A0}' => " ",
        _ => return OneOrStr::One(ch),
    };
    OneOrStr::Str(mapped.chars())
}

enum OneOrStr {
    One(char),
    Str(std::str::Chars<'static>),
}

impl Iterator for OneOrStr {
    type Item = char;
    fn next(&mut self) -> Option<char> {
        match self {
            OneOrStr::One(c) => {
                let c = *c;
                *self = OneOrStr::Str("".chars());
                Some(c)
            }
            OneOrStr::Str(it) => it.next(),
        }
    }
}

/// char → (keycode, modifiers) for the user's active layout.
///
/// Levels follow the conventional XKB ordering (dotool does the same):
/// 0 = plain, 1 = Shift, 2 = AltGr, 3 = Shift+AltGr. First win is kept, so
/// unshifted positions beat shifted duplicates.
fn build_reverse_keymap(chosen: Option<&str>) -> Result<HashMap<char, KeyCombo>> {
    let (layout, variant) = crate::layouts::resolve(chosen);
    log::info!("uinput injector: using XKB layout {layout:?} variant {variant:?}");

    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_names(
        &ctx,
        "",
        "",
        &layout,
        &variant,
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .with_context(|| format!("compiling keymap for layout {layout:?}"))?;

    let mut map = HashMap::new();
    // Newline isn't a keysym product; wire it up explicitly.
    map.insert(
        '\n',
        KeyCombo {
            code: KeyCode::KEY_ENTER.code(),
            mods: Mods::default(),
        },
    );
    map.insert(
        '\t',
        KeyCombo {
            code: KeyCode::KEY_TAB.code(),
            mods: Mods::default(),
        },
    );

    let levels = [
        Mods::default(),
        Mods {
            shift: true,
            altgr: false,
        },
        Mods {
            shift: false,
            altgr: true,
        },
        Mods {
            shift: true,
            altgr: true,
        },
    ];

    // Levels outermost so a char reachable at several positions keeps the
    // simplest chord (plain beats Shift beats AltGr).
    for (level, mods) in levels.iter().enumerate() {
        for keycode in keymap.min_keycode().raw()..=keymap.max_keycode().raw() {
            let kc = xkb::Keycode::new(keycode);
            for keysym in keymap.key_get_syms_by_level(kc, 0, level as u32) {
                let cp = xkb::keysym_to_utf32(*keysym);
                if cp == 0 {
                    continue;
                }
                let Some(ch) = char::from_u32(cp) else {
                    continue;
                };
                if ch.is_control() {
                    continue;
                }
                // XKB keycodes are evdev + 8
                let code = (keycode - 8) as u16;
                map.entry(ch).or_insert(KeyCombo { code, mods: *mods });
            }
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo(map: &HashMap<char, KeyCombo>, ch: char) -> (u16, bool, bool) {
        let c = map.get(&ch).unwrap_or_else(|| panic!("no mapping for {ch:?}"));
        (c.code, c.mods.shift, c.mods.altgr)
    }

    /// UK layout: the mappings a US-assuming injector gets wrong.
    #[test]
    fn gb_layout_reverse_map() {
        let map = build_reverse_keymap(Some("gb")).unwrap();

        assert_eq!(combo(&map, 'a'), (KeyCode::KEY_A.code(), false, false));
        assert_eq!(combo(&map, 'A'), (KeyCode::KEY_A.code(), true, false));
        // gb: double-quote is Shift+2, @ is Shift+apostrophe, £ is Shift+3
        assert_eq!(combo(&map, '"'), (KeyCode::KEY_2.code(), true, false));
        assert_eq!(combo(&map, '@'), (KeyCode::KEY_APOSTROPHE.code(), true, false));
        assert_eq!(combo(&map, '\u{00A3}'), (KeyCode::KEY_3.code(), true, false));
        assert_eq!(combo(&map, '\n'), (KeyCode::KEY_ENTER.code(), false, false));
    }
}
