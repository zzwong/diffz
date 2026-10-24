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
/// Application frame policy, including the terminating newline (not an XDG limit).
const LIMIT: u64 = 65_536;
/// Diffz admission policy, not a protocol-specified token length.
pub const ACTIVATION_TOKEN_LIMIT: usize = 4_096;

/// Admit opaque bytes unchanged. Never format rejected metadata in diagnostics.
pub fn admit_activation_token(token: String) -> Option<String> {
    (!token.is_empty() && token.len() <= ACTIVATION_TOKEN_LIMIT && !token.contains('\0'))
        .then_some(token)
}

fn optional_activation_token<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(token) => admit_activation_token(token),
        _ => None,
    })
}
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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_activation_token"
    )]
    activation_token: Option<String>,
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
    if !line.ends_with('\n') {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "incomplete or oversized handoff frame",
        ));
    }
    Ok(line)
}

/// Construct the only frame to be written. Dropping oversized metadata here is
/// not an IPC retry; the request itself is never truncated.
fn frame(
    request: Option<&OpenRequest>,
    writes: Writes,
    activation_token: Option<String>,
) -> Result<Vec<u8>> {
    let mut message = Message {
        version: VERSION,
        request: request.cloned(),
        writes,
        activation_token: activation_token.and_then(admit_activation_token),
    };
    loop {
        let mut bytes =
            serde_json::to_vec(&message).map_err(|_| "could not encode handoff request")?;
        bytes.push(b'\n');
        if bytes.len() <= LIMIT as usize {
            return Ok(bytes);
        }
        if message.activation_token.take().is_none() {
            return Err("handoff request exceeds the 65536-byte frame limit".into());
        }
    }
}

