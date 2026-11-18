//! Control protocol between the GUI and the daemon.
//!
//! The daemon listens on a Unix socket and answers newline-delimited JSON
//! requests. Keeping the protocol in the core crate lets both sides share it
//! without either depending on the other.

use serde::{Deserialize, Serialize};

/// A request sent by a client to the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// Ask for the full state: the curve, the active preset and the status.
    GetState,
    /// List the available presets.
    ListPresets,
    /// Set the gain of one band, in dB.
    SetBand {
        /// Band index, 0 based.
        band: usize,
        /// New gain in dB.
        gain: f32,
    },
    /// Set the preamp, in dB.
    SetPreamp {
        /// New preamp in dB.
        db: f32,
    },
    /// Load a preset by name.
    LoadPreset {
        /// Preset name, as listed by `ListPresets`.
        name: String,
    },
    /// Reset every band and the preamp to zero.
    Reset,
    /// Enable or disable the equalizer.
    SetEnabled {
        /// Whether the equalizer should be active.
        enabled: bool,
    },
}

/// The state of the equalizer, as reported by the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// Whether the equalizer is currently applied.
    pub enabled: bool,
    /// The name of the active preset, empty for a custom curve.
    pub preset: String,
    /// The preamp in dB.
    pub preamp_db: f32,
    /// The gain of each band in dB.
    pub gains_db: Vec<f32>,
    /// The output the equalizer is currently applied to.
    pub output: String,
    /// A human readable description of the last error, empty when fine.
    pub error: String,
}

/// The answer to a request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    /// The request succeeded.
    Ok {
        /// The state after the request.
        state: State,
    },
    /// The request failed.
    Error {
        /// What went wrong.
        message: String,
    },
}

/// Encode a request as a single JSON line (without the trailing newline).
pub fn encode_request(request: &Request) -> String {
    serde_json::to_string(request).expect("request serialises")
}

/// Decode a request from a JSON line.
pub fn decode_request(line: &str) -> Result<Request, serde_json::Error> {
    serde_json::from_str(line)
}

/// Encode a response as a single JSON line (without the trailing newline).
pub fn encode_response(response: &Response) -> String {
    serde_json::to_string(response).expect("response serialises")
}

/// Decode a response from a JSON line.
pub fn decode_response(line: &str) -> Result<Response, serde_json::Error> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let request = Request::SetBand { band: 3, gain: -4.5 };
        let line = encode_request(&request);
        assert!(!line.contains('\n'));
        assert_eq!(decode_request(&line).unwrap(), request);
    }

    #[test]
    fn response_roundtrip() {
        let response = Response::Error {
            message: "no such preset".into(),
        };
        let line = encode_response(&response);
        assert_eq!(decode_response(&line).unwrap(), response);
    }
}
