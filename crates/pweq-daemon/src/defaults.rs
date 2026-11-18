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
//! The metadata is read and written with `pw-metadata`, which talks to the
//! daemon over the same socket any client uses.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

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
///
/// `pw-metadata` keeps monitoring after it prints, so it is read line by
/// line and stopped as soon as the defaults have been seen.
pub fn read() -> Defaults {
    let mut defaults = Defaults::default();

    let Ok(mut child) = Command::new("pw-metadata")
        .arg("-n")
        .arg("default")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return defaults;
    };

    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return defaults;
    };

    let reader = BufReader::new(stdout);
    for line in reader.lines().map_while(Result::ok) {
        // update: id:0 key:'default.audio.sink' value:'{"name":"alsa_output..."}' type:'Spa:String:JSON'
        let Some(key) = between(&line, "key:'", "'") else {
            continue;
        };
        if key != DEFAULT_SINK && key != CONFIGURED_SINK {
            continue;
        }
        let Some(value) = between(&line, "value:'", "'") else {
            continue;
        };
        let Some(name) = parse_name(&value) else {
            continue;
        };

        match key.as_str() {
            DEFAULT_SINK => defaults.current = Some(name),
            CONFIGURED_SINK => defaults.configured = Some(name),
            _ => {}
        }

        if defaults.current.is_some() && defaults.configured.is_some() {
            break;
        }
    }

    let _ = child.kill();
    let _ = child.wait();

    defaults
}

/// Set the sink the user chose, which is the default that plays.
pub fn set_configured(name: &str) -> bool {
    let value = format!("{{\"name\":\"{name}\"}}");
    Command::new("pw-metadata")
        .args([
            "0",
            CONFIGURED_SINK,
            &value,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Restore the sink the user chose.
pub fn restore(defaults: &Defaults) {
    if let Some(name) = defaults.real_output() {
        set_configured(&name);
    }
}

/// Extract the text between two markers.
fn between(text: &str, start: &str, end: &str) -> Option<String> {
    let from = text.find(start)? + start.len();
    let rest = &text[from..];
    let to = rest.find(end)?;
    Some(rest[..to].to_string())
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
    fn extracts_the_text_between_markers() {
        assert_eq!(
            between("key:'default.audio.sink' value:'x'", "key:'", "'"),
            Some("default.audio.sink".to_string())
        );
    }
}
