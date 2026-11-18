//! The control socket.
//!
//! The daemon listens on a Unix socket and answers newline delimited JSON
//! requests, as described in `pweq_core::protocol`.
//!
//! The socket is served from the PipeWire main loop, so every change is
//! applied to the live node in the same thread that owns it.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};

use pweq_core::protocol::{decode_request, encode_response, Request, Response};

/// A connected client.
struct Client {
    stream: UnixStream,
    buffer: Vec<u8>,
    /// Set when the client closed the connection.
    closed: bool,
}

/// The socket server.
pub struct Server {
    listener: UnixListener,
    clients: Vec<Client>,
}

impl Server {
    /// Bind the socket, replacing a stale one if it is there.
    pub fn bind(path: &std::path::Path) -> std::io::Result<Self> {
        if path.exists() {
            // A socket left behind by a daemon that did not exit cleanly.
            let _ = std::fs::remove_file(path);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            clients: Vec::new(),
        })
    }

    /// Accept new clients and read what is pending.
    ///
    /// Returns the requests that arrived, in order.
    pub fn poll(&mut self) -> Vec<Request> {
        let mut requests = Vec::new();

        // New connections.
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(true);
                    self.clients.push(Client {
                        stream,
                        buffer: Vec::new(),
                        closed: false,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }

        // Pending data.
        let mut keep = Vec::with_capacity(self.clients.len());
        for mut client in std::mem::take(&mut self.clients) {
            let mut chunk = [0u8; 1024];
            loop {
                match client.stream.read(&mut chunk) {
                    Ok(0) => {
                        client.closed = true;
                        break;
                    }
                    Ok(n) => client.buffer.extend_from_slice(&chunk[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        client.closed = true;
                        break;
                    }
                }
            }

            // Complete lines are requests.
            while let Some(position) = client.buffer.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = client.buffer.drain(..=position).collect();
                let text = String::from_utf8_lossy(&line);
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                match decode_request(text) {
                    Ok(request) => requests.push(request),
                    Err(e) => {
                        let response = Response::Error {
                            message: format!("invalid request: {e}"),
                        };
                        let _ = writeln!(client.stream, "{}", encode_response(&response));
                    }
                }
            }

            keep.push(client);
        }
        self.clients = keep;

        requests
    }

    /// Send an answer to every connected client.
    pub fn broadcast(&mut self, response: &Response) {
        let line = format!("{}\n", encode_response(response));
        for client in &mut self.clients {
            let _ = client.stream.write_all(line.as_bytes());
            let _ = client.stream.flush();
        }
    }

    /// Drop the clients that went away.
    ///
    /// A client that closed its side is detected by the read that returns
    /// zero bytes, so the connections are marked there.
    pub fn prune(&mut self) {
        self.clients.retain(|client| !client.closed);
    }
}
