//! Loading presets from disk.
//!
//! Two formats are supported:
//!
//! * JSON, the native format of this project (`{"name": ..., "preamp_db":
//!   ..., "gains_db": [...]}`).
//! * The legacy plain-text format of `pulseaudio-equalizer-ladspa`, so users
//!   can move their existing presets over unchanged.

use crate::preset::{read_legacy_preset, EqState, Preset, PresetError};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// A preset as stored on disk in the native JSON format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresetFile {
    /// Preset name.
    pub name: String,
    /// Preamp in dB.
    pub preamp_db: f32,
    /// Gain per band in dB.
    pub gains_db: Vec<f32>,
}

impl PresetFile {
    /// Convert to the in-memory model.
    pub fn to_preset(&self) -> Option<Preset> {
        let state = EqState::new(self.preamp_db, &self.gains_db)?;
        Some(Preset {
            name: self.name.clone(),
            state,
        })
    }
}

/// A store of presets, backed by a directory.
#[derive(Debug, Clone)]
pub struct PresetStore {
    dir: PathBuf,
}

impl PresetStore {
    /// Create a store over a directory.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The directory backing the store.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Load every preset in the directory, sorted by name.
    ///
    /// Files that cannot be parsed are skipped, so one broken preset does not
    /// take the whole set down.
    pub fn load_all(&self) -> Vec<Preset> {
        let mut presets = Vec::new();
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return presets;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(preset) = self.load_file(&path) {
                presets.push(preset);
            }
        }

        presets.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        presets
    }

    /// Load a single preset file, in either supported format.
    pub fn load_file(&self, path: &Path) -> Option<Preset> {
        let extension = path.extension()?.to_str()?;
        match extension {
            "json" => {
                let text = fs::read_to_string(path).ok()?;
                let file: PresetFile = serde_json::from_str(&text).ok()?;
                file.to_preset()
            }
            "preset" => read_legacy_preset(path).ok(),
            _ => None,
        }
    }

    /// Find a preset by name, case insensitively.
    pub fn find(&self, name: &str) -> Option<Preset> {
        self.load_all()
            .into_iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// Save a preset, overwriting an existing one with the same name.
    ///
    /// Returns the path it was written to.
    pub fn save(&self, preset: &Preset) -> Result<PathBuf, PresetError> {
        fs::create_dir_all(&self.dir)?;

        let file = PresetFile {
            name: preset.name.clone(),
            preamp_db: preset.state.preamp_db,
            gains_db: preset.state.gains_db.to_vec(),
        };

        let path = self.dir.join(format!("{}.json", slugify(&preset.name)));
        let text = serde_json::to_string_pretty(&file)
            .map_err(|e| PresetError::Format(e.to_string()))?;
        fs::write(&path, text + "\n")?;
        Ok(path)
    }
}

/// Turn a preset name into a file name.
pub fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_dash = true;

    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }

    slug.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_handles_spaces_and_symbols() {
        assert_eq!(slugify("Bose QuietComfort 35"), "bose-quietcomfort-35");
        assert_eq!(slugify("Full Bass & Treble"), "full-bass-treble");
        assert_eq!(slugify("  Rock  "), "rock");
    }

    #[test]
    fn saves_and_loads_back() {
        let dir = std::env::temp_dir().join(format!("pweq-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = PresetStore::new(&dir);

        let preset = Preset {
            name: "My Preset".into(),
            state: EqState::new(3.0, &[1.0; crate::preset::NUM_BANDS]).unwrap(),
        };
        let path = store.save(&preset).unwrap();
        assert!(path.exists());

        let loaded = store.find("my preset").unwrap();
        assert_eq!(loaded.state.preamp_db, 3.0);
        assert_eq!(loaded.state.gains_db, preset.state.gains_db);

        let _ = fs::remove_dir_all(&dir);
    }
}
