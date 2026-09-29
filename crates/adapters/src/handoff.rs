//! Later invocations hand their open request to the diffz already running on a state directory,
//! over a private Unix socket beside its writer lock. The lock, not the socket, says whether one
//! is running. Each side writes one JSON line: a versioned request, then an acknowledgement.
use crate::{Result, store};
use diffz_core::provider::OpenRequest;
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const VERSION: u32 = 1;
const LIMIT: u64 = 64 * 1024;
/// How long a request waits on an instance that holds the lock but is not listening yet.
const STARTUP: Duration = Duration::from_secs(3);
const ACK: Duration = Duration::from_secs(10);

/// `request` is `None` when the invocation named no source; the window then only comes forward.
#[derive(Serialize, Deserialize)]
struct Message {
    version: u32,
    request: Option<OpenRequest>,
}
#[derive(Deserialize)]
struct Header {
    version: u32,
}
#[derive(Serialize, Deserialize)]
struct Ack {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// No instance holds the lock; any socket left behind has been removed.
    NotRunning,
    Accepted,
    Refused(String),
}
pub fn socket_path(dir: &Path) -> PathBuf {
    dir.join("handoff.sock")
}
fn read_line(stream: &UnixStream) -> Result<String> {
    let mut line = String::new();
    BufReader::new(stream.take(LIMIT)).read_line(&mut line)?;
    Ok(line)
}
/// Gives `request` to the instance running on `dir`, waiting for it to take or refuse it.
pub fn send(dir: &Path, request: Option<&OpenRequest>) -> Result<Outcome> {
    let path = socket_path(dir);
    let deadline = Instant::now() + STARTUP;
    let mut stream = loop {
        match UnixStream::connect(&path) {
            Ok(stream) => break stream,
            Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => {
                if !store::in_use(dir)? {
                    // Left by an instance that ended without removing it.
                    if let Err(e) = std::fs::remove_file(&path)
                        && e.kind() != ErrorKind::NotFound
                    {
                        return Err(e.into());
                    }
                    return Ok(Outcome::NotRunning);
                }
                if Instant::now() >= deadline {
                    return Err(
                        "another diffz instance is using this state directory but is not taking requests"
                            .into(),
                    );
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            // A path too long for a socket address: no instance can be listening on it.
            Err(e) if e.kind() == ErrorKind::InvalidInput => {
                if store::in_use(dir)? {
                    return Err("another diffz instance is using this state directory, whose path is too long to take requests".into());
                }
                return Ok(Outcome::NotRunning);
            }
            Err(e) => return Err(e.into()),
        }
    };
    stream.set_read_timeout(Some(ACK))?;
    stream.set_write_timeout(Some(ACK))?;
    let mut message = serde_json::to_vec(&Message {
        version: VERSION,
        request: request.cloned(),
    })?;
    message.push(b'\n');
    stream.write_all(&message)?;
    let line = read_line(&stream)?;
    if line.is_empty() {
        return Err("the running diffz closed the connection without answering".into());
    }
    let ack: Ack = serde_json::from_str(&line)?;
    Ok(match ack.error {
        None => Outcome::Accepted,
        Some(error) => Outcome::Refused(error),
    })
}
pub struct Listener {
    socket: UnixListener,
    path: PathBuf,
}
impl Listener {
    /// Call only while holding the writer lock of `dir`: any socket already there is stale.
    pub fn bind(dir: &Path) -> Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        let path = socket_path(dir);
        if let Err(e) = std::fs::remove_file(&path)
            && e.kind() != ErrorKind::NotFound
        {
            return Err(e.into());
        }
        let socket = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self { socket, path })
    }
    /// Waits for the next request. A connection that sends no readable request of this
    /// version is answered with an error and skipped.
    pub fn accept(&self) -> Result<Incoming> {
        loop {
            let (stream, _) = self.socket.accept()?;
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
            let line = read_line(&stream).unwrap_or_default();
            let mut incoming = Incoming {
                request: None,
                stream,
            };
            let error = match serde_json::from_str::<Header>(&line) {
                Ok(h) if h.version != VERSION => format!(
                    "the running diffz reads handoff version {VERSION}, not {}; restart it",
                    h.version
                ),
                Ok(_) => match serde_json::from_str::<Message>(&line) {
                    Ok(message) => {
                        incoming.request = message.request;
                        return Ok(incoming);
                    }
                    Err(e) => format!("the running diffz could not read the request: {e}"),
                },
                Err(e) => format!("the running diffz could not read the request: {e}"),
            };
            incoming.reply(Err(error));
        }
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
pub struct Incoming {
    pub request: Option<OpenRequest>,
    stream: UnixStream,
}
impl Incoming {
    /// Tells the sender whether the window took the request. A sender that has gone is ignored.
    pub fn reply(mut self, result: std::result::Result<(), String>) {
        if let Ok(mut ack) = serde_json::to_vec(&Ack {
            version: VERSION,
            error: result.err(),
        }) {
            ack.push(b'\n');
            let _ = self.stream.write_all(&ack);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diffz_core::domain::ProviderId;

    fn request() -> OpenRequest {
        OpenRequest::Remote {
            provider: ProviderId::GITHUB,
            address: "owner/repo#1".into(),
        }
    }
    /// Answers each request with `answer`, from another thread, like the window does.
    fn serve(
        listener: Listener,
        answers: Vec<std::result::Result<(), String>>,
    ) -> std::thread::JoinHandle<Vec<Option<OpenRequest>>> {
        std::thread::spawn(move || {
            answers
                .into_iter()
                .map(|answer| {
                    let mut incoming = listener.accept().unwrap();
                    let request = incoming.request.take();
                    incoming.reply(answer);
                    request
                })
                .collect()
        })
    }
    #[test]
    fn requests_are_acknowledged_or_refused() {
        let dir = tempfile::tempdir().unwrap();
        let listener = Listener::bind(dir.path()).unwrap();
        let server = serve(
            listener,
            vec![Ok(()), Err("unsaved changes".into()), Ok(())],
        );
        assert_eq!(
            send(dir.path(), Some(&request())).unwrap(),
            Outcome::Accepted
        );
        assert_eq!(
            send(dir.path(), Some(&request())).unwrap(),
            Outcome::Refused("unsaved changes".into())
        );
        assert_eq!(send(dir.path(), None).unwrap(), Outcome::Accepted);
        assert_eq!(
            server.join().unwrap(),
            vec![Some(request()), Some(request()), None]
        );
    }
    #[test]
    fn socket_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let _listener = Listener::bind(dir.path()).unwrap();
        let mode = std::fs::metadata(socket_path(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    #[test]
    fn other_versions_are_refused_and_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let listener = Listener::bind(dir.path()).unwrap();
        let server = serve(listener, vec![Ok(())]);
        let mut old = UnixStream::connect(socket_path(dir.path())).unwrap();
        old.write_all(b"{\"version\":99,\"request\":null}\n")
            .unwrap();
        let ack: Ack = serde_json::from_str(&read_line(&old).unwrap()).unwrap();
        assert!(ack.error.unwrap().contains("version 1, not 99"));
        assert_eq!(send(dir.path(), None).unwrap(), Outcome::Accepted);
        assert_eq!(server.join().unwrap(), vec![None]);
    }
    #[test]
    fn nothing_running_is_reported_without_a_socket() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            send(dir.path(), Some(&request())).unwrap(),
            Outcome::NotRunning
        );
    }
    #[test]
    fn stale_socket_is_removed_when_the_lock_is_free() {
        let dir = tempfile::tempdir().unwrap();
        // A listener that is gone leaves its socket file behind; connecting is refused.
        drop(UnixListener::bind(socket_path(dir.path())).unwrap());
        std::fs::write(dir.path().join("writer.lock"), "").unwrap();
        assert!(socket_path(dir.path()).exists());
        assert_eq!(
            send(dir.path(), Some(&request())).unwrap(),
            Outcome::NotRunning
        );
        assert!(!socket_path(dir.path()).exists());
    }
    #[test]
    fn a_held_lock_without_a_listener_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let _store = store::Store::open(dir.path()).unwrap();
        assert!(store::in_use(dir.path()).unwrap());
        let error = send(dir.path(), Some(&request())).unwrap_err();
        assert!(error.to_string().contains("not taking requests"), "{error}");
    }
    #[test]
    fn a_state_directory_too_long_for_a_socket_still_launches() {
        let dir = tempfile::tempdir().unwrap();
        let long = dir.path().join("d".repeat(120));
        assert_eq!(send(&long, Some(&request())).unwrap(), Outcome::NotRunning);
        assert!(Listener::bind(&long).is_err());
    }
    #[test]
    fn binding_replaces_a_stale_socket() {
        let dir = tempfile::tempdir().unwrap();
        drop(UnixListener::bind(socket_path(dir.path())).unwrap());
        let listener = Listener::bind(dir.path()).unwrap();
        let server = serve(listener, vec![Ok(())]);
        assert_eq!(send(dir.path(), None).unwrap(), Outcome::Accepted);
        server.join().unwrap();
    }
}
