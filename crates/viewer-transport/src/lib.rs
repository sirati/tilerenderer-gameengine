#![forbid(unsafe_code)]

//! Local process transport. Scene acknowledgment means the display consumed
//! the batch, while CPU art completion and GPU upload have separate counters.

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use civ3r_sync::Mailbox;

pub const VERSION: u32 = 1;
pub const MAX_FRAME: usize = 64 * 1024 * 1024;
const MAX_QUEUED: usize = MAX_FRAME;
const MAX_INPUT: usize = 4096;
const MAX_UPDATES: usize = 4096;
const MAX_LOG: usize = 128;
const TIMEOUT: Duration = Duration::from_secs(10);

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Tokens must contain at least 128 bits from the launcher's random source.
pub fn valid_token(token: &str) -> bool {
    (32..=128).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Hello {
        version: u32,
        scene_version: u32,
        token: String,
        fullscreen: bool,
    },
    Ready {
        max_texture: u32,
    },
    Assets(Vec<u8>),
    Scene {
        serial: u64,
        bytes: Vec<u8>,
    },
    SceneTaken(u64),
    Input(Vec<u8>),
    Status {
        processed: u64,
        uploaded: u64,
    },
    Error(String),
    Quit,
}

impl Message {
    fn encoded_len(&self) -> io::Result<usize> {
        let size = match self {
            Self::Hello { token, .. } => {
                if !valid_token(token) {
                    return Err(invalid("invalid session token"));
                }
                10 + token.len()
            }
            Self::Ready { .. } => 5,
            Self::Assets(bytes) | Self::Input(bytes) => bytes.len().saturating_add(1),
            Self::Scene { bytes, .. } => bytes.len().saturating_add(9),
            Self::SceneTaken(_) => 9,
            Self::Status { .. } => 17,
            Self::Error(message) => message.len().min(16 * 1024) + 1,
            Self::Quit => 1,
        };
        if size > MAX_FRAME {
            return Err(invalid("viewer frame exceeds 64 MiB"));
        }
        Ok(size)
    }

    fn encode(&self) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(self.encoded_len()?);
        match self {
            Self::Hello {
                version,
                scene_version,
                token,
                fullscreen,
            } => {
                if !valid_token(token) {
                    return Err(invalid("invalid session token"));
                }
                out.push(0);
                out.extend(version.to_le_bytes());
                out.extend(scene_version.to_le_bytes());
                out.push(u8::from(*fullscreen));
                out.extend(token.as_bytes());
            }
            Self::Ready { max_texture } => {
                out.push(1);
                out.extend(max_texture.to_le_bytes());
            }
            Self::Assets(bytes) => {
                out.push(2);
                out.extend(bytes);
            }
            Self::Scene { serial, bytes } => {
                out.push(3);
                out.extend(serial.to_le_bytes());
                out.extend(bytes);
            }
            Self::SceneTaken(serial) => {
                out.push(4);
                out.extend(serial.to_le_bytes());
            }
            Self::Input(bytes) => {
                out.push(5);
                out.extend(bytes);
            }
            Self::Status {
                processed,
                uploaded,
            } => {
                out.push(6);
                out.extend(processed.to_le_bytes());
                out.extend(uploaded.to_le_bytes());
            }
            Self::Error(message) => {
                out.push(7);
                out.extend(message.as_bytes().iter().copied().take(16 * 1024));
            }
            Self::Quit => out.push(8),
        }
        if out.len() > MAX_FRAME {
            return Err(invalid("viewer frame exceeds 64 MiB"));
        }
        Ok(out)
    }

    fn decode(bytes: Vec<u8>) -> io::Result<Self> {
        let Some((&tag, body)) = bytes.split_first() else {
            return Err(invalid("empty viewer frame"));
        };
        let u32_at = |at: usize| u32::from_le_bytes(body[at..at + 4].try_into().unwrap());
        let u64_at = |at: usize| u64::from_le_bytes(body[at..at + 8].try_into().unwrap());
        match tag {
            0 if (41..=137).contains(&body.len()) && body[8] <= 1 => {
                let token = std::str::from_utf8(&body[9..])
                    .map_err(|_| invalid("invalid session token"))?
                    .to_owned();
                if !valid_token(&token) {
                    return Err(invalid("invalid session token"));
                }
                Ok(Self::Hello {
                    version: u32_at(0),
                    scene_version: u32_at(4),
                    token,
                    fullscreen: body[8] == 1,
                })
            }
            1 if body.len() == 4 => Ok(Self::Ready {
                max_texture: u32_at(0),
            }),
            2 => Ok(Self::Assets(body.to_vec())),
            3 if body.len() >= 8 => Ok(Self::Scene {
                serial: u64_at(0),
                bytes: body[8..].to_vec(),
            }),
            4 if body.len() == 8 => Ok(Self::SceneTaken(u64_at(0))),
            5 => Ok(Self::Input(body.to_vec())),
            6 if body.len() == 16 => Ok(Self::Status {
                processed: u64_at(0),
                uploaded: u64_at(8),
            }),
            7 if body.len() <= 16 * 1024 => Ok(Self::Error(
                String::from_utf8(body.to_vec()).map_err(|_| invalid("invalid error text"))?,
            )),
            8 if body.is_empty() => Ok(Self::Quit),
            _ => Err(invalid(format!("invalid viewer message {tag}"))),
        }
    }
}

