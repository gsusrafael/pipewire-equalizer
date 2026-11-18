//! The equalizer daemon.
//!
//! It owns the PipeWire equalizer node and exposes it over a Unix socket so
//! the graphical interface (and scripts) can drive it.
//!
//! The daemon:
//!
//! * loads the `filter-chain` module, which creates the equalizer sink in
//!   the running PipeWire daemon;
//! * applies presets and band changes while the audio is running;
//! * answers control requests on a Unix socket.
//!
//! Everything runs in the PipeWire main loop, so a request is applied to the
//! live node in the very thread that owns it.

mod engine;
mod server;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use pweq_core::graph::FilterGraph;
use pweq_core::preset::EqState;
use pweq_core::protocol::{Request, Response, State};
use pweq_core::store::PresetStore;

use engine::EqEngine;
use server::Server;

/// Where the configuration and the presets live.
fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PWEQ_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
        });
    base.join("pweq")
}

/// The path of the control socket.
fn socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("PWEQ_SOCKET") {
        return PathBuf::from(path);
    }
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    runtime.join("pweq.sock")
}

/// Everything the daemon knows about the current state.
#[derive(Clone)]
struct Session {
    /// The current curve.
    state: EqState,
    /// The name of the active preset, empty for a custom curve.
    preset: String,
    /// Whether the equalizer is applied.
    enabled: bool,
    /// The last error, empty when everything is fine.
    error: String,
}

impl Session {
    fn new() -> Self {
        Self {
            state: EqState::flat(),
            preset: String::new(),
            enabled: true,
            error: String::new(),
        }
    }

    fn report(&self, presets: Vec<String>) -> State {
        State {
            enabled: self.enabled,
            preset: self.preset.clone(),
            preamp_db: self.state.preamp_db,
            gains_db: self.state.gains_db.to_vec(),
            output: String::new(),
            presets,
            error: self.error.clone(),
        }
    }
}

/// What the timer callback needs to answer a request.
struct Context {
    server: Server,
    session: Session,
    engine: EqEngine,
    store: PresetStore,
}

fn main() {
    let config_dir = config_dir();
    let socket = socket_path();
    let store = PresetStore::new(config_dir.join("presets"));

    // `--print-presets` is a convenience for scripts and for the GUI.
    if std::env::args().any(|a| a == "--print-presets") {
        for preset in store.load_all() {
            println!("{}", preset.name);
        }
        return;
    }

    let mut session = Session::new();

    // Start from the Flat preset when it is available.
    if let Some(flat) = store.load_all().into_iter().find(|p| p.name == "Flat") {
        session.state = flat.state;
        session.preset = flat.name;
    }

    let engine = match EqEngine::connect() {
        Ok(engine) => engine,
        Err(e) => {
            eprintln!("pweq-daemon: cannot connect to PipeWire: {e}");
            std::process::exit(1);
        }
    };

    // The equalizer plays into the current default output, unless the
    // environment says otherwise.
    let target = std::env::var("PWEQ_TARGET").ok();

    let graph = FilterGraph::build(&session.state);
    if let Err(e) = engine.create_node(&graph, target.as_deref()) {
        eprintln!("pweq-daemon: cannot create the equalizer node: {e}");
        std::process::exit(1);
    }

    if let Err(e) = engine.discover_node() {
        eprintln!("pweq-daemon: the equalizer node did not appear: {e}");
        std::process::exit(1);
    }

    eprintln!(
        "pweq-daemon: equalizer node {} ready",
        engine.node_id().unwrap_or_default()
    );

    let server = match Server::bind(&socket) {
        Ok(server) => server,
        Err(e) => {
            eprintln!("pweq-daemon: cannot bind the control socket: {e}");
            std::process::exit(1);
        }
    };

    let context = Rc::new(RefCell::new(Context {
        server,
        session,
        engine,
        store,
    }));

    // The socket is polled from a timer in the PipeWire loop.
    let timer_context = context.clone();
    let main_loop = context.borrow().engine.main_loop().clone();

    let timer = {
        let loop_ = main_loop.loop_();
        loop_.add_timer(move |_| {
        let mut context = timer_context.borrow_mut();
        context.server.prune();

        let requests = context.server.poll();
        if requests.is_empty() {
            return;
        }

            let response = handle_requests(&requests, &mut context);
            context.server.broadcast(&response);
        })
    };

    // Poll often enough to feel immediate, rarely enough to stay cheap.
    timer
        .update_timer(
            Some(std::time::Duration::from_millis(50)),
            Some(std::time::Duration::from_millis(50)),
        )
        .into_result()
        .expect("cannot arm the poll timer");

    // Keep the node alive and let the PipeWire loop run.
    //
    // The borrow of the context is released before running the loop: the
    // poll timer lives in that loop and needs the context itself.
    loop {
        let main_loop = {
            let context = context.borrow();
            context.engine.main_loop().clone()
        };
        main_loop.run();
    }
}

/// Apply a batch of requests and build the answer.
fn handle_requests(requests: &[Request], context: &mut Context) -> Response {
    let presets: Vec<String> = context.store.load_all().into_iter().map(|p| p.name).collect();
    let mut response = Response::Ok {
        state: context.session.report(presets.clone()),
    };

    for request in requests {
        match request {
            Request::GetState | Request::ListPresets => {}
            Request::SetBand { band, gain } => {
                if *band >= context.session.state.gains_db.len() {
                    context.session.error = format!("unknown band {band}");
                    continue;
                }
                context.session.state.gains_db[*band] =
                    gain.clamp(pweq_core::GAIN_MIN, pweq_core::GAIN_MAX);
                context.session.preset.clear();
                context.session.error.clear();
                if let Err(e) = context
                    .engine
                    .set_band_gain(*band, context.session.state.gains_db[*band])
                {
                    context.session.error = e.to_string();
                }
            }
            Request::SetPreamp { db } => {
                context.session.state.preamp_db =
                    db.clamp(pweq_core::GAIN_MIN, pweq_core::GAIN_MAX);
                context.session.preset.clear();
                context.session.error.clear();
                if let Err(e) = context.engine.set_preamp(context.session.state.preamp_db) {
                    context.session.error = e.to_string();
                }
            }
            Request::LoadPreset { name } => match context.store.find(name) {
                Some(preset) => {
                    context.session.state = preset.state.clamped();
                    context.session.preset = preset.name.clone();
                    context.session.error.clear();
                    if let Err(e) = context.engine.apply_state(&context.session.state) {
                        context.session.error = e.to_string();
                    }
                }
                None => {
                    context.session.error = format!("no such preset: {name}");
                }
            },
            Request::Reset => {
                context.session.state = EqState::flat();
                context.session.preset.clear();
                context.session.error.clear();
                if let Err(e) = context.engine.apply_state(&context.session.state) {
                    context.session.error = e.to_string();
                }
            }
            Request::SetEnabled { enabled } => {
                context.session.enabled = *enabled;
                context.session.error.clear();
            }
        }

        response = Response::Ok {
            state: context.session.report(presets.clone()),
        };
    }

    response
}
