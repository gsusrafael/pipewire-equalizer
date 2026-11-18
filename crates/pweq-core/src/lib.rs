//! Shared model for `pipewire-equalizer`.
//!
//! The crate holds everything that does not talk to PipeWire: the preset
//! model, the legacy preset parser, the band definitions, the translation to a
//! PipeWire filter graph and the control protocol spoken between the daemon
//! and the GUI. Keeping it free of PipeWire bindings means it can be tested
//! and reused without a running audio server.

pub mod graph;
pub mod preset;
pub mod protocol;
pub mod store;

pub use graph::{
    band_label, band_node, band_q, FilterGraph, GraphNode, PreampNode, NODE_BAND_HIGH,
    NODE_BAND_LOW, NODE_BAND_PEAKING, NODE_PREAMP, Q_HIGH_SHELF, Q_LOW_SHELF, Q_PEAKING,
};
pub use preset::{
    db_to_linear, linear_to_db, parse_legacy_preset, read_legacy_preset, BAND_FREQUENCIES,
    BAND_FREQUENCY_LABELS, EqState, GAIN_MAX, GAIN_MIN, NUM_BANDS, Preset, PresetError,
};
pub use protocol::{
    decode_request, decode_response, encode_request, encode_response, Request, Response, State,
};
pub use store::{slugify, PresetFile, PresetStore};
