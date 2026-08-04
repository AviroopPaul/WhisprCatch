use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// PTT key: rctrl, lctrl, ralt, lalt, super, f13, scrolllock
    pub key: String,
    /// Speech model: "parakeet" (accurate) or "moonshine" (light, low RAM)
    pub model: String,
    /// Model directory override; defaults to <data-dir>/whisper-catch/models/<model>
    pub model_dir: Option<PathBuf>,
    /// Keep a local log of transcriptions (history.jsonl)
    pub history: bool,
    /// Type words live while speaking instead of all at once on release
    pub streaming: bool,
    /// Show the floating recording indicator while dictating
    pub overlay: bool,
    /// XKB layout to type against on Wayland ("gb", "us+dvorak"). None detects
    /// it from the session; set this when detection guesses wrong.
    pub layout: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Right Alt is a low-conflict PTT key on Linux; on macOS Right Alt
            // is a dead key for accents, so default to Right Command there.
            key: if cfg!(target_os = "macos") { "rcmd" } else { "ralt" }.into(),
            // macOS floor device is an 8 GB M1 Air — default to the light model;
            // Linux defaults to the more accurate Parakeet.
            model: if cfg!(target_os = "macos") { "moonshine" } else { "parakeet" }.into(),
            model_dir: None,
            history: true,
            streaming: true,
            overlay: true,
            layout: None,
        }
    }
}

/// PTT keys offered in Settings, with display names. Platform-aware: macOS
/// leads with the Command keys (Right Alt is a dead key there).
pub const KEYS: &[(&str, &str)] = if cfg!(target_os = "macos") {
    &[
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

pub fn save(cfg: &Config) -> Result<()> {
    let path = config_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, toml::to_string_pretty(cfg)?)
        .with_context(|| format!("writing {}", path.display()))
}
