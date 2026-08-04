//! Which XKB layout the injector types against, and what Settings can offer.
//!
//! The compositor interprets our injected keycodes through whatever layout it
//! has loaded, so the injector must reverse that same layout. Detection is a
//! guess — it reads the sources the compositor reads, but a user with a
//! physical keyboard that disagrees with their session settings needs the last
//! word. Hence an explicit override, with detection as the default.

/// A layout the picker can offer. `id` is the XKB spelling the injector needs
/// (`"gb"`, or `"us+dvorak"` for a variant); `description` is
/// xkeyboard-config's own name for it, so it matches what the user sees in
/// their desktop's keyboard settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub id: String,
    pub description: String,
}

/// xkeyboard-config's layout catalogue. `evdev.lst` is the modern rule set;
/// `base.lst` is the same content on older installs, and the `/usr/local`
/// path covers source builds.
const RULES_PATHS: [&str; 3] = [
    "/usr/share/X11/xkb/rules/evdev.lst",
    "/usr/share/X11/xkb/rules/base.lst",
    "/usr/local/share/X11/xkb/rules/evdev.lst",
];

/// Used when xkeyboard-config's list isn't readable. A short list of common
/// layouts beats an empty picker, and detection still covers anyone whose
/// layout isn't here.
const FALLBACK: [(&str, &str); 14] = [
    ("us", "English (US)"),
    ("us+dvorak", "English (Dvorak)"),
    ("us+colemak", "English (Colemak)"),
    ("gb", "English (UK)"),
    ("de", "German"),
    ("fr", "French"),
    ("es", "Spanish"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("br", "Portuguese (Brazil)"),
    ("ru", "Russian"),
    ("se", "Swedish"),
    ("pl", "Polish"),
    ("tr", "Turkish"),
];

/// Every layout the picker can offer, base layouts each followed by their own
/// variants. Reads the filesystem — call it once and keep the result.
pub fn available() -> Vec<Layout> {
    for path in RULES_PATHS {
        if let Ok(text) = std::fs::read_to_string(path) {
            let list = parse_rules(&text);
            if !list.is_empty() {
                return list;
            }
        }
    }
    log::debug!("no xkeyboard-config rules list found; offering the fallback layouts");
    FALLBACK
        .iter()
        .map(|(id, description)| Layout {
            id: (*id).into(),
            description: (*description).into(),
        })
        .collect()
}

/// Parses the `! layout` and `! variant` sections of an xkbcomp rules list:
///
/// ```text
/// ! layout
///   gb              English (UK)
/// ! variant
///   dvorak          us: English (Dvorak)
/// ```
fn parse_rules(text: &str) -> Vec<Layout> {
    let mut bases: Vec<Layout> = Vec::new();
    // (parent layout code, variant)
    let mut variants: Vec<(String, Layout)> = Vec::new();
    let mut section = "";

    for line in text.lines() {
        if let Some(header) = line.strip_prefix('!') {
            section = header.split_whitespace().next().unwrap_or("");
            continue;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((code, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let rest = rest.trim();
        match section {
            "layout" => bases.push(Layout {
                id: code.into(),
                description: rest.into(),
            }),
            "variant" => {
                // "us: English (Dvorak)" — the parent layout, then its name.
                let Some((parent, description)) = rest.split_once(':') else {
                    continue;
                };
                variants.push((
                    parent.trim().into(),
                    Layout {
                        id: format!("{}+{}", parent.trim(), code),
                        description: description.trim().into(),
                    },
                ));
            }
            _ => {}
        }
    }

    bases.sort_by(|a, b| a.description.cmp(&b.description));
    let mut out = Vec::with_capacity(bases.len() + variants.len());
    for base in bases {
        let mut mine: Vec<Layout> = variants
            .iter()
            .filter(|(parent, _)| *parent == base.id)
            .map(|(_, l)| l.clone())
            .collect();
        mine.sort_by(|a, b| a.description.cmp(&b.description));
        out.push(base);
        out.append(&mut mine);
    }
    out
}

/// The layout to type against: the user's explicit choice if they made one,
/// otherwise whatever the session looks like it's set to.
pub fn resolve(chosen: Option<&str>) -> (String, String) {
    if let Some(id) = chosen.map(str::trim).filter(|s| !s.is_empty()) {
        let (layout, variant) = id.split_once('+').unwrap_or((id, ""));
        return (layout.to_string(), variant.to_string());
    }
    detected()
}

/// Asks the same sources the compositor does: explicit env override, GNOME's
/// dconf, the distro console default.
pub fn detected() -> (String, String) {
    if let Ok(l) = std::env::var("XKB_DEFAULT_LAYOUT") {
        if !l.is_empty() {
            let v = std::env::var("XKB_DEFAULT_VARIANT").unwrap_or_default();
            return (l, v);
        }
    }
    if std::env::var("XDG_CURRENT_DESKTOP")
        .map(|d| d.to_ascii_lowercase().contains("gnome"))
        .unwrap_or(false)
    {
        if let Some(lv) = gnome_input_source() {
            return lv;
        }
    }
    if let Some(lv) = etc_default_keyboard() {
        return lv;
    }
    ("us".into(), String::new())
}

/// The detected layout as a picker id (`"gb"`, `"us+dvorak"`), for labelling
/// the auto-detect entry with what it currently resolves to.
pub fn detected_id() -> String {
    let (layout, variant) = detected();
    if variant.is_empty() {
        layout
    } else {
        format!("{layout}+{variant}")
    }
}

/// First entry of org.gnome.desktop.input-sources sources,
/// e.g. `[('xkb', 'gb'), ('xkb', 'us')]`; variants come as `layout+variant`.
fn gnome_input_source() -> Option<(String, String)> {
    let out = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.input-sources", "sources"])
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let entry = text.split("'xkb', '").nth(1)?;
    let source = entry.split('\'').next()?;
    let (layout, variant) = source.split_once('+').unwrap_or((source, ""));
    Some((layout.to_string(), variant.to_string()))
}

/// Debian/Ubuntu: XKBLAYOUT="gb" / XKBVARIANT="" in /etc/default/keyboard.
fn etc_default_keyboard() -> Option<(String, String)> {
    let content = std::fs::read_to_string("/etc/default/keyboard").ok()?;
    let get = |key: &str| {
        content
            .lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').to_string())
    };
    let layout = get("XKBLAYOUT").filter(|l| !l.is_empty())?;
    // multi-layout configs like "gb,us" — take the first
    let layout = layout.split(',').next().unwrap_or(&layout).to_string();
    let variant = get("XKBVARIANT")
        .map(|v| v.split(',').next().unwrap_or("").to_string())
        .unwrap_or_default();
    Some((layout, variant))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
! model
  pc105           Generic 105-key PC
! layout
  us              English (US)
  gb              English (UK)
! variant
  dvorak          us: English (Dvorak)
  colemak         us: English (Colemak)
  extd            gb: English (UK, extended)
! option
  grp:alt_shift_toggle  Alt+Shift
";

    #[test]
    fn parses_layouts_with_variants_grouped_under_their_base() {
        let list = parse_rules(SAMPLE);
        let ids: Vec<&str> = list.iter().map(|l| l.id.as_str()).collect();
        // Bases sorted by description ("English (UK)" < "English (US)"), each
        // trailed by its own variants. Models and options are ignored.
        assert_eq!(
            ids,
            vec!["gb", "gb+extd", "us", "us+colemak", "us+dvorak"]
        );
        assert_eq!(list[0].description, "English (UK)");
        assert_eq!(list[4].description, "English (Dvorak)");
    }

    #[test]
    fn an_explicit_choice_wins_over_detection() {
        assert_eq!(
            resolve(Some("us+dvorak")),
            ("us".to_string(), "dvorak".to_string())
        );
        assert_eq!(resolve(Some("gb")), ("gb".to_string(), String::new()));
    }

    #[test]
    fn blank_choices_fall_through_to_detection() {
        std::env::set_var("XKB_DEFAULT_LAYOUT", "gb");
        assert_eq!(resolve(None), ("gb".to_string(), String::new()));
        assert_eq!(resolve(Some("  ")), ("gb".to_string(), String::new()));
        std::env::remove_var("XKB_DEFAULT_LAYOUT");
    }
}
