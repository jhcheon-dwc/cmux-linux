// src/socket/keys.rs — key descriptor parsing for surface.send_key
//
// Turns descriptors like "enter", "ctrl+c", "alt+x", "ctrl+shift+t", "f5" or a
// single character into the fields of a ghostty_input_key_s. Keycodes are XKB
// keycodes (evdev + 8, US layout), the same values GTK hands to the key
// controller in src/ghostty/surface.rs, so Ghostty resolves the physical key
// exactly as it does for real keystrokes.

use crate::ghostty::ffi;

/// A parsed key descriptor. `text` is NUL-terminated (UTF-8, max 4 bytes + NUL)
/// so it can be passed to Ghostty without allocating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeySpec {
    /// XKB keycode (evdev + 8). 0 when the character has no US-layout key.
    pub keycode: u32,
    /// ghostty_input_mods_e bitmask.
    pub mods: ffi::ghostty_input_mods_e,
    /// UTF-8 text the key produces (NUL-terminated); empty for non-text keys.
    pub text: [u8; 8],
    /// Codepoint of the key without shift applied (0 for non-text keys).
    pub unshifted_codepoint: u32,
}

impl KeySpec {
    /// Pointer to the NUL-terminated text, or null when the key produces no text.
    pub fn text_ptr(&self) -> *const std::os::raw::c_char {
        if self.text[0] == 0 {
            std::ptr::null()
        } else {
            self.text.as_ptr() as *const std::os::raw::c_char
        }
    }

    /// The produced text as &str (empty for non-text keys).
    #[cfg(test)]
    pub fn text_str(&self) -> &str {
        let len = self.text.iter().position(|&b| b == 0).unwrap_or(self.text.len());
        std::str::from_utf8(&self.text[..len]).unwrap_or("")
    }
}

const MOD_SHIFT: ffi::ghostty_input_mods_e = ffi::ghostty_input_mods_e_GHOSTTY_MODS_SHIFT;
const MOD_CTRL: ffi::ghostty_input_mods_e = ffi::ghostty_input_mods_e_GHOSTTY_MODS_CTRL;
const MOD_ALT: ffi::ghostty_input_mods_e = ffi::ghostty_input_mods_e_GHOSTTY_MODS_ALT;
const MOD_SUPER: ffi::ghostty_input_mods_e = ffi::ghostty_input_mods_e_GHOSTTY_MODS_SUPER;

/// evdev + 8 offset used by XKB/GTK hardware keycodes.
const XKB_OFFSET: u32 = 8;

/// Named (non-character) keys → evdev code. These keys produce no text; Ghostty
/// encodes them from the physical key.
fn named_key(name: &str) -> Option<u32> {
    let evdev = match name {
        "enter" | "return" | "ret" | "cr" => 28,
        "escape" | "esc" => 1,
        "tab" => 15,
        "backspace" | "bs" => 14,
        "delete" | "del" => 111,
        "insert" | "ins" => 110,
        "home" => 102,
        "end" => 107,
        "pageup" | "pgup" | "page_up" => 104,
        "pagedown" | "pgdn" | "page_down" => 109,
        "up" | "arrowup" => 103,
        "down" | "arrowdown" => 108,
        "left" | "arrowleft" => 105,
        "right" | "arrowright" => 106,
        "f1" => 59, "f2" => 60, "f3" => 61, "f4" => 62, "f5" => 63, "f6" => 64,
        "f7" => 65, "f8" => 66, "f9" => 67, "f10" => 68, "f11" => 87, "f12" => 88,
        _ => return None,
    };
    Some(evdev + XKB_OFFSET)
}

