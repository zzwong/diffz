//! Later invocations hand their open request to the diffz already running on a state directory,
//! over a private Unix socket beside its writer lock. The lock, not the socket, says whether one
//! is running. Each side writes one JSON line: a versioned request, then an acknowledgement.
//! Invocations that find none running take turns on a launch lock, so only one starts a window.
use crate::{Result, store};
use diffz_core::provider::OpenRequest;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
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
/// How long a window just launched has to start taking requests.
const LAUNCH: Duration = Duration::from_secs(5);
const LOG_LIMIT: u64 = 1024 * 1024;

/// The write permissions a window started with. A request asking for more is refused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Writes {
    pub github: bool,
    pub gitlab: bool,
}
/// `request` is `None` when the invocation named no source; the window then only comes forward.
#[derive(Serialize, Deserialize)]
struct Message {
    version: u32,
    request: Option<OpenRequest>,
    #[serde(default)]
    writes: Writes,
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
    /// The instance closed the connection or did not answer in time, as one that is closing does.
    Unanswered,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Delivery {
    HandedOff,
    /// A new window opens the request and is already taking others.
    Launched,
}
pub fn socket_path(dir: &Path) -> PathBuf {
    dir.join("handoff.sock")
}
pub fn log_path(dir: &Path) -> PathBuf {
    dir.join("diffz.log")
}
/// The log a window started in the background appends its stderr to, emptied past 1 MiB.
pub fn log_file(dir: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    store::private_dir(dir)?;
    let log = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(log_path(dir))?;
    if log.metadata()?.len() > LOG_LIMIT {
        log.set_len(0)?;
    }
    Ok(log)
}
fn read_line(stream: &UnixStream) -> std::io::Result<String> {
    let mut line = String::new();
    BufReader::new(stream.take(LIMIT)).read_line(&mut line)?;
    Ok(line)
}
fn unanswered(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        ErrorKind::WouldBlock
            | ErrorKind::TimedOut
            | ErrorKind::BrokenPipe
            | ErrorKind::ConnectionReset
    )
}
/// Gives `request` to the instance running on `dir`, waiting for it to take or refuse it.
pub fn send(dir: &Path, request: Option<&OpenRequest>, writes: Writes) -> Result<Outcome> {
    send_within(dir, request, writes, STARTUP)
}
fn send_within(
    dir: &Path,
    request: Option<&OpenRequest>,
    writes: Writes,
    startup: Duration,
) -> Result<Outcome> {
    let path = socket_path(dir);
    let deadline = Instant::now() + startup;
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
        writes,
    })?;
    message.push(b'\n');
    let line = match stream.write_all(&message).and_then(|()| read_line(&stream)) {
        Ok(line) if !line.is_empty() => line,
        Ok(_) => return Ok(Outcome::Unanswered),
        Err(e) if unanswered(&e) => return Ok(Outcome::Unanswered),
        Err(e) => return Err(e.into()),
    };
    let ack: Ack = serde_json::from_str(&line)?;
    Ok(match ack.error {
        None => Outcome::Accepted,
        Some(error) => Outcome::Refused(error),
    })
}
/// Gives `request` to the window running on `dir`, or calls `launch` to start one that opens it.
/// Invocations that find no window take the launch lock in turn and look again, so the first
/// starts the window and the rest hand their requests to it. The lock is held until the new
/// window takes requests.
pub fn deliver(
    dir: &Path,
    request: Option<&OpenRequest>,
    writes: Writes,
    launch: impl FnOnce() -> Result<()>,
) -> Result<Delivery> {
    deliver_within(dir, request, writes, launch, STARTUP, LAUNCH)
}
fn deliver_within(
    dir: &Path,
    request: Option<&OpenRequest>,
    writes: Writes,
    launch: impl FnOnce() -> Result<()>,
    startup: Duration,
    listen: Duration,
) -> Result<Delivery> {
    let outcome = send_within(dir, request, writes, startup)?;
    if let Some(delivery) = settle(outcome, dir, startup)? {
        return Ok(delivery);
    }
    store::private_dir(dir)?;
    // Released when this returns, after the new window takes requests.
    let lock = store::private_file(&dir.join("launch.lock"))?;
    lock.lock_exclusive()?;
    let outcome = send_within(dir, request, writes, startup)?;
    if let Some(delivery) = settle(outcome, dir, startup)? {
        return Ok(delivery);
    }
    launch()?;
    let deadline = Instant::now() + listen;
    loop {
        match UnixStream::connect(socket_path(dir)) {
            Ok(_) => break,
            // A state directory too long for a socket gets a window that takes no requests.
            Err(e) if e.kind() == ErrorKind::InvalidInput => break,
            Err(_) if Instant::now() >= deadline => {
                return Err(format!(
                    "the new diffz window did not start taking requests; see {}",
                    log_path(dir).display()
                )
                .into());
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    Ok(Delivery::Launched)
}
/// What `deliver` does after `outcome`; `None` means no window is running and one should start.
fn settle(outcome: Outcome, dir: &Path, startup: Duration) -> Result<Option<Delivery>> {
    match outcome {
        Outcome::Accepted => Ok(Some(Delivery::HandedOff)),
        Outcome::Refused(message) => Err(message.into()),
        Outcome::NotRunning => Ok(None),
        // A window that is closing answers nothing; once it lets go of the state, start another.
        Outcome::Unanswered => {
            let deadline = Instant::now() + startup;
            while store::in_use(dir)? {
                if Instant::now() >= deadline {
                    return Err(
                        "the running diffz did not answer in time; try again in a moment".into(),
                    );
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None)
        }
    }
}
/// Runs `open` to start a window on `dir`. When it fails because another instance holds the
/// state, as when two windows start at once, the request goes to that instance instead and
/// this returns `None`.
pub fn open_or_hand_off<T>(
    dir: &Path,
    request: Option<&OpenRequest>,
    writes: Writes,
    open: impl Fn() -> Result<T>,
) -> Result<Option<T>> {
    let error = match open() {
        Ok(opened) => return Ok(Some(opened)),
        Err(e) => e,
    };
    match send(dir, request, writes)? {
        Outcome::Accepted => Ok(None),
        Outcome::Refused(message) => Err(message.into()),
        // The lock was free after all, as when another invocation probed it at that moment.
        Outcome::NotRunning => open().map(Some),
        Outcome::Unanswered => Err(error),
    }
}
pub struct Listener {
    socket: UnixListener,
    path: PathBuf,
    writes: Writes,
}
impl Listener {
    /// Call only while holding the writer lock of `dir`: any socket already there is stale.
    /// `writes` are the permissions of the window the requests go to.
    pub fn bind(dir: &Path, writes: Writes) -> Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        let path = socket_path(dir);
        if let Err(e) = std::fs::remove_file(&path)
            && e.kind() != ErrorKind::NotFound
        {
            return Err(e.into());
        }
        let socket = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            socket,
            path,
            writes,
        })
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
                    Ok(message) => match missing_write_flag(message.writes, self.writes) {
                        Some(flag) => format!(
                            "the running diffz was started without {flag}; close it, then run diffz again with the flag"
                        ),
                        None => {
                            incoming.request = message.request;
                            return Ok(incoming);
                        }
                    },
                    Err(e) => format!("the running diffz could not read the request: {e}"),
                },
                Err(e) => format!("the running diffz could not read the request: {e}"),
            };
            incoming.reply(Err(error));
        }
    }
}
fn missing_write_flag(asked: Writes, granted: Writes) -> Option<&'static str> {
    if asked.github && !granted.github {
        Some("--allow-github-writes")
    } else if asked.gitlab && !granted.gitlab {
        Some("--allow-gitlab-writes")
    } else {
        None
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
    use std::sync::Arc;

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
        let listener = Listener::bind(dir.path(), Writes::default()).unwrap();
        let server = serve(
            listener,
            vec![Ok(()), Err("unsaved changes".into()), Ok(())],
        );
        assert_eq!(
            send(dir.path(), Some(&request()), Writes::default()).unwrap(),
            Outcome::Accepted
        );
        assert_eq!(
            send(dir.path(), Some(&request()), Writes::default()).unwrap(),
            Outcome::Refused("unsaved changes".into())
        );
        assert_eq!(
            send(dir.path(), None, Writes::default()).unwrap(),
            Outcome::Accepted
        );
        assert_eq!(
            server.join().unwrap(),
            vec![Some(request()), Some(request()), None]
        );
    }
    #[test]
    fn socket_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let _listener = Listener::bind(dir.path(), Writes::default()).unwrap();
        let mode = std::fs::metadata(socket_path(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    #[test]
    fn other_versions_are_refused_and_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let listener = Listener::bind(dir.path(), Writes::default()).unwrap();
        let server = serve(listener, vec![Ok(())]);
        let mut old = UnixStream::connect(socket_path(dir.path())).unwrap();
        old.write_all(b"{\"version\":99,\"request\":null}\n")
            .unwrap();
        let ack: Ack = serde_json::from_str(&read_line(&old).unwrap()).unwrap();
        assert!(ack.error.unwrap().contains("version 1, not 99"));
        assert_eq!(
            send(dir.path(), None, Writes::default()).unwrap(),
            Outcome::Accepted
        );
        assert_eq!(server.join().unwrap(), vec![None]);
    }
    #[test]
    fn nothing_running_is_reported_without_a_socket() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            send(dir.path(), Some(&request()), Writes::default()).unwrap(),
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
            send(dir.path(), Some(&request()), Writes::default()).unwrap(),
            Outcome::NotRunning
        );
        assert!(!socket_path(dir.path()).exists());
    }
    #[test]
    fn a_held_lock_without_a_listener_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let _store = store::Store::open(dir.path()).unwrap();
        assert!(store::in_use(dir.path()).unwrap());
        let error = send_within(
            dir.path(),
            Some(&request()),
            Writes::default(),
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert!(error.to_string().contains("not taking requests"), "{error}");
    }
    #[test]
    fn a_state_directory_too_long_for_a_socket_still_launches() {
        let dir = tempfile::tempdir().unwrap();
        let long = dir.path().join("d".repeat(120));
        assert_eq!(
            send(&long, Some(&request()), Writes::default()).unwrap(),
            Outcome::NotRunning
        );
        assert!(Listener::bind(&long, Writes::default()).is_err());
    }
    #[test]
    fn binding_replaces_a_stale_socket() {
        let dir = tempfile::tempdir().unwrap();
        drop(UnixListener::bind(socket_path(dir.path())).unwrap());
        let listener = Listener::bind(dir.path(), Writes::default()).unwrap();
        let server = serve(listener, vec![Ok(())]);
        assert_eq!(
            send(dir.path(), None, Writes::default()).unwrap(),
            Outcome::Accepted
        );
        server.join().unwrap();
    }
    #[test]
    fn requests_needing_writes_the_window_lacks_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let granted = Writes {
            github: true,
            gitlab: false,
        };
        let listener = Listener::bind(dir.path(), granted).unwrap();
        let server = serve(listener, vec![Ok(())]);
        let gitlab = Writes {
            github: false,
            gitlab: true,
        };
        let Outcome::Refused(error) = send(dir.path(), Some(&request()), gitlab).unwrap() else {
            panic!("a request needing GitLab writes was taken");
        };
        assert!(error.contains("without --allow-gitlab-writes"), "{error}");
        assert_eq!(
            send(dir.path(), Some(&request()), granted).unwrap(),
            Outcome::Accepted
        );
        assert_eq!(server.join().unwrap(), vec![Some(request())]);
    }
    #[test]
    fn a_window_that_closes_without_answering_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let store = store::Store::open(dir.path()).unwrap();
        let listener = Listener::bind(dir.path(), Writes::default()).unwrap();
        // Takes the request, then closes like a window going away, lock and all.
        let closing = std::thread::spawn(move || {
            drop(listener.accept().unwrap());
            drop(store);
        });
        let mut launches = 0;
        let delivery = deliver(dir.path(), Some(&request()), Writes::default(), || {
            launches += 1;
            launch_window(dir.path(), Arc::default());
            Ok(())
        })
        .unwrap();
        closing.join().unwrap();
        assert_eq!((delivery, launches), (Delivery::Launched, 1));
    }
    #[test]
    fn a_window_that_never_listens_points_at_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let error = deliver_within(
            dir.path(),
            None,
            Writes::default(),
            || Ok(()),
            Duration::from_millis(100),
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert!(error.to_string().contains("diffz.log"), "{error}");
    }
    type Received = Arc<std::sync::Mutex<Vec<Option<OpenRequest>>>>;
    /// Starts a stand-in window on `dir` that serves requests for a second. Its own request
    /// arrives on its command line, so the caller records that one.
    fn launch_window(dir: &Path, received: Received) {
        let dir = dir.to_path_buf();
        let (ready, started) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _store =
                open_or_hand_off(&dir, None, Writes::default(), || store::Store::open(&dir))
                    .unwrap()
                    .unwrap();
            let listener = Listener::bind(&dir, Writes::default()).unwrap();
            ready.send(()).unwrap();
            let listener = Arc::new(listener);
            let accepting = listener.clone();
            std::thread::spawn(move || {
                while let Ok(mut incoming) = accepting.accept() {
                    received.lock().unwrap().push(incoming.request.take());
                    incoming.reply(Ok(()));
                }
            });
            std::thread::sleep(Duration::from_secs(1));
        });
        started.recv().unwrap();
    }
    #[test]
    fn simultaneous_cold_starts_launch_one_window_that_gets_both_requests() {
        let dir = tempfile::tempdir().unwrap();
        let received = Received::default();
        let launches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let fixtures = ["F01", "F03"].map(|id| OpenRequest::Fixture(id.into()));
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let invocations: Vec<_> = fixtures
            .clone()
            .into_iter()
            .map(|request| {
                let (dir, received, launches, barrier) = (
                    dir.path().to_path_buf(),
                    received.clone(),
                    launches.clone(),
                    barrier.clone(),
                );
                std::thread::spawn(move || {
                    barrier.wait();
                    deliver(&dir, Some(&request), Writes::default(), || {
                        launches.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        received.lock().unwrap().push(Some(request.clone()));
                        launch_window(&dir, received.clone());
                        Ok(())
                    })
                    .unwrap()
                })
            })
            .collect();
        let mut deliveries: Vec<_> = invocations.into_iter().map(|t| t.join().unwrap()).collect();
        deliveries.sort_by_key(|d| *d == Delivery::HandedOff);
        assert_eq!(deliveries, [Delivery::Launched, Delivery::HandedOff]);
        assert_eq!(launches.load(std::sync::atomic::Ordering::SeqCst), 1);
        let mut received = received.lock().unwrap().clone();
        received.sort_by_key(|r| format!("{r:?}"));
        assert_eq!(received, fixtures.map(Some));
    }
    #[test]
    fn a_window_that_loses_the_lock_hands_its_request_over() {
        let dir = tempfile::tempdir().unwrap();
        let _store = store::Store::open(dir.path()).unwrap();
        let listener = Listener::bind(dir.path(), Writes::default()).unwrap();
        let server = serve(listener, vec![Ok(())]);
        let opened = open_or_hand_off(dir.path(), Some(&request()), Writes::default(), || {
            store::Store::open(dir.path())
        })
        .unwrap();
        assert!(opened.is_none());
        assert_eq!(server.join().unwrap(), vec![Some(request())]);
    }
    #[test]
    fn background_logs_are_private_and_bounded() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let mut log = log_file(dir.path()).unwrap();
        log.write_all(&vec![b'x'; LOG_LIMIT as usize + 1]).unwrap();
        let meta = log_file(dir.path()).unwrap().metadata().unwrap();
        assert_eq!((meta.len(), meta.permissions().mode() & 0o777), (0, 0o600));
    }
}
