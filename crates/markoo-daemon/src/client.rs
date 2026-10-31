//! Asking a running daemon for a sync, from a Git hook. It uses a plain socket:
//! the request is a short HTTP message sent to `127.0.0.1`, and the caller has
//! no async runtime.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

use crate::token;

/// Says who is asking, for the history.
const HOOK_BODY: &str = r#"{"trigger":"hook"}"#;

/// How long a daemon has to accept the connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
/// How long it has to answer: it only queues the job.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);

/// The answer to a sync request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncRequest {
    /// The daemon queued the sync.
    Queued,
    /// No daemon of this data folder is listening: nothing was asked.
    Unavailable,
    /// The daemon refused or did not answer.
    Refused(String),
}

/// Asks the daemon of a data folder to sync a pair.
pub fn request_sync(data_dir: &Path, port: u16, pair: &str) -> SyncRequest {
    let Ok(token) = std::fs::read_to_string(token::path(data_dir)) else {
        return SyncRequest::Unavailable;
    };
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) else {
        return SyncRequest::Unavailable;
    };
    let request = format!(
        "POST /pairs/{pair}/sync HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\
         Authorization: Bearer {}\r\nContent-Length: {}\r\n\r\n{HOOK_BODY}",
        token.trim(),
        HOOK_BODY.len()
    );
    let mut response = String::new();
    let exchange = stream
        .set_read_timeout(Some(ANSWER_TIMEOUT))
        .and_then(|()| stream.write_all(request.as_bytes()))
        .and_then(|()| stream.read_to_string(&mut response));
    if let Err(error) = exchange {
        return SyncRequest::Refused(format!("no answer from the daemon: {error}"));
    }
    match response.split_whitespace().nth(1) {
        Some(status) if status.starts_with('2') => SyncRequest::Queued,
        // The port belongs to the daemon of another data folder.
        Some("401") => SyncRequest::Unavailable,
        Some(status) => {
            let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
            SyncRequest::Refused(format!("the daemon answered {status}: {}", body.trim()))
        }
        None => SyncRequest::Refused("the daemon sent an invalid answer".to_owned()),
    }
}
