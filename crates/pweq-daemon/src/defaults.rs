//! Following the output that is playing.
//!
//! The equalizer is a virtual sink. For the audio to go through it, it has
//! to be the default sink, and the default has to follow the device the user
//! is listening to.
//!
//! Both live in the metadata of the PipeWire daemon:
//!
//! * `default.audio.sink` is the sink that is playing right now;
//! * `default.configured.audio.sink` is the one the user chose, and it is
//!   the one to restore when the equalizer is switched off.
//!
//! The metadata is read and written through the PipeWire API, so the daemon
//! does not depend on `pw-metadata` (which keeps monitoring after it sets a
//! property, and would hang the process).

use pipewire::core::CoreRc;
use pipewire::metadata::Metadata;
use pipewire::registry::GlobalObject;
use pipewire::types::ObjectType;

use std::cell::RefCell;
use std::rc::Rc;

/// The metadata key of the sink that is playing.
pub const DEFAULT_SINK: &str = "default.audio.sink";
/// The metadata key of the sink the user chose.
pub const CONFIGURED_SINK: &str = "default.configured.audio.sink";

/// The name of the equalizer sink.
pub const EQ_SINK: &str = "effect_input.pweq";

/// The default sinks of the session.
#[derive(Debug, Clone, Default)]
pub struct Defaults {
    /// The sink that is playing right now.
    pub current: Option<String>,
    /// The sink the user chose.
    pub configured: Option<String>,
}

impl Defaults {
    /// The name of the sink the audio really goes to.
    ///
    /// When the equalizer is the default, the real output is the one the
    /// user configured.
    pub fn real_output(&self) -> Option<String> {
        self.configured.clone().filter(|name| name != EQ_SINK)
    }
}

/// Read the default sinks from the metadata of the daemon.
pub fn read(engine: &crate::engine::EqEngine) -> Defaults {
    let core = engine.core();
    let mut defaults = Defaults::default();

    let Some(metadata) = bind_default_metadata(engine, core) else {
        return defaults;
    };

    // The metadata delivers its properties through events, so the current
    // values arrive when the loop runs.
    let found: Rc<RefCell<Defaults>> = Rc::new(RefCell::new(Defaults::default()));
    let captured = found.clone();
    let watcher = found.clone();

    let _listener = metadata
        .add_listener_local()
        .property(move |_subject, key, _type, value| {
            let Some(key) = key else { return 0 };
            let Some(value) = value else { return 0 };
            let Some(name) = parse_name(value) else {
                return 0;
            };
            let mut found = captured.borrow_mut();
            match key {
                DEFAULT_SINK => found.current = Some(name),
                CONFIGURED_SINK => found.configured = Some(name),
                _ => {}
            }
            0
        })
        .register();

    // Run the loop until both defaults have been seen.
    {
        let watcher = watcher.clone();
        engine.wait_for(move || {
            let found = watcher.borrow();
            found.current.is_some() && found.configured.is_some()
        });
    }

    defaults = found.borrow().clone();
    defaults
}

/// Set the sink the user chose, which is the default that plays.
pub fn set_configured(engine: &crate::engine::EqEngine, name: &str) -> bool {
    let core = engine.core();
    let Some(metadata) = bind_default_metadata(engine, core) else {
        eprintln!("pweq-daemon: the default metadata was not found");
        return false;
    };

    let value = format!("{{\"name\":\"{name}\"}}");
    metadata.set_property(0, CONFIGURED_SINK, Some("Spa:String:JSON"), Some(&value));
    true
}

/// Restore the sink the user chose.
pub fn restore(engine: &crate::engine::EqEngine, defaults: &Defaults) {
    if let Some(name) = defaults.real_output() {
        set_configured(engine, &name);
    }
}

/// Bind the "default" metadata of the daemon.
fn bind_default_metadata(
    engine: &crate::engine::EqEngine,
    core: &CoreRc,
) -> Option<Metadata> {
    let registry = core.get_registry_rc().ok()?;
    let registry = Rc::new(registry);

    // The metadata is a global of the daemon, so it is found through the
    // registry.
    let global: Rc<RefCell<Option<GlobalObject<pipewire::properties::PropertiesBox>>>> =
        Rc::new(RefCell::new(None));
    {
        let captured = global.clone();
        let _listener = registry
            .add_listener_local()
            .global(move |object| {
                if object.type_ != ObjectType::Metadata {
                    return;
                }
                let Some(props) = object.props else {
                    return;
                };
                if props.get("metadata.name") == Some("default") {
                    *captured.borrow_mut() = Some(object.to_owned());
                }
            })
            .register();

        // The registry delivers the globals that already exist when the
        // main loop runs.
        engine.wait_for(|| global.borrow().is_some());
    }

    let taken = global.borrow_mut().take();
    let Some(global) = taken else {
        return None;
    };

    registry.bind::<Metadata, _>(&global).ok()
}

/// Extract the "name" field of a JSON object.
fn parse_name(value: &str) -> Option<String> {
    let start = value.find("\"name\"")?;
    let rest = &value[start..];
    let colon = rest.find(':')?;
    let rest = &rest[colon + 1..];
    let open = rest.find('"')?;
    let rest = &rest[open + 1..];
    let close = rest.find('"')?;
    Some(rest[..close].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_name_of_a_json_object() {
        assert_eq!(
            parse_name("{\"name\":\"alsa_output.foo\"}"),
            Some("alsa_output.foo".to_string())
        );
    }

    #[test]
    fn the_equalizer_is_not_a_real_output() {
        let defaults = Defaults {
            current: Some(EQ_SINK.to_string()),
            configured: Some(EQ_SINK.to_string()),
        };
        assert_eq!(defaults.real_output(), None);

        let defaults = Defaults {
            current: Some(EQ_SINK.to_string()),
            configured: Some("alsa_output.foo".to_string()),
        };
        assert_eq!(defaults.real_output(), Some("alsa_output.foo".to_string()));
    }
}