fn unreadable(error: &serde_json::Error) -> String {
    // serde's normal error text can quote rejected values, including token bytes.
    format!(
        "the running diffz could not read the request at line {} column {}",
        error.line(),
        error.column()
    )
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
pub fn send(
    dir: &Path,
    request: Option<&OpenRequest>,
    writes: Writes,
    activation_token: &mut Option<String>,
) -> Result<Outcome> {
    send_within(dir, request, writes, activation_token, STARTUP)
}
fn send_within(
    dir: &Path,
    request: Option<&OpenRequest>,
    writes: Writes,
    activation_token: &mut Option<String>,
    startup: Duration,
) -> Result<Outcome> {
    let path = socket_path(dir);
    let deadline = Instant::now() + startup;
    let stream = loop {
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
    exchange(
        &mut &stream,
        || read_line(&stream),
        request,
        writes,
        activation_token,
    )
}

/// The single write/Ack exchange, separated only so partial writes and missing
/// acknowledgements can be injected without pretending to test native focus.
fn exchange(
    writer: &mut impl Write,
    read_ack: impl FnOnce() -> std::io::Result<String>,
    request: Option<&OpenRequest>,
    writes: Writes,
    activation_token: &mut Option<String>,
) -> Result<Outcome> {
    // Once connected, consume before constructing the frame and before the first
    // write attempt. A partial write or lost Ack must never restore this token.
    let message = frame(request, writes, activation_token.take())?;
    let line = match writer.write_all(&message).and_then(|()| read_ack()) {
        Ok(line) if !line.is_empty() => line,
        Ok(_) => return Ok(Outcome::Unanswered),
        Err(e) if e.kind() == ErrorKind::InvalidData => return Ok(Outcome::Unanswered),
        Err(e) if unanswered(&e) => return Ok(Outcome::Unanswered),
        Err(e) => return Err(e.into()),
    };
    let ack: Ack =
        serde_json::from_str(&line).map_err(|_| "could not read handoff acknowledgement")?;
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
    activation_token: Option<String>,
    launch: impl FnOnce(Option<String>) -> Result<()>,
) -> Result<Delivery> {
    deliver_within(
        dir,
        request,
        writes,
        activation_token,
        launch,
        STARTUP,
        LAUNCH,
    )
}
fn deliver_within(
    dir: &Path,
    request: Option<&OpenRequest>,
    writes: Writes,
    activation_token: Option<String>,
    launch: impl FnOnce(Option<String>) -> Result<()>,
    startup: Duration,
    listen: Duration,
) -> Result<Delivery> {
    deliver_observed(
        dir,
        request,
        writes,
        activation_token,
        launch,
        (startup, listen),
        || {},
    )
}

// The observer is a no-op in production; tests synchronize the owner replacement
// at this real recovery boundary instead of sleeping or duplicating delivery logic.
fn deliver_observed(
    dir: &Path,
    request: Option<&OpenRequest>,
    writes: Writes,
    mut activation_token: Option<String>,
    launch: impl FnOnce(Option<String>) -> Result<()>,
    (startup, listen): (Duration, Duration),
    before_launch_lock: impl FnOnce(),
) -> Result<Delivery> {
    let outcome = send_within(dir, request, writes, &mut activation_token, startup)?;
    if let Some(delivery) = settle(outcome, dir, startup)? {
        return Ok(delivery);
    }
    before_launch_lock();
    store::private_dir(dir)?;
    // Released when this returns, after the new window takes requests.
    let lock = store::private_file(&dir.join("launch.lock"))?;
    lock.lock_exclusive()?;
    let outcome = send_within(dir, request, writes, &mut activation_token, startup)?;
    if let Some(delivery) = settle(outcome, dir, startup)? {
        return Ok(delivery);
    }
    launch(activation_token)?;
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
    activation_token: &mut Option<String>,
    open: impl Fn() -> Result<T>,
) -> Result<Option<T>> {
    let error = match open() {
        Ok(opened) => return Ok(Some(opened)),
        Err(e) => e,
    };
    match send(dir, request, writes, activation_token)? {
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
        self.accept_observed(|_| {})
    }
    fn accept_observed(&self, mut accepted: impl FnMut(Instant)) -> Result<Incoming> {
        loop {
            let (stream, _) = self.socket.accept()?;
            let accepted_at = Instant::now();
            accepted(accepted_at);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
            let line = read_line(&stream).unwrap_or_default();
            let mut incoming = Incoming {
                request: None,
                activation_token: None,
                accepted_at,
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
                            incoming.activation_token = message.activation_token;
                            return Ok(incoming);
                        }
                    },
                    Err(e) => unreadable(&e),
                },
                Err(e) => unreadable(&e),
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
    pub activation_token: Option<String>,
    /// Receiver-local accept time: includes reads and UI delays, not listen backlog.
    pub accepted_at: Instant,
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
    const TOKEN: &str = "synthetic opaque \n \" token ☃";

    #[test]
    fn wire_v1_accepts_old_bytes_and_old_decoders_ignore_metadata() {
        #[derive(Deserialize)]
        struct OldMessage {
            version: u32,
            request: Option<OpenRequest>,
            #[serde(default)]
            writes: Writes,
        }
        for request in [None, Some(OpenRequest::Patch("a path/review.patch".into()))] {
            let bytes = frame(request.as_ref(), Writes::default(), Some(TOKEN.into())).unwrap();
            let decoded: Message = serde_json::from_slice(&bytes).unwrap();
            assert!(decoded.activation_token.as_deref() == Some(TOKEN));
            assert_eq!(decoded.request, request);
            let old: OldMessage = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(old.version, VERSION);
            assert_eq!(old.request, request);
            assert_eq!(old.writes, Writes::default());
        }
        for old in [
            r#"{"version":1,"request":null}"#,
            r#"{"version":1,"request":null,"activation_token":null}"#,
        ] {
            let decoded: Message = serde_json::from_str(old).unwrap();
            assert!(decoded.activation_token.is_none());
        }
    }

    #[test]
    fn metadata_admission_is_byte_bounded_and_not_normalized() {
        for value in [
            serde_json::json!(""),
            serde_json::json!("a\0b"),
            serde_json::json!("x".repeat(4097)),
            serde_json::json!({"token": TOKEN}),
            serde_json::json!([TOKEN]),
            serde_json::json!(42),
            serde_json::Value::Null,
        ] {
            let bytes = serde_json::to_vec(
                &serde_json::json!({"version":1,"request":null,"activation_token":value}),
            )
            .unwrap();
            let decoded: Message = serde_json::from_slice(&bytes).unwrap();
            assert!(decoded.activation_token.is_none());
        }
        for token in [
            " ".to_owned(),
            TOKEN.to_owned(),
            "x".repeat(4096),
            "é".repeat(2048),
        ] {
            let bytes = frame(None, Writes::default(), Some(token.clone())).unwrap();
            let decoded: Message = serde_json::from_slice(&bytes).unwrap();
            assert!(decoded.activation_token.as_ref() == Some(&token));
        }
        let decoded: Message =
            serde_json::from_str(r#"{"version":1,"request":null,"activation_token":"\u2603"}"#)
                .unwrap();
        assert!(decoded.activation_token.as_deref() == Some("☃"));
    }

    #[test]
    fn exact_serialized_frame_limit_includes_newline_and_escaping() {
        let base = OpenRequest::Fixture(String::new());
        let overhead = frame(Some(&base), Writes::default(), None).unwrap().len();
        for size in [65_535, 65_536, 65_537] {
            let request = OpenRequest::Fixture("x".repeat(size - overhead));
            let result = frame(Some(&request), Writes::default(), None);
            if size <= LIMIT as usize {
                let bytes = result.unwrap();
                assert_eq!(bytes.len(), size);
                assert_eq!(bytes.last(), Some(&b'\n'));
                // Metadata-only overflow chooses one tokenless frame before any write.
                let with_token =
                    frame(Some(&request), Writes::default(), Some("\"".repeat(4096))).unwrap();
                assert!(with_token == bytes);
            } else {
                assert!(result.is_err());
            }
        }
        let token = "\"".repeat(4096);
        let bytes = frame(None, Writes::default(), Some(token)).unwrap();
        assert!(bytes.len() > 8192);
    }

    #[test]
    fn complete_limit_frame_is_valid_but_capped_prefix_is_not() {
        for (size, newline, valid) in [
            (65_535, true, true),
            (65_536, true, true),
            (65_537, true, false),
            (20, false, false),
        ] {
            let (mut writer, reader) = UnixStream::pair().unwrap();
            let task = std::thread::spawn(move || {
                let mut bytes = vec![b' '; size];
                if newline {
                    *bytes.last_mut().unwrap() = b'\n';
                }
                let _ = writer.write_all(&bytes);
            });
            assert_eq!(read_line(&reader).is_ok(), valid);
            drop(reader);
            task.join().unwrap();
        }
    }

    #[test]
    fn rejected_envelopes_do_not_echo_metadata_and_listener_keeps_serving() {
        let dir = tempfile::tempdir().unwrap();
        let listener = Listener::bind(dir.path(), Writes::default()).unwrap();
        let server = serve(listener, vec![Ok(())]);
        let malformed = format!(
            "{{\"version\":1,\"request\":{},\"activation_token\":{}}}\n",
            serde_json::to_string(TOKEN).unwrap(),
            serde_json::to_string(TOKEN).unwrap()
        );
        let nested = format!(
            "{{\"version\":1,\"request\":null,\"activation_token\":{}{}{} }}\n",
            "[".repeat(200),
            serde_json::to_string(TOKEN).unwrap(),
            "]".repeat(200)
        );
        for bytes in [malformed, nested, "{\"version\":1,\"request\":null}".into()] {
            let mut stream = UnixStream::connect(socket_path(dir.path())).unwrap();
            stream.write_all(bytes.as_bytes()).unwrap();
            stream.shutdown(std::net::Shutdown::Write).unwrap();
            let response = read_line(&stream).unwrap();
            assert!(!response.contains(TOKEN));
            assert!(!response.contains("synthetic opaque"));
            let ack: Ack = serde_json::from_str(&response).unwrap();
            assert!(ack.error.is_some());
        }
        assert_eq!(
            send(dir.path(), None, Writes::default(), &mut None).unwrap(),
            Outcome::Accepted
        );
        server.join().unwrap();
    }

    #[test]
    fn no_instance_keeps_ownership_and_a_successful_send_consumes_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut token = Some(TOKEN.into());
        assert_eq!(
            send(dir.path(), None, Writes::default(), &mut token).unwrap(),
            Outcome::NotRunning
        );
        assert!(token.as_deref() == Some(TOKEN));
        let listener = Listener::bind(dir.path(), Writes::default()).unwrap();
        let server = std::thread::spawn(move || {
            let incoming = listener.accept().unwrap();
            assert!(incoming.activation_token.as_deref() == Some(TOKEN));
            incoming.reply(Ok(()));
        });
        assert_eq!(
            send(dir.path(), None, Writes::default(), &mut token).unwrap(),
            Outcome::Accepted
        );
        assert!(token.is_none());
        server.join().unwrap();
    }

    #[test]
    fn accepted_at_precedes_read_completion() {
        let dir = tempfile::tempdir().unwrap();
        let listener = Listener::bind(dir.path(), Writes::default()).unwrap();
        let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let incoming = listener
                .accept_observed(|at| accepted_tx.send(at).unwrap())
                .unwrap();
            let accepted_at = incoming.accepted_at;
            incoming.reply(Ok(()));
            accepted_at
        });
        let mut stream = UnixStream::connect(socket_path(dir.path())).unwrap();
        stream.write_all(b"{\"version\":1,").unwrap();
        // Synchronize actual accept rather than assuming the receiver was scheduled.
        let accepted_at = accepted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let before_completion = Instant::now();
        stream.write_all(b"\"request\":null}\n").unwrap();
        assert!(accepted_at <= before_completion);
        assert!(read_line(&stream).is_ok());
        assert_eq!(server.join().unwrap(), accepted_at);
    }

    #[test]
    fn partial_writes_and_lost_ack_never_restore_or_retry_a_token() {
        struct FailingWriter {
            written: usize,
            partial: bool,
        }
        impl Write for FailingWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if self.partial && self.written == 0 {
                    self.written = bytes.len().min(7);
                    return Ok(self.written);
                }
                Err(std::io::Error::from(ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        for partial in [false, true] {
            let mut token = Some(TOKEN.into());
            let mut writer = FailingWriter {
                written: 0,
                partial,
            };
            assert_eq!(
                exchange(
                    &mut writer,
                    || panic!("must not read after failed write"),
                    None,
                    Writes::default(),
                    &mut token
                )
                .unwrap(),
                Outcome::Unanswered
            );
            assert!(token.is_none());
            assert_eq!(writer.written, if partial { 7 } else { 0 });
        }
        let mut token = Some(TOKEN.into());
        let mut written = Vec::new();
        assert_eq!(
            exchange(
                &mut written,
                || Err(std::io::Error::from(ErrorKind::TimedOut)),
                None,
                Writes::default(),
                &mut token
            )
            .unwrap(),
            Outcome::Unanswered
        );
        assert!(token.is_none());
        let decoded: Message = serde_json::from_slice(&written).unwrap();
        assert!(decoded.activation_token.as_deref() == Some(TOKEN));
        assert_eq!(written.iter().filter(|&&b| b == b'\n').count(), 1);
    }

    #[test]
    fn oversized_base_fails_before_any_write_and_refusal_consumes_token() {
        let mut bytes = Vec::new();
        let mut token = Some(TOKEN.into());
        assert!(
            exchange(
                &mut bytes,
                || panic!("oversize has no Ack"),
                Some(&OpenRequest::Fixture("x".repeat(65_536))),
                Writes::default(),
                &mut token
            )
            .is_err()
        );
        assert!(bytes.is_empty());
        assert!(token.is_none());
        let mut token = Some(TOKEN.into());
        assert_eq!(
            exchange(
                &mut bytes,
                || Ok("{\"version\":1,\"error\":\"refused\"}\n".into()),
                None,
                Writes::default(),
                &mut token
            )
            .unwrap(),
            Outcome::Refused("refused".into())
        );
        assert!(token.is_none());
    }

    #[test]
    fn lost_ack_replacement_launch_does_not_receive_attempted_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = store::Store::open(dir.path()).unwrap();
        let listener = Listener::bind(dir.path(), Writes::default()).unwrap();
        let server = std::thread::spawn(move || {
            let incoming = listener.accept().unwrap();
            assert!(incoming.activation_token.as_deref() == Some(TOKEN));
            drop(incoming);
            drop(listener);
            drop(store);
        });
        let result = deliver(
            dir.path(),
            None,
            Writes::default(),
            Some(TOKEN.into()),
            |token| {
                assert!(token.is_none());
                launch_window(dir.path(), Arc::default());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(result, Delivery::Launched);
        server.join().unwrap();
    }

    #[test]
    fn actual_delivery_second_probe_never_replays_a_lost_ack_token() {
        let dir = tempfile::tempdir().unwrap();
        let owner = store::Store::open(dir.path()).unwrap();
        let launch_lock = store::private_file(&dir.path().join("launch.lock")).unwrap();
        launch_lock.lock_exclusive().unwrap();
        let listener = Listener::bind(dir.path(), Writes::default()).unwrap();
        let closing = std::thread::spawn(move || {
            let incoming = listener.accept().unwrap();
            assert!(incoming.activation_token.as_deref() == Some(TOKEN));
            assert_eq!(incoming.request, Some(request()));
            drop(incoming);
            drop(listener);
            drop(owner);
        });
        let (settled, after_settle) = std::sync::mpsc::channel();
        let sender_dir = dir.path().to_path_buf();
        let sender = std::thread::spawn(move || {
            deliver_observed(
                &sender_dir,
                Some(&request()),
                Writes::default(),
                Some(TOKEN.into()),
                |_| panic!("replacement instance must receive the request"),
                (STARTUP, LAUNCH),
                || settled.send(()).unwrap(),
            )
            .unwrap()
        });
        // The actual first probe has returned Unanswered and settle has observed
        // the old writer lock released. Hold the launch lock until replacement is
        // listening, then let the real second probe run with the same owned slot.
        after_settle.recv_timeout(Duration::from_secs(5)).unwrap();
        closing.join().unwrap();
        let replacement_owner = store::Store::open(dir.path()).unwrap();
        let listener = Listener::bind(dir.path(), Writes::default()).unwrap();
        let replacement = std::thread::spawn(move || {
            let incoming = listener.accept().unwrap();
            assert!(incoming.activation_token.is_none());
            assert_eq!(incoming.request, Some(request()));
            incoming.reply(Ok(()));
            drop(replacement_owner);
        });
        FileExt::unlock(&launch_lock).unwrap();
        assert_eq!(sender.join().unwrap(), Delivery::HandedOff);
        replacement.join().unwrap();
        // Two request deliveries, only the first token-bearing; no launch/retry
        // follows the accepted replacement acknowledgement.
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
            send(dir.path(), Some(&request()), Writes::default(), &mut None).unwrap(),
            Outcome::Accepted
        );
        assert_eq!(
            send(dir.path(), Some(&request()), Writes::default(), &mut None).unwrap(),
            Outcome::Refused("unsaved changes".into())
        );
        assert_eq!(
            send(dir.path(), None, Writes::default(), &mut None).unwrap(),
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
            send(dir.path(), None, Writes::default(), &mut None).unwrap(),
            Outcome::Accepted
        );
        assert_eq!(server.join().unwrap(), vec![None]);
    }
    #[test]
    fn nothing_running_is_reported_without_a_socket() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            send(dir.path(), Some(&request()), Writes::default(), &mut None).unwrap(),
            Outcome::NotRunning
        );
    }
    #[test]
    fn stale_socket_is_removed_when_the_lock_is_free() {
        const FIXTURE: &str = "DIFFZ_TEST_STALE_SOCKET_FIXTURE";
        if let Some(path) = std::env::var_os(FIXTURE) {
            drop(UnixListener::bind(path).unwrap());
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        // Create the stale listener in a separate process. Other tests fork to
        // launch commands; their children must not inherit this listener and
        // briefly keep it alive after we drop it.
        let fixture = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "handoff::tests::stale_socket_is_removed_when_the_lock_is_free",
            ])
            .env(FIXTURE, socket_path(dir.path()))
            .output()
            .unwrap();
        assert!(fixture.status.success(), "{fixture:?}");
        std::fs::write(dir.path().join("writer.lock"), "").unwrap();
        assert!(socket_path(dir.path()).exists());
        assert_eq!(
            send(dir.path(), Some(&request()), Writes::default(), &mut None).unwrap(),
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
            &mut None,
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
            send(&long, Some(&request()), Writes::default(), &mut None).unwrap(),
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
            send(dir.path(), None, Writes::default(), &mut None).unwrap(),
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
        let Outcome::Refused(error) =
            send(dir.path(), Some(&request()), gitlab, &mut None).unwrap()
        else {
            panic!("a request needing GitLab writes was taken");
        };
        assert!(error.contains("without --allow-gitlab-writes"), "{error}");
        assert_eq!(
            send(dir.path(), Some(&request()), granted, &mut None).unwrap(),
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
        let delivery = deliver(
            dir.path(),
            Some(&request()),
            Writes::default(),
            None,
            |_| {
                launches += 1;
                launch_window(dir.path(), Arc::default());
                Ok(())
            },
        )
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
            None,
            |_| Ok(()),
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
        launch_window_with_tokens(dir, received, None);
    }
    fn launch_window_with_tokens(
        dir: &Path,
        received: Received,
        tokens: Option<Arc<std::sync::Mutex<Vec<String>>>>,
    ) {
        let dir = dir.to_path_buf();
        let (ready, started) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _store = open_or_hand_off(&dir, None, Writes::default(), &mut None, || {
                store::Store::open(&dir)
            })
            .unwrap()
            .unwrap();
            let listener = Listener::bind(&dir, Writes::default()).unwrap();
            ready.send(()).unwrap();
            let listener = Arc::new(listener);
            let accepting = listener.clone();
            std::thread::spawn(move || {
                while let Ok(mut incoming) = accepting.accept() {
                    received.lock().unwrap().push(incoming.request.take());
                    if let (Some(tokens), Some(token)) = (&tokens, incoming.activation_token.take())
                    {
                        tokens.lock().unwrap().push(token);
                    }
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
                    deliver(&dir, Some(&request), Writes::default(), None, |_| {
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
    fn simultaneous_cold_starts_deliver_each_owned_token_only_once() {
        let dir = tempfile::tempdir().unwrap();
        let tokens = Arc::new(std::sync::Mutex::new(Vec::new()));
        let launches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let senders: Vec<_> = ["synthetic-first", "synthetic-second"]
            .into_iter()
            .map(|token| {
                let (dir, tokens, launches, barrier) = (
                    dir.path().to_path_buf(),
                    tokens.clone(),
                    launches.clone(),
                    barrier.clone(),
                );
                std::thread::spawn(move || {
                    barrier.wait();
                    deliver(&dir, None, Writes::default(), Some(token.into()), |owned| {
                        launches.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        tokens.lock().unwrap().push(owned.unwrap());
                        launch_window_with_tokens(&dir, Arc::default(), Some(tokens.clone()));
                        Ok(())
                    })
                    .unwrap()
                })
            })
            .collect();
        for sender in senders {
            sender.join().unwrap();
        }
        assert_eq!(launches.load(std::sync::atomic::Ordering::SeqCst), 1);
        let mut actual = tokens.lock().unwrap();
        actual.sort();
        assert!(actual.as_slice() == ["synthetic-first", "synthetic-second"]);
    }

    #[test]
    fn foreground_lock_loser_moves_token_but_successful_open_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = store::Store::open(dir.path()).unwrap();
        let listener = Listener::bind(dir.path(), Writes::default()).unwrap();
        let server = std::thread::spawn(move || {
            let incoming = listener.accept().unwrap();
            assert!(incoming.activation_token.as_deref() == Some(TOKEN));
            incoming.reply(Ok(()));
        });
        let mut token = Some(TOKEN.into());
        assert!(
            open_or_hand_off(dir.path(), None, Writes::default(), &mut token, || {
                store::Store::open(dir.path())
            })
            .unwrap()
            .is_none()
        );
        assert!(token.is_none());
        server.join().unwrap();
        drop(store);
        let mut token = Some(TOKEN.into());
        assert!(
            open_or_hand_off(dir.path(), None, Writes::default(), &mut token, || {
                store::Store::open(dir.path())
            })
            .unwrap()
            .is_some()
        );
        assert!(token.as_deref() == Some(TOKEN));
    }

    #[test]
    fn a_window_that_loses_the_lock_hands_its_request_over() {
        let dir = tempfile::tempdir().unwrap();
        let _store = store::Store::open(dir.path()).unwrap();
        let listener = Listener::bind(dir.path(), Writes::default()).unwrap();
        let server = serve(listener, vec![Ok(())]);
        let opened = open_or_hand_off(
            dir.path(),
            Some(&request()),
            Writes::default(),
            &mut None,
            || store::Store::open(dir.path()),
        )
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
