//! Preset model and the legacy `pulseaudio-equalizer-ladspa` preset format.
//!
//! The upstream project shipped plain-text presets with one value per line:
//!
//! ```text
//! mbeq_1197        <- LADSPA plugin id
//! mbeq             <- LADSPA label
//! Multiband EQ     <- plugin name
//! 2.0              <- preamp (linear multiplier, not dB)
//! Flat             <- preset name
//! 15               <- number of bands
//! 0                <- gain of band 1 (dB)
//! ...              <- one gain per band
//! 50               <- centre frequency of band 1 (Hz)
//! ...              <- one frequency per band
//! ```
//!
//! We keep that format readable so users can move their existing presets over
//! unchanged, but the in-memory model uses explicit dB values.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

/// The 15 band centre frequencies used by the upstream equalizer.
///
/// The first band is a low shelf and the last one a high shelf; the 13 in
/// between are peaking filters.
pub const BAND_FREQUENCIES: [f64; 15] = [
    50.0, 100.0, 156.0, 220.0, 311.0, 440.0, 622.0, 880.0, 1250.0, 1750.0, 2500.0, 3500.0, 5000.0,
    10000.0, 20000.0,
];

/// Number of bands in a preset.
pub const NUM_BANDS: usize = BAND_FREQUENCIES.len();

/// Minimum gain a band can be set to, in dB.
pub const GAIN_MIN: f32 = -30.0;

/// Maximum gain a band can be set to, in dB.
pub const GAIN_MAX: f32 = 30.0;

/// A full equalizer state: a preamp and one gain per band.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EqState {
    /// Preamp in dB.
    pub preamp_db: f32,
    /// Gain per band in dB, in band order.
    pub gains_db: [f32; NUM_BANDS],
}

impl Default for EqState {
    fn default() -> Self {
        Self {
            preamp_db: 0.0,
            gains_db: [0.0; NUM_BANDS],
        }
    }
}

impl EqState {
    /// A flat curve.
    pub fn flat() -> Self {
        Self::default()
    }

    /// Build from a preamp and a slice of band gains.
    ///
    /// Returns `None` when the slice does not hold exactly [`NUM_BANDS`] values.
    pub fn new(preamp_db: f32, gains_db: &[f32]) -> Option<Self> {
        if gains_db.len() != NUM_BANDS {
            return None;
        }
        let mut gains = [0.0f32; NUM_BANDS];
        gains.copy_from_slice(gains_db);
        Some(Self {
            preamp_db,
            gains_db: gains,
        })
    }

    /// Clamp every value into the supported range.
    pub fn clamped(&self) -> Self {
        let mut out = self.clone();
        out.preamp_db = out.preamp_db.clamp(GAIN_MIN, GAIN_MAX);
        for gain in out.gains_db.iter_mut() {
            *gain = gain.clamp(GAIN_MIN, GAIN_MAX);
        }
        out
    }
}

/// A named preset: a curve plus the metadata carried over from upstream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    /// Preset name, e.g. "Rock".
    pub name: String,
    /// The curve.
    pub state: EqState,
}

/// Errors that can come out of parsing a legacy preset file.
#[derive(Debug)]
pub enum PresetError {
    /// The file could not be read.
    Io(io::Error),
    /// The file did not have the expected shape.
    Format(String),
}

impl fmt::Display for PresetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PresetError::Io(e) => write!(f, "cannot read preset: {e}"),
            PresetError::Format(m) => write!(f, "malformed preset: {m}"),
        }
    }
}

impl std::error::Error for PresetError {}

impl From<io::Error> for PresetError {
    fn from(e: io::Error) -> Self {
        PresetError::Io(e)
    }
}

