//! Talking to the daemon over the control socket.
//!
//! The socket speaks the JSON protocol described in `pweq_core::protocol`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use pweq_core::protocol::{decode_response, encode_request, Request, Response, State};

/// Errors raised while talking to the daemon.
#[derive(Debug)]
pub enum ClientError {
    /// The socket could not be reached.
    Connect(std::io::Error),
    /// The daemon answered something that could not be understood.
    Protocol(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Connect(e) => write!(f, "cannot reach the daemon: {e}"),
            ClientError::Protocol(m) => write!(f, "protocol error: {m}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<std::io::Error> for ClientError {
    fn from(e: std::io::Error) -> Self {
        ClientError::Connect(e)
    }
}

/// Where the control socket lives.
pub fn socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("PWEQ_SOCKET") {
        return PathBuf::from(path);
    }
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    runtime.join("pweq.sock")
}

/// A client of the daemon.
pub struct Client {
    path: PathBuf,
}

impl Client {
    /// Create a client for the default socket.
    pub fn new() -> Self {
        Self {
            path: socket_path(),
        }
    }

    /// Send a request and wait for the answer.
    pub fn send(&self, request: &Request) -> Result<Response, ClientError> {
        let mut stream = UnixStream::connect(&self.path)?;
        stream.write_all(encode_request(request).as_bytes())?;
        stream.write_all(b"\n")?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line)?;

        decode_response(line.trim())
            .map_err(|e| ClientError::Protocol(format!("cannot read the answer: {e}")))
    }

    /// Send a request and return the state it carries.
    pub fn state(&self, request: &Request) -> Result<State, ClientError> {
        match self.send(request)? {
            Response::Ok { state } => Ok(state),
            Response::Error { message } => Err(ClientError::Protocol(message)),
        }
    }

    /// Ask for the current state.
    pub fn get_state(&self) -> Result<State, ClientError> {
        self.state(&Request::GetState)
    }
}
