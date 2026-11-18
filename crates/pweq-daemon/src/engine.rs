//! The PipeWire side of the equalizer.
//!
//! The equalizer is a `filter-chain` node built out of native PipeWire
//! biquads: a preamp followed by 15 bands (low shelf, 13 peaking filters and
//! a high shelf).
//!
//! Two things happen here:
//!
//! * the node is created, by loading the `filter-chain` module in the
//!   *client* context, which registers it in the running PipeWire daemon so
//!   it can be routed like any other sink;
//! * the band gains are changed while the audio is running, by setting the
//!   `params` property of the node, which the module applies without
//!   recreating the filter.
//!
//! The control ports of the graph are addressed as `<node name>:<port>`,
//! for example `band_peaking1:Gain`.

use pweq_core::graph::{band_node, FilterGraph};
use pweq_core::preset::{EqState, NUM_BANDS};

use pipewire::context::ContextRc;
use pipewire::core::CoreRc;
use pipewire::main_loop::MainLoopRc;
use pipewire::node::Node;
use pipewire::registry::RegistryRc;
use pipewire::spa::param::ParamType;
use pipewire::spa::pod::Pod;
use pipewire::spa::utils::SpaTypes;
use pipewire::types::ObjectType;

use std::cell::RefCell;
use std::rc::Rc;

/// The name of the sink the equalizer presents itself as.
pub const EQ_SINK_NAME: &str = "effect_input.pweq";
/// The name of the node that plays the equalised audio into the real output.
pub const EQ_OUTPUT_NAME: &str = "effect_output.pweq";

/// The name of the `filter-chain` module to load.
const FILTER_CHAIN_MODULE: &str = "libpipewire-module-filter-chain";

/// Errors raised while talking to PipeWire.
#[derive(Debug)]
pub enum EqError {
    /// The PipeWire connection failed.
    PipeWire(pipewire::Error),
    /// The equaliser node could not be found.
    NodeNotFound,
    /// The control port of a band does not exist.
    UnknownBand(usize),
    /// The filter chain module could not be loaded.
    ModuleLoad(String),
}

impl std::fmt::Display for EqError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EqError::PipeWire(e) => write!(f, "pipewire error: {e}"),
            EqError::NodeNotFound => write!(f, "equalizer node not found"),
            EqError::UnknownBand(b) => write!(f, "unknown band {b}"),
            EqError::ModuleLoad(m) => write!(f, "cannot load filter-chain module: {m}"),
        }
    }
}

impl std::error::Error for EqError {}

impl From<pipewire::Error> for EqError {
    fn from(e: pipewire::Error) -> Self {
        EqError::PipeWire(e)
    }
}

/// A live connection to PipeWire that hosts an equalizer node.
pub struct EqEngine {
    main_loop: MainLoopRc,
    context: ContextRc,
    core: CoreRc,
    /// The registry, kept alive for as long as the node proxy is used.
    registry: RefCell<Option<Rc<RegistryRc>>>,
    /// The equalizer node, once it has been found in the daemon.
    node: RefCell<Option<Node>>,
    /// The id of the equalizer node.
    node_id: RefCell<Option<u32>>,
    /// Handle of the loaded module, kept alive while the node must exist.
    module: RefCell<Option<*mut pipewire::sys::pw_impl_module>>,
}

impl EqEngine {
    /// Connect to the running PipeWire daemon.
    pub fn connect() -> Result<Self, EqError> {
        pipewire::init();

        let main_loop = MainLoopRc::new(None)?;
        let context = ContextRc::new(&main_loop, None)?;
        let core = context.connect_rc(None)?;

        Ok(Self {
            main_loop,
            context,
            core,
            registry: RefCell::new(None),
            node: RefCell::new(None),
            node_id: RefCell::new(None),
            module: RefCell::new(None),
        })
    }

    /// The PipeWire main loop, to be run by the caller.
    pub fn main_loop(&self) -> &MainLoopRc {
        &self.main_loop
    }

    /// The id of the equalizer node, once it has been found.
    pub fn node_id(&self) -> Option<u32> {
        *self.node_id.borrow()
    }