/// US-layout character → (evdev code, needs shift, unshifted char).
fn char_key(c: char) -> Option<(u32, bool, char)> {
    let lower = |c: char| c.to_ascii_lowercase();
    let letter = |c: char| -> u32 {
        match lower(c) {
            'a' => 30, 'b' => 48, 'c' => 46, 'd' => 32, 'e' => 18, 'f' => 33, 'g' => 34,
            'h' => 35, 'i' => 23, 'j' => 36, 'k' => 37, 'l' => 38, 'm' => 50, 'n' => 49,
            'o' => 24, 'p' => 25, 'q' => 16, 'r' => 19, 's' => 31, 't' => 20, 'u' => 22,
            'v' => 47, 'w' => 17, 'x' => 45, 'y' => 21, _ => 44, // 'z'
        }
    };
    let (evdev, shift, base) = match c {
        'a'..='z' => (letter(c), false, c),
        'A'..='Z' => (letter(c), true, lower(c)),
        '1'..='9' => (c as u32 - '1' as u32 + 2, false, c),
        '0' => (11, false, '0'),
        ' ' => (57, false, ' '),
        '-' => (12, false, '-'), '_' => (12, true, '-'),
        '=' => (13, false, '='), '+' => (13, true, '='),
        '[' => (26, false, '['), '{' => (26, true, '['),
        ']' => (27, false, ']'), '}' => (27, true, ']'),
        '\\' => (43, false, '\\'), '|' => (43, true, '\\'),
        ';' => (39, false, ';'), ':' => (39, true, ';'),
        '\'' => (40, false, '\''), '"' => (40, true, '\''),
        '`' => (41, false, '`'), '~' => (41, true, '`'),
        ',' => (51, false, ','), '<' => (51, true, ','),
        '.' => (52, false, '.'), '>' => (52, true, '.'),
        '/' => (53, false, '/'), '?' => (53, true, '/'),
        '!' => (2, true, '1'), '@' => (3, true, '2'), '#' => (4, true, '3'),
        '$' => (5, true, '4'), '%' => (6, true, '5'), '^' => (7, true, '6'),
        '&' => (8, true, '7'), '*' => (9, true, '8'), '(' => (10, true, '9'),
        ')' => (11, true, '0'),
        _ => return None,
    };
    Some((evdev + XKB_OFFSET, shift, base))
}

/// US-layout character a key produces with Shift held, given its unshifted char.
fn shifted(base: char) -> Option<char> {
    Some(match base {
        'a'..='z' => base.to_ascii_uppercase(),
        '1' => '!', '2' => '@', '3' => '#', '4' => '$', '5' => '%',
        '6' => '^', '7' => '&', '8' => '*', '9' => '(', '0' => ')',
        '-' => '_', '=' => '+', '[' => '{', ']' => '}', '\\' => '|',
        ';' => ':', '\'' => '"', '`' => '~', ',' => '<', '.' => '>', '/' => '?',
        _ => return None,
    })
}

fn encode_text(c: char) -> [u8; 8] {
    let mut buf = [0u8; 8];
    c.encode_utf8(&mut buf[..4]);
    buf
}

