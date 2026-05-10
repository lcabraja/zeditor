use gpui::{App, Global};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const OLD_DEFAULT_KEY_CODE: u32 = 0x0E; // 'E'
const OLD_DEFAULT_MODIFIERS: u32 = (1 << 8) | (1 << 9); // Cmd + Shift

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HotkeyConfig {
    pub key_code: u32,
    pub modifiers: u32,
    pub display_string: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            key_code: 0x09,                  // 'V'
            modifiers: (1 << 8) | (1 << 11), // Cmd + Option
            display_string: "Option+Cmd+V".to_string(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Preferences {
    pub hotkey: HotkeyConfig,
}

impl Global for Preferences {}

fn config_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Zeditor")
        .join("config.json")
}

pub fn load_preferences() -> Preferences {
    let path = config_path();
    let mut prefs = if let Ok(data) = std::fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Preferences::default()
    };

    let migrated_old_default = prefs.hotkey.key_code == OLD_DEFAULT_KEY_CODE
        && prefs.hotkey.modifiers == OLD_DEFAULT_MODIFIERS;
    if migrated_old_default {
        prefs.hotkey = HotkeyConfig::default();
        save_preferences(&prefs);
    }

    prefs
}

pub fn save_preferences(prefs: &Preferences) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(prefs) {
        let _ = std::fs::write(&path, json);
    }
}

impl Preferences {
    pub fn init(app: &mut App) {
        let prefs = load_preferences();
        app.set_global(prefs);
    }
}
