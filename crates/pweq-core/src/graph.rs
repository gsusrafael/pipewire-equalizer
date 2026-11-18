//! Translation from an [`EqState`] to a PipeWire `filter-chain` graph.
//!
//! The graph is a chain of native PipeWire biquads:
//!
//! ```text
//! preamp (linear) -> lowshelf -> peaking x13 -> highshelf
//! ```
//!
//! Every band of the legacy equalizer maps to one biquad, so the control port
//! of band *n* is reachable at runtime as `<node name>:Gain`.

use crate::preset::{EqState, BAND_FREQUENCIES};

/// Index of the last band (20 kHz), the high shelf.
const LAST_BAND: usize = crate::preset::NUM_BANDS - 1;

/// The name given to each biquad node inside the graph.
///
/// Kept stable because the control ports are addressed by name at runtime.
pub const NODE_PREAMP: &str = "preamp";
/// First band (50 Hz), a low shelf.
pub const NODE_BAND_LOW: &str = "band_low";
/// Last band (20 kHz), a high shelf.
pub const NODE_BAND_HIGH: &str = "band_high";
/// Middle bands, peaking filters.
pub const NODE_BAND_PEAKING: &str = "band_peaking";

/// Q factor of the low shelf (50 Hz).
pub const Q_LOW_SHELF: f64 = 0.55;
/// Q factor of the high shelf (19912.16 Hz).
pub const Q_HIGH_SHELF: f64 = 0.25;
/// Q factor of the peaking filters, per band.
///
/// The upstream `mbeq_1197` plugin is not a biquad equalizer: it works in the
/// frequency domain (1024 point FFT, raised cosine window, 75% overlap) and
/// interpolates the gain linearly between band centres. A cascade of biquads
/// cannot reproduce that curve exactly, so these Q values are the ones that
/// fit the original curve most closely, band by band.
pub const Q_PEAKING: [f64; 15] = [
    1.93, 2.67, 2.92, 2.92, 2.92, 2.92, 2.92, 2.92, 2.87, 2.87, 2.82, 1.38, 0.74, 0.55, 0.25,
];

/// The label of the biquad kind for a given band index.
///
/// Band 0 is a low shelf, band 14 a high shelf, the rest peaking.
pub fn band_label(index: usize) -> &'static str {
    match index {
        0 => "bq_lowshelf",
        LAST_BAND => "bq_highshelf",
        _ => "bq_peaking",
    }
}

/// The Q factor of a given band.
pub fn band_q(index: usize) -> f64 {
    match index {
        0 => Q_LOW_SHELF,
        LAST_BAND => Q_HIGH_SHELF,
        _ => Q_PEAKING[index],
    }
}

/// The node name of a given band index.
pub fn band_node(index: usize) -> String {
    match index {
        0 => NODE_BAND_LOW.to_string(),
        LAST_BAND => NODE_BAND_HIGH.to_string(),
        _ => format!("{NODE_BAND_PEAKING}{index}"),
    }
}

/// A single node of the filter graph, ready to be rendered.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphNode {
    /// Node name inside the graph.
    pub name: String,
    /// PipeWire biquad label: `bq_lowshelf`, `bq_peaking`, `bq_highshelf`.
    pub label: &'static str,
    /// Centre frequency in Hz.
    pub freq: f64,
    /// Q factor.
    pub q: f64,
    /// Gain in dB.
    pub gain: f64,
}

/// A preamp node: a linear gain applied before the bands.
#[derive(Debug, Clone, PartialEq)]
pub struct PreampNode {
    /// Node name inside the graph.
    pub name: &'static str,
    /// Linear multiplier.
    pub mult: f64,
}

/// The complete filter graph for a given EQ state.
#[derive(Debug, Clone, PartialEq)]
pub struct FilterGraph {
    /// The preamp node.
    pub preamp: PreampNode,
    /// One node per band, in band order.
    pub bands: Vec<GraphNode>,
}

impl FilterGraph {
    /// Build the graph for an EQ state.
    pub fn build(state: &EqState) -> Self {
        let mult = crate::preset::db_to_linear(state.preamp_db);
        let bands = BAND_FREQUENCIES
            .iter()
            .enumerate()
            .map(|(i, &freq)| GraphNode {
                name: band_node(i),
                label: band_label(i),
                freq,
                q: band_q(i),
                gain: f64::from(state.gains_db[i]),
            })
            .collect();

        Self {
            preamp: PreampNode {
                name: NODE_PREAMP,
                mult,
            },
            bands,
        }
    }

    /// The node names in the order they appear in the chain.
    pub fn chain(&self) -> Vec<String> {
        let mut names = vec![self.preamp.name.to_string()];
        names.extend(self.bands.iter().map(|b| b.name.clone()));
        names
    }

    /// Render the graph as the `filter.graph` value of a PipeWire module
    /// argument, in the SPA-JSON syntax the `filter-chain` module expects.
    pub fn to_spa_json(&self) -> String {
        let mut nodes = String::new();

        nodes.push_str(&format!(
            "{{ type = builtin name = {} label = linear control = {{ \"Mult\" = {} \"Add\" = 0.0 }} }}",
            self.preamp.name,
            fmt_f64(self.preamp.mult)
        ));

        for band in &self.bands {
            nodes.push_str(&format!(
                ", {{ type = builtin name = {} label = {} control = {{ \"Freq\" = {} \"Q\" = {} \"Gain\" = {} }} }}",
                band.name,
                band.label,
                fmt_f64(band.freq),
                fmt_f64(band.q),
                fmt_f64(band.gain)
            ));
        }

        let chain = self.chain();
        let links: Vec<String> = chain
            .windows(2)
            .map(|pair| {
                format!(
                    "{{ output = \"{}:Out\" input = \"{}:In\" }}",
                    pair[0], pair[1]
                )
            })
            .collect();

        format!(
            "filter.graph = {{ nodes = [ {} ] links = [ {} ] }}",
            nodes,
            links.join(", ")
        )
    }
}

/// Format a float the way SPA-JSON wants it: always with a decimal point.
fn fmt_f64(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::NUM_BANDS;

    #[test]
    fn flat_state_builds_full_chain() {
        let graph = FilterGraph::build(&EqState::flat());
        assert_eq!(graph.bands.len(), NUM_BANDS);
        assert_eq!(graph.chain().len(), NUM_BANDS + 1);
        assert_eq!(graph.bands[0].label, "bq_lowshelf");
        assert_eq!(graph.bands[1].label, "bq_peaking");
        assert_eq!(graph.bands[NUM_BANDS - 1].label, "bq_highshelf");
    }

    #[test]
    fn preamp_is_linear_multiplier() {
        let mut state = EqState::flat();
        state.preamp_db = 6.0206;
        let graph = FilterGraph::build(&state);
        assert!((graph.preamp.mult - 2.0).abs() < 0.001);
    }

    #[test]
    fn spa_json_contains_every_control() {
        let graph = FilterGraph::build(&EqState::flat());
        let json = graph.to_spa_json();
        assert!(json.contains("label = bq_lowshelf"));
        assert!(json.contains("label = bq_highshelf"));
        assert!(json.contains("\"Gain\" = 0.0"));
        assert_eq!(json.matches("output = ").count(), NUM_BANDS);
    }
}