    /// Create the equalizer node with the given graph.
    ///
    /// The module is loaded in the client context; it then registers its
    /// streams in the running daemon, so the node is visible to every client
    /// and can be routed like any other sink.
    pub fn create_node(&self, graph: &FilterGraph) -> Result<(), EqError> {
        let args = format!(
            "node.description = \"PipeWire Equalizer\" \
             media.name = \"PipeWire Equalizer\" \
             {} \
             audio.channels = 2 \
             audio.position = [ FL FR ] \
             capture.props = {{ node.name = \"{EQ_SINK_NAME}\" media.class = Audio/Sink priority.session = 0 }} \
             playback.props = {{ node.name = \"{EQ_OUTPUT_NAME}\" node.passive = true }}",
            graph.to_spa_json()
        );

        let name = std::ffi::CString::new(FILTER_CHAIN_MODULE)
            .map_err(|e| EqError::ModuleLoad(e.to_string()))?;
        let args_c =
            std::ffi::CString::new(args).map_err(|e| EqError::ModuleLoad(e.to_string()))?;

        let module = unsafe {
            pipewire::sys::pw_context_load_module(
                self.context.as_raw_ptr(),
                name.as_ptr(),
                args_c.as_ptr(),
                std::ptr::null_mut(),
            )
        };

        if module.is_null() {
            return Err(EqError::ModuleLoad(
                "pw_context_load_module returned null".into(),
            ));
        }

        *self.module.borrow_mut() = Some(module);
        Ok(())
    }

    /// Destroy the equalizer node.
    pub fn destroy_node(&self) {
        if let Some(module) = self.module.borrow_mut().take() {
            unsafe {
                pipewire::sys::pw_impl_module_destroy(module);
            }
        }
        *self.node.borrow_mut() = None;
        *self.node_id.borrow_mut() = None;
    }

    /// Look for the equalizer node in the daemon and keep a proxy to it.
    ///
    /// The registry delivers the globals that already exist when it is
    /// created, so a few round trips are enough to find the node.
    pub fn discover_node(&self) -> Result<(), EqError> {
        let registry: RegistryRc = self.core.get_registry_rc()?;
        let registry = Rc::new(registry);
        let weak = registry.downgrade();
        let found: Rc<RefCell<Option<(u32, Node)>>> = Rc::new(RefCell::new(None));

        {
            let found_in_closure = found.clone();
            let _listener = registry
                .add_listener_local()
                .global(move |global| {
                    if global.type_ != ObjectType::Node {
                        return;
                    }
                    let Some(props) = global.props else {
                        return;
                    };
                    if props.get("node.name") != Some(EQ_SINK_NAME) {
                        return;
                    }
                    let Some(registry) = weak.upgrade() else {
                        return;
                    };
                    if let Ok(node) = registry.bind::<Node, _>(global) {
                        *found_in_closure.borrow_mut() = Some((global.id, node));
                    }
                })
                .register();

            // Run the loop until the registry has delivered the node.
            // `iterate` processes the pending events and returns, which is
            // what a bounded wait needs; `run` would block forever.
            for _ in 0..200 {
                self.main_loop
                    .loop_()
                    .iterate(pipewire::loop_::Timeout::Finite(
                        std::time::Duration::from_millis(50),
                    ));
                if found.borrow().is_some() {
                    break;
                }
            }
        }

        let found_node = found.borrow_mut().take();
        match found_node {
            Some((id, node)) => {
                *self.node_id.borrow_mut() = Some(id);
                *self.node.borrow_mut() = Some(node);
                *self.registry.borrow_mut() = Some(registry);
                Ok(())
            }
            None => Err(EqError::NodeNotFound),
        }
    }

    /// Change the gain of one band while the audio is running.
    pub fn set_band_gain(&self, band: usize, gain_db: f32) -> Result<(), EqError> {
        if band >= NUM_BANDS {
            return Err(EqError::UnknownBand(band));
        }
        let key = format!("{}:Gain", band_node(band));
        self.set_control(&key, gain_db)
    }

    /// Change the preamp while the audio is running.
    pub fn set_preamp(&self, preamp_db: f32) -> Result<(), EqError> {
        let mult = pweq_core::db_to_linear(preamp_db) as f32;
        let key = format!("{}:Mult", pweq_core::NODE_PREAMP);
        self.set_control(&key, mult)
    }

