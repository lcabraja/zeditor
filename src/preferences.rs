use gpui::{App, Global};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HotkeyConfig {
    pub key_code: u32,
    pub modifiers: u32,
    pub display_string: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            key_code: 0x0E,      // 'E'
            modifiers: (1 << 8) | (1 << 9), // Cmd + Shift
            display_string: "Cmd+Shift+E".to_string(),
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
    if let Ok(data) = std::fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Preferences::default()
    }
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