/// Parse a key descriptor.
///
/// Grammar: `[mod+]*key`, case-insensitive for modifiers and named keys.
/// Modifiers: ctrl/control, shift, alt/meta/opt/option, super/cmd/win.
/// Key: a named key (enter, escape, tab, backspace, arrows, f1-f12, ...),
/// "space", "plus", or a single character (any Unicode scalar; characters
/// without a US-layout key are sent as text with keycode 0).
pub fn parse_key_descriptor(desc: &str) -> Result<KeySpec, String> {
    if desc.is_empty() {
        return Err("empty key descriptor".into());
    }
    // "ctrl++" and "+" mean the plus key: peel a trailing '+' off as the key.
    let (mod_part, key_part) = if desc == "+" {
        ("", "+")
    } else if let Some(prefix) = desc.strip_suffix("++") {
        (prefix, "+")
    } else {
        match desc.rfind('+') {
            Some(i) => (&desc[..i], &desc[i + 1..]),
            None => ("", desc),
        }
    };
    if key_part.is_empty() {
        return Err(format!("missing key in descriptor '{desc}'"));
    }

    let mut mods: ffi::ghostty_input_mods_e = 0;
    if !mod_part.is_empty() {
        for m in mod_part.split('+') {
            let bit = match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => MOD_CTRL,
                "shift" => MOD_SHIFT,
                "alt" | "meta" | "opt" | "option" => MOD_ALT,
                "super" | "cmd" | "command" | "win" | "logo" => MOD_SUPER,
                other => return Err(format!("unknown modifier '{other}' in '{desc}'")),
            };
            mods |= bit;
        }
    }

    let key_lower = key_part.to_ascii_lowercase();
    if let Some(keycode) = named_key(&key_lower) {
        return Ok(KeySpec { keycode, mods, text: [0; 8], unshifted_codepoint: 0 });
    }

    let ch = match key_lower.as_str() {
        "space" => ' ',
        "plus" => '+',
        "minus" => '-',
        _ => {
            let mut chars = key_part.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => c,
                _ => return Err(format!("unknown key '{key_part}' in '{desc}'")),
            }
        }
    };

    match char_key(ch) {
        Some((keycode, needs_shift, base)) => {
            if needs_shift {
                mods |= MOD_SHIFT;
            }
            // With shift held the key produces its shifted character (like GTK's
            // keyval): "shift+1" types "!", "shift+a" types "A".
            let produced = if mods & MOD_SHIFT != 0 {
                shifted(base).unwrap_or(ch)
            } else {
                ch
            };
            Ok(KeySpec {
                keycode,
                mods,
                text: encode_text(produced),
                unshifted_codepoint: base as u32,
            })
        }
        // Non-US-layout character (e.g. Korean): no physical key, text only.
        None => Ok(KeySpec {
            keycode: 0,
            mods,
            text: encode_text(ch),
            unshifted_codepoint: ch as u32,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys() {
        let k = parse_key_descriptor("enter").unwrap();
        assert_eq!(k.keycode, 36);
        assert_eq!(k.mods, 0);
        assert!(k.text_ptr().is_null());
        assert_eq!(parse_key_descriptor("Escape").unwrap().keycode, 9);
        assert_eq!(parse_key_descriptor("tab").unwrap().keycode, 23);
        assert_eq!(parse_key_descriptor("backspace").unwrap().keycode, 22);
        assert_eq!(parse_key_descriptor("up").unwrap().keycode, 111);
        assert_eq!(parse_key_descriptor("left").unwrap().keycode, 113);
        assert_eq!(parse_key_descriptor("f1").unwrap().keycode, 67);
        assert_eq!(parse_key_descriptor("F12").unwrap().keycode, 96);
    }

    #[test]
    fn modifier_combos() {
        let k = parse_key_descriptor("ctrl+c").unwrap();
        assert_eq!(k.keycode, 54);
        assert_eq!(k.mods, MOD_CTRL);
        assert_eq!(k.text_str(), "c");
        assert_eq!(k.unshifted_codepoint, 'c' as u32);

        let k = parse_key_descriptor("alt+x").unwrap();
        assert_eq!(k.mods, MOD_ALT);
        assert_eq!(k.keycode, 53);

        let k = parse_key_descriptor("Ctrl+Shift+T").unwrap();
        assert_eq!(k.mods, MOD_CTRL | MOD_SHIFT);
        assert_eq!(k.keycode, 28);
        assert_eq!(k.text_str(), "T");
        assert_eq!(k.unshifted_codepoint, 't' as u32);

        let k = parse_key_descriptor("shift+tab").unwrap();
        assert_eq!(k.mods, MOD_SHIFT);
        assert_eq!(k.keycode, 23);
    }

    #[test]
    fn characters() {
        let k = parse_key_descriptor("A").unwrap();
        assert_eq!(k.mods, MOD_SHIFT);
        assert_eq!(k.text_str(), "A");
        assert_eq!(k.unshifted_codepoint, 'a' as u32);

        let k = parse_key_descriptor("!").unwrap();
        assert_eq!(k.keycode, 10);
        assert_eq!(k.mods, MOD_SHIFT);
        assert_eq!(k.unshifted_codepoint, '1' as u32);

        let k = parse_key_descriptor("shift+1").unwrap();
        assert_eq!(k.text_str(), "!");
        assert_eq!(k.unshifted_codepoint, '1' as u32);
        assert_eq!(parse_key_descriptor("shift+/").unwrap().text_str(), "?");
        assert_eq!(parse_key_descriptor("shift+a").unwrap().text_str(), "A");

        let k = parse_key_descriptor("space").unwrap();
        assert_eq!(k.keycode, 65);
        assert_eq!(k.text_str(), " ");

        let k = parse_key_descriptor("+").unwrap();
        assert_eq!(k.text_str(), "+");
        let k = parse_key_descriptor("ctrl++").unwrap();
        assert_eq!(k.mods, MOD_CTRL | MOD_SHIFT);
        assert_eq!(k.unshifted_codepoint, '=' as u32);

        // Korean: no US-layout key, sent as text.
        let k = parse_key_descriptor("한").unwrap();
        assert_eq!(k.keycode, 0);
        assert_eq!(k.text_str(), "한");
    }

    #[test]
    fn errors() {
        assert!(parse_key_descriptor("").is_err());
        assert!(parse_key_descriptor("ctrl+").is_err());
        assert!(parse_key_descriptor("hyper+c").is_err());
        assert!(parse_key_descriptor("notakey").is_err());
    }
}