    /// Apply a whole curve at once.
    pub fn apply_state(&self, state: &EqState) -> Result<(), EqError> {
        self.set_preamp(state.preamp_db)?;
        for (band, gain) in state.gains_db.iter().enumerate() {
            self.set_band_gain(band, *gain)?;
        }
        Ok(())
    }

    /// Set a single control port of the filter graph.
    fn set_control(&self, key: &str, value: f32) -> Result<(), EqError> {
        let node = self.node.borrow();
        let node = node.as_ref().ok_or(EqError::NodeNotFound)?;

        let pod = ParamsPod::build(&[(key.to_string(), value)])?;
        node.set_param(ParamType::Props, 0, pod.pod());
        Ok(())
    }
}

/// A `SPA_PARAM_Props` pod holding a `params` struct.
///
/// The `params` property is a struct of alternating string keys and values,
/// which is how the `filter-chain` module reads its control ports. The pod
/// borrows its buffer, so the buffer is owned by this struct.
pub struct ParamsPod {
    data: Vec<u8>,
}

impl ParamsPod {
    /// Build a pod with the given control entries.
    pub fn build(entries: &[(String, f32)]) -> Result<Self, EqError> {
        use pipewire::spa::pod::builder::Builder;

        let mut data = Vec::<u8>::new();
        {
            let mut builder = Builder::new(&mut data);
            build_params(&mut builder, entries)?;
        }

        if Pod::from_bytes(&data).is_none() {
            return Err(EqError::ModuleLoad("cannot parse the pod".into()));
        }

        Ok(Self { data })
    }

    /// The pod itself.
    pub fn pod(&self) -> &Pod {
        // The buffer was validated when the pod was built.
        Pod::from_bytes(&self.data).expect("the pod was validated on creation")
    }
}

/// Build a `SPA_PARAM_Props` pod with a `params` struct inside.
fn build_params(
    builder: &mut pipewire::spa::pod::builder::Builder,
    entries: &[(String, f32)],
) -> Result<(), EqError> {
    use pipewire::spa::pod::builder::{builder_add, Builder};

    // A single entry is the common case and the macro handles it cleanly.
    if entries.len() == 1 {
        let (key, value) = &entries[0];
        let result = builder_add!(
            builder,
            Object(SpaTypes::ObjectParamProps.as_raw(), ParamType::Props.as_raw()) {
                pipewire::spa::sys::SPA_PROP_params => Struct {
                    String(key.as_str()),
                    Float(*value),
                }
            }
        );
        if let Err(e) = result {
            return Err(EqError::ModuleLoad(format!("cannot build pod: {e:?}")));
        }
        return Ok(());
    }

    // More than one entry: the macro cannot express it, so the pod is built
    // with the lower level API.
    let mut frame = std::mem::MaybeUninit::uninit();
    let type_ = SpaTypes::ObjectParamProps.as_raw();
    let id = ParamType::Props.as_raw();

    unsafe {
        let res = pipewire::spa::sys::spa_pod_builder_push_object(
            builder.as_raw_ptr(),
            frame.as_mut_ptr(),
            type_,
            id,
        );
        if res < 0 {
            return Err(EqError::ModuleLoad("push_object failed".into()));
        }

        let res = pipewire::spa::sys::spa_pod_builder_prop(
            builder.as_raw_ptr(),
            pipewire::spa::sys::SPA_PROP_params,
            0,
        );
        if res < 0 {
            return Err(EqError::ModuleLoad("prop failed".into()));
        }

        let mut inner = std::mem::MaybeUninit::uninit();
        let res = pipewire::spa::sys::spa_pod_builder_push_struct(
            builder.as_raw_ptr(),
            inner.as_mut_ptr(),
        );
        if res < 0 {
            return Err(EqError::ModuleLoad("push_struct failed".into()));
        }

        for (key, value) in entries {
            Builder::add_string(builder, key)
                .map_err(|e| EqError::ModuleLoad(format!("add_string failed: {e:?}")))?;
            Builder::add_float(builder, *value)
                .map_err(|e| EqError::ModuleLoad(format!("add_float failed: {e:?}")))?;
        }

        pipewire::spa::sys::spa_pod_builder_pop(builder.as_raw_ptr(), inner.assume_init_mut());
        pipewire::spa::sys::spa_pod_builder_pop(builder.as_raw_ptr(), frame.assume_init_mut());
    }

    Ok(())
}
