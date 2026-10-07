use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// PTT key: fn, rcmd, lcmd, rctrl, lctrl, ralt, lalt, super, f13, scrolllock
    pub key: String,
    /// Speech model: "parakeet" (accurate) or "moonshine" (light, low RAM)
    pub model: String,
    /// Model directory override; defaults to <data-dir>/whisper-catch/models/<model>
    pub model_dir: Option<PathBuf>,
    /// Keep a local log of transcriptions (history.jsonl)
    pub history: bool,
    /// Type words live while speaking instead of all at once on release
    pub streaming: bool,
    /// Show the Catcher (the capsule above the Dock) while dictating
    pub overlay: bool,
    /// Show a Notes button when the Catcher is hovered. Off until switched on.
    /// The Catcher process re-reads the config, so this applies live.
    #[serde(default)]
    pub catcher_notes: bool,
    /// Deterministic text cleanup (#36). Every transform ships off, so a
    /// config that predates this section behaves exactly as it did.
    ///
    /// Last by convention, not by requirement: `toml` 0.9 hoists scalars above
    /// tables when it serializes, so a plain field declared after this one
    /// still round-trips. (That was not true of `toml` 0.8, which errored.)
    /// Keeping it last just makes the written file read in declaration order.
    pub polish: wc_text::PolishConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // macOS: fn, the key Mac dictation tools have trained people to
            // hold (it needs "Press fn to: Do Nothing", which setup offers).
            // Linux: Right Alt, because most laptops never report fn there.
            key: if cfg!(target_os = "macos") { "fn" } else { "ralt" }.into(),
            // macOS floor device is an 8 GB M1 Air — default to the light model;
            // Linux defaults to the more accurate Parakeet.
            model: if cfg!(target_os = "macos") { "moonshine" } else { "parakeet" }.into(),
            model_dir: None,
            history: true,
            streaming: true,
            overlay: true,
            catcher_notes: false,
            polish: wc_text::PolishConfig::default(),
        }
    }
}

/// PTT keys offered in Settings, with display names. Platform-aware: macOS
/// leads with fn, then the Command keys (Right Alt is a dead key there).
pub const KEYS: &[(&str, &str)] = if cfg!(target_os = "macos") {
    &[
        ("fn", "fn"),
        ("rcmd", "Right ⌘"),
        ("lcmd", "Left ⌘"),
        ("ralt", "Right Option"),
        ("lalt", "Left Option"),
        ("rctrl", "Right Ctrl"),
        ("lctrl", "Left Ctrl"),
        ("f13", "F13"),
    ]
} else {
    &[
        ("ralt", "Right Alt"),
        ("lalt", "Left Alt"),
        ("rctrl", "Right Ctrl"),
        ("lctrl", "Left Ctrl"),
        ("super", "Super / Win"),
        ("f13", "F13"),
        ("scrolllock", "Scroll Lock"),
    ]
};

/// Human label for a PTT key slug ("rcmd" → "Right ⌘").
pub fn key_label(key: &str) -> &str {
    KEYS.iter()
        .chain(
            [
                ("fn", "fn"),
                ("rcmd", "Right ⌘"),
                ("lcmd", "Left ⌘"),
                ("super", "Super / Win"),
                ("scrolllock", "Scroll Lock"),
                ("ralt", "Right Alt"),
                ("lalt", "Left Alt"),
                ("rctrl", "Right Ctrl"),
                ("lctrl", "Left Ctrl"),
            ]
            .iter(),
        )
        .find(|(k, _)| *k == key)
        .map(|(_, l)| *l)
        .unwrap_or(key)
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .expect("no config dir on this platform")
        .join("whisper-catch")
        .join("config.toml")
}

