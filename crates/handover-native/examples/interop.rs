//! Manual Rust<->Kotlin pairing/TLS interop server for the native backend.
//!
//! Binds an ephemeral-port listener (no DNS-SD advertisement) on a throwaway
//! state directory and exposes the real [`NativeBackend`] pairing surface over
//! stdin, so a physical Android build can pair against it:
//!
//! ```sh
//! cargo run -p handover-native --example interop -- --port 24838
//! adb reverse tcp:24838 tcp:24838
//! # in the Android app, connect to the manual endpoint 127.0.0.1:24838
//! ```
//!
//! Commands: `pending`, `approve <id> <code>`, `peers`, `unpair <id>`, `quit`.
use std::io::{self, BufRead};
use std::net::TcpListener;
use std::sync::Arc;

use handover_native::NativeBackend;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut port: u16 = 24838;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--port" {
            port = args
                .next()
                .ok_or("--port requires a value")?
                .parse()
                .map_err(|_| "invalid --port value")?;
        } else {
            return Err(format!("unknown argument: {arg}").into());
        }
    }

    let dir = tempfile::tempdir()?;
    let backend = NativeBackend::open(dir.path().to_path_buf())?;
    println!("interop server identity: {}", backend.identity());
    println!("interop server state dir: {}", dir.path().display());

    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let addr = listener.local_addr()?;
    println!("interop server listening on {addr}");

    let events = backend.clone();
    std::thread::spawn(move || {
        let event = Arc::new(move |ev: handover_core::StateEvent| {
            println!("event: {ev:?}");
        });
        if let Err(error) = events.serve(listener, event) {
            eprintln!("interop server stopped: {error}");
        }
    });

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("pending") => {
                for peer in backend.pending() {
                    println!("pending {}\t{}\t{}", peer.id, peer.name, peer.code);
                }
            }
            Some("approve") => {
                let (Some(id), Some(code)) = (parts.next(), parts.next()) else {
                    eprintln!("usage: approve <id> <code>");
                    continue;
                };
                match backend.approve(id, code) {
                    Ok(()) => println!("approved {id}; waiting for phone confirmation"),
                    Err(error) => eprintln!("approve failed: {error}"),
                }
            }
            Some("peers") => {
                for peer in backend.peers() {
                    println!("peer {}\t{}\t{}", peer.id, peer.name, peer.fingerprint);
                }
            }
            Some("unpair") => {
                let Some(id) = parts.next() else {
                    eprintln!("usage: unpair <id>");
                    continue;
                };
                println!("unpaired {id}: {}", backend.unpair(id)?);
            }
            Some("quit") => break,
            Some(other) => eprintln!("unknown command: {other}"),
            None => {}
        }
    }
    Ok(())
}