/// Parse the upstream `pulseaudio-equalizer-ladspa` preset format.
///
/// The upstream format stores the preamp as a *linear multiplier* (2.0 means
/// +6 dB); we convert it to dB so the rest of the code only deals with one
/// unit.
pub fn parse_legacy_preset(text: &str) -> Result<Preset, PresetError> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 6 {
        return Err(PresetError::Format(format!(
            "expected at least 6 lines, found {}",
            lines.len()
        )));
    }

    let name = lines[4].trim().to_string();
    let preamp_linear: f64 = lines[3]
        .trim()
        .parse()
        .map_err(|_| PresetError::Format(format!("bad preamp value {:?}", lines[3])))?;
    let num_bands: usize = lines[5]
        .trim()
        .parse()
        .map_err(|_| PresetError::Format(format!("bad band count {:?}", lines[5])))?;

    if num_bands != NUM_BANDS {
        return Err(PresetError::Format(format!(
            "expected {NUM_BANDS} bands, found {num_bands}"
        )));
    }

    let need = 6 + num_bands * 2;
    if lines.len() < need {
        return Err(PresetError::Format(format!(
            "expected {need} lines, found {}",
            lines.len()
        )));
    }

    let mut gains = [0.0f32; NUM_BANDS];
    for (i, gain) in gains.iter_mut().enumerate() {
        *gain = lines[6 + i]
            .trim()
            .parse::<f32>()
            .map_err(|_| PresetError::Format(format!("bad gain value {:?}", lines[6 + i])))?;
    }

    // The upstream format lists the band frequencies; they must match the ones
    // we implement, otherwise the curve would mean something different.
    for i in 0..num_bands {
        let freq: f64 = lines[6 + num_bands + i]
            .trim()
            .parse()
            .map_err(|_| {
                PresetError::Format(format!("bad frequency value {:?}", lines[6 + num_bands + i]))
            })?;
        if (freq - BAND_FREQUENCIES[i]).abs() > 0.5 {
            return Err(PresetError::Format(format!(
                "band {i} is at {freq} Hz, expected {} Hz",
                BAND_FREQUENCIES[i]
            )));
        }
    }

    Ok(Preset {
        name,
        state: EqState {
            preamp_db: linear_to_db(preamp_linear),
            gains_db: gains,
        },
    })
}

/// Read and parse a legacy preset file.
pub fn read_legacy_preset(path: &Path) -> Result<Preset, PresetError> {
    let text = fs::read_to_string(path)?;
    parse_legacy_preset(&text)
}

/// Convert a linear amplitude multiplier to dB.
pub fn linear_to_db(linear: f64) -> f32 {
    if linear <= 0.0 {
        return GAIN_MIN;
    }
    (20.0 * linear.log10()) as f32
}

/// Convert dB to a linear amplitude multiplier.
pub fn db_to_linear(db: f32) -> f64 {
    10f64.powf(f64::from(db) / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLAT: &str = "mbeq_1197\nmbeq\nMultiband EQ\n2.0\nFlat\n15\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n0\n50\n100\n156\n220\n311\n440\n622\n880\n1250\n1750\n2500\n3500\n5000\n10000\n20000\n";

    #[test]
    fn parses_flat_preset() {
        let preset = parse_legacy_preset(FLAT).unwrap();
        assert_eq!(preset.name, "Flat");
        assert_eq!(preset.state.gains_db, [0.0; NUM_BANDS]);
        // 2.0 linear is +6.02 dB.
        assert!((preset.state.preamp_db - 6.0206).abs() < 0.001);
    }

    #[test]
    fn rejects_wrong_band_count() {
        let bad = FLAT.replace("\n15\n", "\n14\n");
        assert!(matches!(
            parse_legacy_preset(&bad),
            Err(PresetError::Format(_))
        ));
    }

    #[test]
    fn rejects_mismatched_frequencies() {
        let bad = FLAT.replace("\n50\n", "\n55\n");
        assert!(matches!(
            parse_legacy_preset(&bad),
            Err(PresetError::Format(_))
        ));
    }

    #[test]
    fn linear_db_roundtrip() {
        for db in [-30.0f32, -6.0, 0.0, 6.0, 30.0] {
            let back = linear_to_db(db_to_linear(db));
            assert!((back - db).abs() < 0.001, "{db} -> {back}");
        }
    }
}
