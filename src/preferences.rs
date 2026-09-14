use anyhow::{Context, Result};
use gpui::{App, Global};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

const OLD_DEFAULT_KEY_CODE: u32 = 0x0E; // 'E'
const OLD_DEFAULT_MODIFIERS: u32 = (1 << 8) | (1 << 9); // Cmd + Shift

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
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
    match load_preferences_from(&path) {
        Ok(prefs) => prefs,
        Err(error) => {
            crate::logging::event("preferences.load", format!("{error:#}"));
            Preferences::default()
        }
    }
}

fn load_preferences_from(path: &Path) -> Result<Preferences> {
    let data = match std::fs::read_to_string(path) {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Preferences::default());
        }
        Err(error) => return Err(error).with_context(|| format!("Read {}", path.display())),
    };
    let mut prefs: Preferences =
        serde_json::from_str(&data).with_context(|| format!("Parse {}", path.display()))?;

    let migrated_old_default = prefs.hotkey.key_code == OLD_DEFAULT_KEY_CODE
        && prefs.hotkey.modifiers == OLD_DEFAULT_MODIFIERS;
    if migrated_old_default {
        prefs.hotkey = HotkeyConfig::default();
        if let Err(error) = save_preferences_to(&prefs, path) {
            crate::logging::event("preferences.migrate", format!("{error:#}"));
        }
    }

    Ok(prefs)
}

pub fn save_preferences(prefs: &Preferences) -> Result<()> {
    save_preferences_to(prefs, &config_path())
}

fn save_preferences_to(prefs: &Preferences, path: &Path) -> Result<()> {
    let parent = path.parent().context("Configuration path has no parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("Create {}", parent.display()))?;
    // Keep the temporary file on the same filesystem so rename is atomic.
    // A failed write must never truncate the working configuration.
    let mut file =
        tempfile::NamedTempFile::new_in(parent).context("Create temporary preferences file")?;
    serde_json::to_writer_pretty(&mut file, prefs).context("Serialize preferences")?;
    file.write_all(b"\n").context("Write preferences")?;
    file.as_file().sync_all().context("Flush preferences")?;
    file.persist(path)
        .with_context(|| format!("Replace {}", path.display()))?;
    Ok(())
}

impl Preferences {
    pub fn init(app: &mut App) {
        let prefs = load_preferences();
        app.set_global(prefs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_uses_defaults_without_creating_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert_eq!(
            load_preferences_from(&path).unwrap(),
            Preferences::default()
        );
        assert!(!path.exists());
    }

    #[test]
    fn malformed_config_is_reported_and_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{broken").unwrap();
        assert!(load_preferences_from(&path).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "{broken");
    }

    #[test]
    fn missing_fields_use_defaults() {
        assert_eq!(
            serde_json::from_str::<Preferences>("{}").unwrap(),
            Preferences::default()
        );
    }

    #[test]
    fn save_creates_parents_and_replaces_existing_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/config.json");
        save_preferences_to(&Preferences::default(), &path).unwrap();
        let mut prefs = Preferences::default();
        prefs.hotkey.key_code = 0x00;
        prefs.hotkey.display_string = "Option+Cmd+A".into();
        save_preferences_to(&prefs, &path).unwrap();
        assert_eq!(load_preferences_from(&path).unwrap(), prefs);
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn failed_replace_preserves_destination_and_cleans_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("keep"), "existing data").unwrap();
        assert!(save_preferences_to(&Preferences::default(), &path).is_err());
        assert_eq!(
            std::fs::read_to_string(path.join("keep")).unwrap(),
            "existing data"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn legacy_default_migrates_but_custom_shortcuts_survive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut prefs = Preferences::default();
        prefs.hotkey.key_code = OLD_DEFAULT_KEY_CODE;
        prefs.hotkey.modifiers = OLD_DEFAULT_MODIFIERS;
        save_preferences_to(&prefs, &path).unwrap();
        assert_eq!(
            load_preferences_from(&path).unwrap(),
            Preferences::default()
        );
        assert_eq!(
            load_preferences_from(&path).unwrap(),
            Preferences::default()
        );
        prefs.hotkey.modifiers |= 1 << 11;
        save_preferences_to(&prefs, &path).unwrap();
        assert_eq!(load_preferences_from(&path).unwrap(), prefs);
    }
}
