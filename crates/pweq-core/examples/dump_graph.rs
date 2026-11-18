//! Print the filter graph of a preset, to check it against the tools.

use pweq_core::graph::FilterGraph;
use pweq_core::preset::EqState;

fn main() {
    let mut state = EqState::flat();
    state.gains_db = [0.0; pweq_core::NUM_BANDS];
    state.preamp_db = 0.0;

    let graph = FilterGraph::build(&state);
    println!("{}", graph.to_spa_json());
}
