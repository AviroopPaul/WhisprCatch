//! The bit-twiddling half of "lift the modifiers the user is still holding".
//!
//! Split out from the X11 call so it can be tested on a machine with no X
//! server — which includes both CI runners and every Mac.

/// Which of `modifiers` the server reports as physically down.
///
/// `keymap` is the 32-byte vector from `QueryKeymap`: 256 bits, one per
/// keycode, LSB first within each byte. `modifiers` is the flat keycode list
/// from `GetModifierMapping` — eight modifier groups of `keycodes_per_modifier`
/// entries each, padded with zeros for the slots a group does not use.
///
/// Keycode 0 is not a key; it means "empty slot" and must never be sent as a
/// fake release. Duplicates are dropped so a keycode listed under two modifier
/// groups is released once.
pub fn modifiers_down(keymap: &[u8], modifiers: &[u8]) -> Vec<u8> {
    let mut down: Vec<u8> = Vec::new();
    for &keycode in modifiers {
        if keycode == 0 || down.contains(&keycode) {
            continue;
        }
        if is_down(keymap, keycode) {
            down.push(keycode);
        }
    }
    down
}

/// Whether `keycode`'s bit is set in a `QueryKeymap` vector.
fn is_down(keymap: &[u8], keycode: u8) -> bool {
    let byte = keycode as usize / 8;
    keymap
        .get(byte)
        .is_some_and(|bits| bits & (1 << (keycode % 8)) != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `QueryKeymap` reply with exactly these keycodes held down.
    fn keymap(down: &[u8]) -> [u8; 32] {
        let mut bits = [0u8; 32];
        for &k in down {
            bits[k as usize / 8] |= 1 << (k % 8);
        }
        bits
    }

    /// Real keycodes on a typical X server: Shift_L 50, Control_L 37,
    /// Control_R 105, Alt_L 64, Super_L 133.
    const MODMAP: &[u8] = &[
        50, 62, 0, 0, // shift
        66, 0, 0, 0, // lock
        37, 105, 0, 0, // control
        64, 0, 0, 0, // mod1 / alt
        0, 0, 0, 0, // mod2
        0, 0, 0, 0, // mod3
        133, 134, 0, 0, // mod4 / super
        108, 0, 0, 0, // mod5
    ];

    #[test]
    fn nothing_held_lifts_nothing() {
        assert!(modifiers_down(&keymap(&[]), MODMAP).is_empty());
    }

    #[test]
    fn the_held_push_to_talk_modifier_is_found() {
        // Right-Ctrl, the default push-to-talk key.
        assert_eq!(modifiers_down(&keymap(&[105]), MODMAP), vec![105]);
    }

    #[test]
    fn ordinary_keys_are_never_lifted() {
        // `a` (38) and `space` (65) are down but are not modifiers.
        assert!(modifiers_down(&keymap(&[38, 65]), MODMAP).is_empty());
        assert_eq!(modifiers_down(&keymap(&[38, 50, 65]), MODMAP), vec![50]);
    }

    #[test]
    fn several_modifiers_are_all_lifted() {
        let held = keymap(&[50, 37, 133]);
        assert_eq!(modifiers_down(&held, MODMAP), vec![50, 37, 133]);
    }

    #[test]
    fn empty_modmap_slots_are_not_keys() {
        // Keycode 0's bit set in the keymap must not turn the padding zeros in
        // the modifier map into eight fake releases of a key that isn't one.
        let held = keymap(&[0]);
        assert!(modifiers_down(&held, MODMAP).is_empty());
    }

    #[test]
    fn a_keycode_in_two_groups_is_lifted_once() {
        let modmap: &[u8] = &[50, 0, 50, 0];
        assert_eq!(modifiers_down(&keymap(&[50]), modmap), vec![50]);
    }

    #[test]
    fn bit_order_matches_the_x_protocol() {
        // Keycode 8 is bit 0 of byte 1, keycode 15 is bit 7 of byte 1.
        let mut bits = [0u8; 32];
        bits[1] = 0b1000_0001;
        assert!(is_down(&bits, 8));
        assert!(is_down(&bits, 15));
        assert!(!is_down(&bits, 9));
        assert!(!is_down(&bits, 16));
    }

    #[test]
    fn a_short_keymap_does_not_panic() {
        // Never seen from a real server, but a reply is remote input.
        assert!(!is_down(&[], 50));
        assert!(modifiers_down(&[0u8; 4], MODMAP).is_empty());
    }
}