pub fn write_message(writer: &mut impl Write, message: &Message) -> io::Result<()> {
    let bytes = message.encode()?;
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&bytes)
}

pub fn read_message(reader: &mut impl Read) -> io::Result<Message> {
    let mut size = [0; 4];
    reader.read_exact(&mut size)?;
    let size = u32::from_le_bytes(size) as usize;
    if !(1..=MAX_FRAME).contains(&size) {
        return Err(invalid("viewer frame length outside 1..=64 MiB"));
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    Message::decode(bytes)
}

/// Authenticate before any game data is accepted. The listener must bind to
/// loopback; this check also prevents an accidentally public listener being used.
pub fn accept(
    mut stream: TcpStream,
    token: &str,
    max_texture: u32,
) -> io::Result<(TcpStream, bool)> {
    if !stream.peer_addr()?.ip().is_loopback()
        || !stream.local_addr()?.ip().is_loopback()
        || !valid_token(token)
    {
        return Err(invalid(
            "viewer sessions require loopback and a random token",
        ));
    }
    // Darwin inherits O_NONBLOCK from a nonblocking listener. This
    // connection is consumed by blocking framed readers after accept.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.set_nodelay(true)?;
    let fullscreen = match read_message(&mut stream)? {
        Message::Hello {
            version,
            scene_version,
            token: supplied,
            fullscreen,
        } if version == VERSION
            && scene_version == scene::wire::VERSION
            && equal_token(supplied.as_bytes(), token.as_bytes()) =>
        {
            fullscreen
        }
        _ => {
            let _ = write_message(&mut stream, &Message::Quit);
            return Err(invalid("viewer authentication or version mismatch"));
        }
    };
    if max_texture == 0 {
        return Err(invalid("viewer texture limit is zero"));
    }
    write_message(&mut stream, &Message::Ready { max_texture })?;
    stream.set_read_timeout(None)?;
    Ok((stream, fullscreen))
}

fn equal_token(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for (i, &byte) in b.iter().enumerate() {
        diff |= usize::from(byte ^ a.get(i).copied().unwrap_or(0));
    }
    diff == 0
}

/// Limit outer updates before the wire decoder reserves its vector. Large
/// worlds and cell deltas remain inside their individual update messages.
pub fn decode_updates_bounded(bytes: &[u8]) -> io::Result<Vec<scene::Update>> {
    let count = bytes
        .get(4..8)
        .ok_or_else(|| invalid("truncated viewer scene header"))?;
    if u32::from_le_bytes(count.try_into().unwrap()) as usize > MAX_UPDATES {
        return Err(invalid("viewer scene update count exceeds 4096"));
    }
    scene::wire::decode_updates(bytes).map_err(|e| invalid(e.to_string()))
}

fn decode_input_bounded(bytes: &[u8]) -> io::Result<Vec<scene::Input>> {
    let count = bytes
        .get(4..8)
        .ok_or_else(|| invalid("truncated viewer input header"))?;
    let count = u32::from_le_bytes(count.try_into().unwrap()) as usize;
    // The scene decoder reserves its vector from this count. Enforce the
    // transport bound before decoding, including malformed input batches.
    if count > MAX_INPUT {
        return Err(invalid("viewer input backlog exhausted"));
    }
    scene::wire::decode_input(bytes).map_err(|e| invalid(e.to_string()))
}

struct Shared {
    disconnected: AtomicBool,
    quit_taken: AtomicBool,
    scene: AtomicU64,
    next_scene: AtomicU64,
    processed: AtomicU64,
    uploaded: AtomicU64,
    queued: AtomicUsize,
    /// Decoded viewer input; the reader thread is its only producer.
    input: Mailbox<scene::Input>,
    /// Rung after input arrived (see [`Client::wake_on_input`]).
    input_wake: std::sync::OnceLock<&'static civ3r_sync::Wake>,
    /// Diagnostics, about the newest `MAX_LOG` kept.
    log: Mailbox<String>,
    /// Set by the first `fail`, which alone records its error.
    failed: AtomicBool,
    /// Messages for the writer thread, at most `MAX_OUTBOUND` of them. The
    /// writer parks rather than waiting in a channel, so the game's thread
    /// never touches a channel's waiter list (a lock) when it sends.
    outgoing: Mailbox<Outbound>,
    outbound: AtomicUsize,
    writer: std::sync::OnceLock<std::thread::Thread>,
    shutdown: TcpStream,
}

const MAX_OUTBOUND: usize = 8;

impl Shared {
    fn fail(&self, error: impl ToString) {
        if self.failed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.log(error.to_string());
        // Publish the error before readers observe disconnection and restore
        // the game, which may immediately drain the final diagnostics.
        self.disconnected.store(true, Ordering::Release);
        let _ = self.shutdown.shutdown(Shutdown::Both);
        self.wake_writer();
    }
    fn wake_writer(&self) {
        if let Some(writer) = self.writer.get() {
            writer.unpark();
        }
    }
    fn log(&self, line: String) {
        // Concurrent loggers may each drop a line; the bound only keeps a
        // game that never drains from growing the queue without limit.
        while self.log.len() >= MAX_LOG && self.log.try_recv().is_some() {}
        self.log.send(line);
    }
}

struct Outbound {
    message: Message,
    bytes: usize,
}

/// The game thread never waits for rendering, asset compilation, or socket
/// writes after connect. A slow display keeps one scene outstanding.
pub struct Client {
    shared: Arc<Shared>,
    max_texture: u32,
}

impl Client {
    pub fn connect(address: SocketAddr, token: &str, fullscreen: bool) -> io::Result<Self> {
        if !address.ip().is_loopback() || !valid_token(token) {
            return Err(invalid("viewer address or token is invalid"));
        }
        let mut stream = TcpStream::connect_timeout(&address, TIMEOUT)?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        write_message(
            &mut stream,
            &Message::Hello {
                version: VERSION,
                scene_version: scene::wire::VERSION,
                token: token.into(),
                fullscreen,
            },
        )?;
        let max_texture = match read_message(&mut stream)? {
            Message::Ready { max_texture } if max_texture > 0 => max_texture,
            _ => return Err(invalid("native viewer did not acknowledge the session")),
        };
        stream.set_read_timeout(None)?;
        let shared = Arc::new(Shared {
            disconnected: AtomicBool::new(false),
            quit_taken: AtomicBool::new(false),
            scene: AtomicU64::new(0),
            next_scene: AtomicU64::new(1),
            processed: AtomicU64::new(0),
            uploaded: AtomicU64::new(0),
            queued: AtomicUsize::new(0),
            input: Mailbox::new(),
            input_wake: std::sync::OnceLock::new(),
            log: Mailbox::new(),
            failed: AtomicBool::new(false),
            outgoing: Mailbox::new(),
            outbound: AtomicUsize::new(0),
            writer: std::sync::OnceLock::new(),
            shutdown: stream.try_clone()?,
        });
        let mut writer = stream.try_clone()?;
        let send_shared = shared.clone();
        let sender = std::thread::Builder::new()
            .name("viewer-send".into())
            .spawn(move || {
                loop {
                    let Some(out) = send_shared.outgoing.try_recv() else {
                        if send_shared.disconnected.load(Ordering::Acquire) {
                            break;
                        }
                        // Unparked by each message and by `fail`. (Bounded:
                        // under Wine/FEX on macOS a wake-up was seen lost,
                        // which would hold every later message back.)
                        std::thread::park_timeout(std::time::Duration::from_millis(10));
                        continue;
                    };
                    send_shared.outbound.fetch_sub(1, Ordering::AcqRel);
                    let result = write_message(&mut writer, &out.message);
                    send_shared.queued.fetch_sub(out.bytes, Ordering::AcqRel);
                    if let Err(e) = result {
                        send_shared.fail(e);
                        break;
                    }
                    if send_shared.disconnected.load(Ordering::Acquire) {
                        break;
                    }
                }
            })?;
        let _ = shared.writer.set(sender.thread().clone());
        let read_shared = shared.clone();
        let started = std::thread::Builder::new()
            .name("viewer-input".into())
            .spawn(move || {
                loop {
                    let result = read_message(&mut stream).and_then(|m| match m {
                        Message::SceneTaken(serial) => read_shared
                            .scene
                            .compare_exchange(serial, 0, Ordering::AcqRel, Ordering::Acquire)
                            .map(|_| ())
                            .map_err(|_| invalid("unexpected viewer scene acknowledgment")),
                        Message::Status {
                            processed,
                            uploaded,
                        } => {
                            read_shared
                                .processed
                                .fetch_max(processed, Ordering::Release);
                            read_shared.uploaded.fetch_max(uploaded, Ordering::Release);
                            Ok(())
                        }
                        Message::Input(bytes) => {
                            let inputs = decode_input_bounded(&bytes)?;
                            // This thread is the only sender, so the length
                            // can only shrink while it checks.
                            let queue = &read_shared.input;
                            if queue.len().saturating_add(inputs.len()) > MAX_INPUT {
                                return Err(invalid("viewer input backlog exhausted"));
                            }
                            inputs.into_iter().for_each(|i| queue.send(i));
                            if let Some(wake) = read_shared.input_wake.get() {
                                wake.ring();
                            }
                            Ok(())
                        }
                        Message::Error(line) => {
                            read_shared.log(line);
                            Ok(())
                        }
                        Message::Quit => Err(io::Error::new(
                            io::ErrorKind::ConnectionAborted,
                            "native viewer stopped",
                        )),
                        _ => Err(invalid("unexpected message from native viewer")),
                    });
                    if let Err(e) = result {
                        read_shared.fail(e);
                        break;
                    }
                }
            });
        if let Err(e) = started {
            shared.fail(&e);
            return Err(e);
        }
        Ok(Self {
            shared,
            max_texture,
        })
    }

    fn enqueue(&self, message: Message) -> io::Result<()> {
        let result = (|| {
            if self.shared.disconnected.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "native viewer disconnected",
                ));
            }
            let size = message.encoded_len()?;
            let mut queued = self.shared.queued.load(Ordering::Acquire);
            loop {
                let next = queued
                    .checked_add(size)
                    .filter(|&n| n <= MAX_QUEUED)
                    .ok_or_else(|| invalid("viewer output backlog exceeds 64 MiB"))?;
                match self.shared.queued.compare_exchange_weak(
                    queued,
                    next,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => break,
                    Err(current) => queued = current,
                }
            }
            let room = self.shared.outbound.try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_OUTBOUND).then_some(n + 1)
            });
            if room.is_err() {
                self.shared.queued.fetch_sub(size, Ordering::AcqRel);
                return Err(invalid("sending on a full channel"));
            }
            self.shared.outgoing.send(Outbound {
                message,
                bytes: size,
            });
            self.shared.wake_writer();
            Ok(())
        })();
        if let Err(e) = &result {
            self.shared.fail(e);
        }
        result
    }

    pub fn max_texture(&self) -> Option<u32> {
        Some(self.max_texture)
    }
    pub fn status(&self) -> u64 {
        self.shared.processed.load(Ordering::Acquire)
    }
    pub fn uploaded_generation(&self) -> u64 {
        self.shared.uploaded.load(Ordering::Acquire)
    }
    pub fn busy(&self) -> bool {
        self.shared.scene.load(Ordering::Acquire) != 0
            || self.shared.disconnected.load(Ordering::Acquire)
    }
    pub fn set_assets(&self, assets: &scene::wire::Assets) -> io::Result<()> {
        self.enqueue(Message::Assets(scene::wire::encode_assets(assets)))
    }

    pub fn publish(&self, updates: &[scene::Update]) -> (bool, usize) {
        if updates.len() > MAX_UPDATES {
            self.shared.fail("viewer scene update count exceeds 4096");
            return (false, 0);
        }
        if updates.is_empty() {
            return (!self.shared.disconnected.load(Ordering::Acquire), 0);
        }
        if self.busy() {
            return (false, 0);
        }
        let serial = self.shared.next_scene.fetch_add(1, Ordering::Relaxed);
        if self
            .shared
            .scene
            .compare_exchange(0, serial, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return (false, 0);
        }
        let bytes = scene::wire::encode_updates(updates);
        let size = bytes.len();
        (self.enqueue(Message::Scene { serial, bytes }).is_ok(), size)
    }

    /// Rings `wake` whenever input arrives (the first one set stays).
    pub fn wake_on_input(&self, wake: &'static civ3r_sync::Wake) {
        let _ = self.shared.input_wake.set(wake);
    }

    pub fn take_input(&self) -> Vec<scene::Input> {
        let mut input: Vec<_> = self.shared.input.drain().collect();
        input.retain(|event| {
            !matches!(event, scene::Input::Quit)
                || !self.shared.quit_taken.swap(true, Ordering::AcqRel)
        });
        if self.shared.disconnected.load(Ordering::Acquire)
            && !self.shared.quit_taken.swap(true, Ordering::AcqRel)
        {
            input.push(scene::Input::Quit);
        }
        input
    }
    pub fn take_log(&self) -> Vec<String> {
        self.shared.log.drain().collect()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.shared.fail("game renderer connection closed");
        // (After an earlier failure `fail` returned at once.)
        self.shared.wake_writer();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;
    const TOKEN: &str = "0123456789abcdef0123456789abcdef";
    fn pair(run: impl FnOnce(TcpStream) + Send + 'static) -> (Client, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let (stream, fullscreen) = accept(stream, TOKEN, 16384).unwrap();
            assert!(fullscreen);
            run(stream);
        });
        (Client::connect(addr, TOKEN, true).unwrap(), server)
    }
    #[test]
    fn frame_round_trips_and_rejects_oversize_before_allocating() {
        let messages = [
            Message::Hello {
                version: VERSION,
                scene_version: scene::wire::VERSION,
                token: TOKEN.into(),
                fullscreen: false,
            },
            Message::Ready { max_texture: 8192 },
            Message::Assets(vec![1, 2]),
            Message::Scene {
                serial: 91,
                bytes: vec![3, 4],
            },
            Message::SceneTaken(91),
            Message::Input(vec![5]),
            Message::Status {
                processed: 7,
                uploaded: 6,
            },
            Message::Error("failed art".into()),
            Message::Quit,
        ];
        for message in messages {
            let mut bytes = Vec::new();
            write_message(&mut bytes, &message).unwrap();
            assert_eq!(read_message(&mut &bytes[..]).unwrap(), message);
        }
        assert!(read_message(&mut &(MAX_FRAME as u32 + 1).to_le_bytes()[..]).is_err());
        assert!(read_message(&mut &0u32.to_le_bytes()[..]).is_err());
        assert!(read_message(&mut &[5, 0, 0, 0, 3, 1][..]).is_err());
    }
    #[test]
    fn auth_rejects_wrong_token_and_protocol() {
        for (token, version, scene_version) in [
            (
                "ffffffffffffffffffffffffffffffff",
                VERSION,
                scene::wire::VERSION,
            ),
            (TOKEN, VERSION + 1, scene::wire::VERSION),
            (TOKEN, VERSION, scene::wire::VERSION + 1),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let thread = std::thread::spawn(move || {
                assert!(accept(listener.accept().unwrap().0, TOKEN, 8192).is_err())
            });
            let mut stream = TcpStream::connect(addr).unwrap();
            write_message(
                &mut stream,
                &Message::Hello {
                    version,
                    scene_version,
                    token: token.into(),
                    fullscreen: false,
                },
            )
            .unwrap();
            assert_eq!(read_message(&mut stream).unwrap(), Message::Quit);
            thread.join().unwrap();
        }
    }
    #[test]
    fn scene_backpressure_and_serialization_preserve_acks_and_art_counters() {
        let (received_tx, received) = mpsc::channel();
        let (resume_tx, resume) = mpsc::channel();
        let (client, server) = pair(move |mut stream| {
            let Message::Assets(bytes) = read_message(&mut stream).unwrap() else {
                panic!()
            };
            assert_eq!(scene::wire::decode_assets(&bytes).unwrap().generation, 9);
            let Message::Scene { serial, bytes } = read_message(&mut stream).unwrap() else {
                panic!()
            };
            assert_eq!(
                scene::wire::decode_updates(&bytes).unwrap(),
                vec![scene::Update::Tick(37)]
            );
            received_tx.send(()).unwrap();
            resume.recv().unwrap();
            write_message(&mut stream, &Message::SceneTaken(serial)).unwrap();
            write_message(
                &mut stream,
                &Message::Status {
                    processed: 9,
                    uploaded: 8,
                },
            )
            .unwrap();
            write_message(
                &mut stream,
                &Message::Input(scene::wire::encode_input(&[scene::Input::Quit])),
            )
            .unwrap();
            // Keep socket open until the test has observed the status.
            resume.recv().unwrap();
        });
        assert_eq!(client.max_texture(), Some(16384));
        client
            .set_assets(&scene::wire::Assets {
                generation: 9,
                ..Default::default()
            })
            .unwrap();
        assert!(client.publish(&[scene::Update::Tick(37)]).0);
        received.recv_timeout(TIMEOUT).unwrap();
        assert!(client.busy());
        assert!(!client.publish(&[scene::Update::Tick(38)]).0);
        resume_tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + TIMEOUT;
        while client.uploaded_generation() != 8 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(client.status(), 9);
        assert!(!client.busy());
        resume_tx.send(()).unwrap();
        server.join().unwrap();
    }
    #[test]
    fn disconnect_and_malformed_input_deliver_quit_once() {
        for malformed in [false, true] {
            let (client, server) = pair(move |mut stream| {
                if malformed {
                    write_message(&mut stream, &Message::Input(vec![0])).unwrap();
                }
            });
            server.join().unwrap();
            let deadline = std::time::Instant::now() + TIMEOUT;
            loop {
                if client
                    .take_input()
                    .iter()
                    .any(|i| matches!(i, scene::Input::Quit))
                {
                    break;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            assert!(client.take_input().is_empty());
        }
    }
    #[test]
    fn invalid_endpoint_and_weak_tokens_are_rejected() {
        assert!(Client::connect("192.0.2.1:1234".parse().unwrap(), TOKEN, false).is_err());
        assert!(!valid_token("secret"));
        assert!(!valid_token("0123456789abcdef0123456789abcdeg"));
    }
    #[test]
    fn explicit_quit_and_socket_close_deliver_quit_exactly_once() {
        let (client, server) = pair(move |mut stream| {
            write_message(
                &mut stream,
                &Message::Input(scene::wire::encode_input(&[
                    scene::Input::Quit,
                    scene::Input::Quit,
                ])),
            )
            .unwrap();
            write_message(&mut stream, &Message::Quit).unwrap();
        });
        server.join().unwrap();
        let deadline = std::time::Instant::now() + TIMEOUT;
        while !client.shared.disconnected.load(Ordering::Acquire) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(
            client
                .take_input()
                .iter()
                .filter(|i| matches!(i, scene::Input::Quit))
                .count(),
            1
        );
        assert!(client.take_input().is_empty());
    }

    #[test]
    fn input_backlog_overflow_disconnects_instead_of_dropping_button_events() {
        let (client, server) = pair(move |mut stream| {
            write_message(
                &mut stream,
                &Message::Input(scene::wire::encode_input(&vec![
                    scene::Input::Quit;
                    MAX_INPUT + 1
                ])),
            )
            .unwrap();
        });
        server.join().unwrap();
        let deadline = std::time::Instant::now() + TIMEOUT;
        while !client.shared.disconnected.load(Ordering::Acquire) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(client.take_log().iter().any(|l| l.contains("backlog")));
        assert_eq!(client.take_input(), vec![scene::Input::Quit]);
    }
    #[test]
    fn excessive_input_count_is_rejected_before_wire_allocation() {
        let mut bytes = scene::wire::VERSION.to_le_bytes().to_vec();
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(
            decode_input_bounded(&bytes)
                .unwrap_err()
                .to_string()
                .contains("backlog")
        );
    }
    #[test]
    fn scene_outer_count_is_bounded_while_large_cell_deltas_are_valid() {
        let mut bytes = scene::wire::VERSION.to_le_bytes().to_vec();
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(
            decode_updates_bounded(&bytes)
                .unwrap_err()
                .to_string()
                .contains("count")
        );
        let updates = vec![scene::Update::Cells(vec![(0, 0, 1); MAX_UPDATES + 1])];
        assert_eq!(
            decode_updates_bounded(&scene::wire::encode_updates(&updates)).unwrap(),
            updates
        );
    }
    #[test]
    fn nonblocking_listener_connections_wait_for_delayed_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let (ready_tx, ready) = mpsc::channel();
        let (result_tx, result) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + TIMEOUT;
            let stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < deadline);
                        std::thread::yield_now();
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            // Make Darwin's inherited state deterministic on every platform.
            stream.set_nonblocking(true).unwrap();
            ready_tx.send(()).unwrap();
            result_tx.send(accept(stream, TOKEN, 8192)).unwrap();
        });
        let mut stream = TcpStream::connect(address).unwrap();
        ready.recv_timeout(TIMEOUT).unwrap();
        assert!(
            matches!(
                result.recv_timeout(Duration::from_millis(30)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "authentication must wait for the delayed Hello, not fail with WouldBlock"
        );
        write_message(
            &mut stream,
            &Message::Hello {
                version: VERSION,
                scene_version: scene::wire::VERSION,
                token: TOKEN.into(),
                fullscreen: false,
            },
        )
        .unwrap();
        assert_eq!(
            read_message(&mut stream).unwrap(),
            Message::Ready { max_texture: 8192 }
        );
        assert!(!result.recv_timeout(TIMEOUT).unwrap().unwrap().1);
        thread.join().unwrap();
    }

    #[test]
    fn concurrent_failures_record_one_error_and_logs_stay_bounded() {
        let (client, server) = pair(|mut stream| {
            for i in 0..MAX_LOG + 10 {
                write_message(&mut stream, &Message::Error(format!("line {i}"))).unwrap();
            }
            write_message(&mut stream, &Message::Quit).unwrap();
        });
        server.join().unwrap();
        while !client.shared.disconnected.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        std::thread::scope(|s| {
            for i in 0..8 {
                let shared = &client.shared;
                s.spawn(move || shared.fail(format!("late {i}")));
            }
        });
        let log = client.take_log();
        assert_eq!(log.len(), MAX_LOG);
        assert!(log.last().unwrap().contains("stopped"));
        assert!(!log.iter().any(|l| l.starts_with("late")));
    }
}