/// Loads config, writing the default file on first run so users can find it.
pub fn load() -> Result<Config> {
    let path = config_path();
    if !path.exists() {
        let cfg = Config::default();
        save(&cfg)?;
        log::info!("wrote default config to {}", path.display());
        return Ok(cfg);
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
}

/// When `config.toml` was last written, for callers that poll it cheaply.
pub fn modified() -> Option<std::time::SystemTime> {
    std::fs::metadata(config_path()).ok()?.modified().ok()
}

pub fn save(cfg: &Config) -> Result<()> {
    let path = config_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, toml::to_string_pretty(cfg)?)
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim `config.toml` as v0.4.0 wrote it — no `[polish]` section.
    const V0_4_CONFIG: &str = "\
key = \"ralt\"
model = \"parakeet\"
history = true
streaming = true
overlay = true
";

    /// The upgrade path for every existing user. Failing to parse here means
    /// the daemon refuses to start after an update.
    #[test]
    fn a_config_written_before_polish_still_loads() {
        let cfg: Config = toml::from_str(V0_4_CONFIG).unwrap();
        assert_eq!(cfg.key, "ralt");
        assert_eq!(cfg.model, "parakeet");
        assert!(cfg.history);
        assert!(
            wc_text::Polish::from_config(&cfg.polish).is_empty(),
            "an absent [polish] section must mean every transform off"
        );
    }

    /// An even older file, from before `streaming`/`overlay` existed. The
    /// struct-level `#[serde(default)]` is what makes this work; this test is
    /// here so nobody removes it.
    #[test]
    fn a_minimal_config_fills_every_missing_field() {
        let cfg: Config = toml::from_str("key = \"rctrl\"\n").unwrap();
        assert_eq!(cfg.key, "rctrl");
        assert_eq!(cfg.model, Config::default().model);
        assert!(wc_text::Polish::from_config(&cfg.polish).is_empty());
    }

    /// fn is the macOS default and must be offered first in Settings; a
    /// default the picker cannot show reads as a blank dropdown.
    #[test]
    fn the_default_key_is_offered_first() {
        let def = Config::default();
        assert_eq!(KEYS[0].0, def.key);
        if cfg!(target_os = "macos") {
            assert_eq!(def.key, "fn");
            assert_eq!(key_label("fn"), "fn");
        }
    }

    #[test]
    fn empty_config_equals_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        let def = Config::default();
        assert_eq!(cfg.key, def.key);
        assert_eq!(cfg.model, def.model);
        assert_eq!(cfg.history, def.history);
        assert_eq!(cfg.streaming, def.streaming);
        assert_eq!(cfg.overlay, def.overlay);
        assert!(!cfg.catcher_notes, "the Notes button ships off");
    }

    /// Settings → Save serializes the whole `Config`, so a change that makes
    /// `to_string_pretty` fail is a runtime panic with nothing at compile time
    /// to catch it. Adding a table field is the usual way to cause that, which
    /// is why this exists — `toml` 0.9 handles it, `toml` 0.8 did not.
    #[test]
    fn default_config_round_trips_through_toml() {
        let text = toml::to_string_pretty(&Config::default())
            .expect("Config must stay serializable — Settings → Save depends on it");
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.key, Config::default().key);
        assert!(wc_text::Polish::from_config(&back.polish).is_empty());
    }

    #[test]
    fn an_enabled_transform_survives_a_save_load_cycle() {
        let mut cfg = Config::default();
        cfg.polish.fillers.enabled = true;
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(
            wc_text::Polish::from_config(&back.polish).names(),
            ["fillers"]
        );
    }

    /// A newer build writes settings this one has never heard of. Ignoring
    /// them beats refusing to start — but "ignored" is the whole story, not
    /// half of it: `save` writes the typed struct back, so the next time the
    /// user touches Settings those keys are gone from `config.toml`. Running
    /// an older build and saving therefore discards a newer build's polish
    /// settings. That is tolerable only because #43 and #47 store the data
    /// users actually author in their own files; if anything ever needs to
    /// survive a downgrade, it cannot live in this struct.
    #[test]
    fn unknown_keys_load_but_are_dropped_on_the_next_save() {
        let from_newer_build =
            format!("{V0_4_CONFIG}tone = \"casual\"\n\n[polish.tone]\nenabled = true\n");
        let cfg: Config = toml::from_str(&from_newer_build).unwrap();
        assert_eq!(cfg.key, "ralt");
        assert!(wc_text::Polish::from_config(&cfg.polish).is_empty());

        let written = toml::to_string_pretty(&cfg).unwrap();
        assert!(!written.contains("tone"), "{written}");
    }
}
