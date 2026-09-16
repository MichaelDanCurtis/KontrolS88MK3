//! Output sinks. `CaptureSink` makes the whole pipeline testable without
//! hardware; `MidirSink` talks to a real port.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MidiError {
    #[error("could not initialise midi backend: {0}")]
    Init(String),
    #[error("no midi output port matching {0:?}; available: {1:?}")]
    NoMatchingPort(String, Vec<String>),
    #[error("midi output port match {0:?} is ambiguous; matches: {1:?}")]
    AmbiguousPort(String, Vec<String>),
    #[error("could not connect to midi port: {0}")]
    Connect(String),
    #[error("could not send midi bytes: {0}")]
    Send(String),
}

pub trait MidiSink {
    fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError>;
}

pub trait MidiSinkFactory {
    fn open(&mut self) -> Result<Box<dyn MidiSink>, MidiError>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MidiConnectionStats {
    pub connected: bool,
    pub connect_attempts: u64,
    pub reconnects: u64,
    pub connect_errors: u64,
    pub send_errors: u64,
    pub dropped_messages: u64,
    pub sent_messages: u64,
}

#[derive(Debug, Default)]
struct AtomicConnectionStats {
    connected: AtomicBool,
    connect_attempts: AtomicU64,
    reconnects: AtomicU64,
    connect_errors: AtomicU64,
    send_errors: AtomicU64,
    dropped_messages: AtomicU64,
    sent_messages: AtomicU64,
}

#[derive(Debug, Clone, Default)]
pub struct MidiConnectionStatus(Arc<AtomicConnectionStats>);

impl MidiConnectionStatus {
    pub fn snapshot(&self) -> MidiConnectionStats {
        let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
        MidiConnectionStats {
            connected: self.0.connected.load(Ordering::Relaxed),
            connect_attempts: load(&self.0.connect_attempts),
            reconnects: load(&self.0.reconnects),
            connect_errors: load(&self.0.connect_errors),
            send_errors: load(&self.0.send_errors),
            dropped_messages: load(&self.0.dropped_messages),
            sent_messages: load(&self.0.sent_messages),
        }
    }
}

/// Keeps a live runtime running across output disconnects. A failed message is
/// counted and dropped; later sends retry the same stable factory on a bounded
/// cadence. Reconnection never blocks the deterministic engine tick path with
/// sleeps, and the last status remains observable through [`MidiConnectionStatus`].
pub struct ReconnectingSink<F> {
    factory: F,
    connection: Option<Box<dyn MidiSink>>,
    reconnect_interval: Duration,
    next_attempt: Instant,
    status: MidiConnectionStatus,
}

impl<F: MidiSinkFactory> ReconnectingSink<F> {
    pub fn connect(mut factory: F, reconnect_interval: Duration) -> Result<Self, MidiError> {
        let status = MidiConnectionStatus::default();
        status.0.connect_attempts.fetch_add(1, Ordering::Relaxed);
        let connection = factory.open()?;
        status.0.connected.store(true, Ordering::Release);
        Ok(Self {
            factory,
            connection: Some(connection),
            reconnect_interval,
            next_attempt: Instant::now(),
            status,
        })
    }

    pub fn status(&self) -> MidiConnectionStatus {
        self.status.clone()
    }

    fn reconnect_if_due(&mut self) {
        if self.connection.is_some() || Instant::now() < self.next_attempt {
            return;
        }
        self.status
            .0
            .connect_attempts
            .fetch_add(1, Ordering::Relaxed);
        match self.factory.open() {
            Ok(connection) => {
                self.connection = Some(connection);
                self.status.0.connected.store(true, Ordering::Release);
                self.status.0.reconnects.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                self.status.0.connect_errors.fetch_add(1, Ordering::Relaxed);
                self.next_attempt = Instant::now() + self.reconnect_interval;
            }
        }
    }
}

impl<F: MidiSinkFactory> MidiSink for ReconnectingSink<F> {
    fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError> {
        self.reconnect_if_due();
        let Some(connection) = self.connection.as_mut() else {
            self.status
                .0
                .dropped_messages
                .fetch_add(1, Ordering::Relaxed);
            return Ok(());
        };
        if connection.send(bytes).is_err() {
            self.connection = None;
            self.status.0.connected.store(false, Ordering::Release);
            self.status.0.send_errors.fetch_add(1, Ordering::Relaxed);
            self.status
                .0
                .dropped_messages
                .fetch_add(1, Ordering::Relaxed);
            self.next_attempt = Instant::now() + self.reconnect_interval;
        } else {
            self.status.0.sent_messages.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }
}

/// Records everything sent. Used by tests and by `--dry-run`.
#[derive(Debug, Default)]
pub struct CaptureSink {
    pub messages: Vec<Vec<u8>>,
}

impl CaptureSink {
    pub fn new() -> Self {
        Self::default()
    }
    /// All captured bytes flattened, for byte-exact stream assertions.
    pub fn flat(&self) -> Vec<u8> {
        self.messages.iter().flatten().copied().collect()
    }
}

impl MidiSink for CaptureSink {
    fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError> {
        self.messages.push(bytes.to_vec());
        Ok(())
    }
}

/// A real MIDI output port.
pub struct MidirSink {
    conn: midir::MidiOutputConnection,
}

#[derive(Debug, Clone)]
pub struct MidirSinkFactory {
    port_name: String,
}

impl MidirSinkFactory {
    pub fn new(port_name: impl Into<String>) -> Self {
        Self {
            port_name: port_name.into(),
        }
    }
}

impl MidiSinkFactory for MidirSinkFactory {
    fn open(&mut self) -> Result<Box<dyn MidiSink>, MidiError> {
        Ok(Box::new(MidirSink::open_exact(&self.port_name)?))
    }
}

impl std::fmt::Debug for MidirSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MidirSink")
            .field("conn", &"MidiOutputConnection")
            .finish()
    }
}

impl MidirSink {
    pub fn list_ports() -> Result<Vec<String>, MidiError> {
        let out =
            midir::MidiOutput::new("control-room").map_err(|e| MidiError::Init(e.to_string()))?;
        Ok(out
            .ports()
            .iter()
            .filter_map(|p| out.port_name(p).ok())
            .collect())
    }

    /// Resolve a case-insensitive substring to exactly one physical port name.
    /// Ambiguous matches are rejected instead of silently selecting whichever
    /// backend entry happens to be first.
    pub fn resolve_port_name(port_substring: &str) -> Result<String, MidiError> {
        let names = Self::list_ports()?;
        select_unique_port_name(port_substring, &names)
    }

    /// Open the unique output port whose name contains `port_substring`
    /// (case-insensitive).
    pub fn open(port_substring: &str) -> Result<Self, MidiError> {
        let resolved = Self::resolve_port_name(port_substring)?;
        Self::open_exact(&resolved)
    }

    /// Open a port by its already-resolved backend name.
    pub fn open_exact(port_name: &str) -> Result<Self, MidiError> {
        let out =
            midir::MidiOutput::new("control-room").map_err(|e| MidiError::Init(e.to_string()))?;

        let ports = out.ports();
        let mut names = Vec::with_capacity(ports.len());
        let mut found = None;
        for p in &ports {
            let name = out.port_name(p).unwrap_or_default();
            if found.is_none() && name == port_name {
                found = Some(p.clone());
            }
            names.push(name);
        }

        let port = found.ok_or_else(|| MidiError::NoMatchingPort(port_name.to_string(), names))?;
        let conn = out
            .connect(&port, "control-room-out")
            .map_err(|e| MidiError::Connect(e.to_string()))?;
        Ok(Self { conn })
    }
}

fn select_unique_port_name(port_substring: &str, names: &[String]) -> Result<String, MidiError> {
    let needle = port_substring.to_lowercase();
    let matches: Vec<&String> = names
        .iter()
        .filter(|name| name.to_lowercase().contains(&needle))
        .collect();
    match matches.as_slice() {
        [name] => Ok((*name).clone()),
        [] => Err(MidiError::NoMatchingPort(
            port_substring.to_owned(),
            names.to_vec(),
        )),
        _ => Err(MidiError::AmbiguousPort(
            port_substring.to_owned(),
            matches.into_iter().cloned().collect(),
        )),
    }
}

impl MidiSink for MidirSink {
    fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError> {
        self.conn
            .send(bytes)
            .map_err(|e| MidiError::Send(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[test]
    fn capture_sink_records_each_message_separately() {
        let mut s = CaptureSink::new();
        s.send(&[0xB0, 29, 64]).unwrap();
        s.send(&[0xB0, 29, 65]).unwrap();
        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.messages[0], vec![0xB0, 29, 64]);
    }

    #[test]
    fn capture_sink_flattens_for_stream_assertions() {
        let mut s = CaptureSink::new();
        s.send(&[0xB0, 1, 127]).unwrap();
        s.send(&[0xB0, 33, 127]).unwrap();
        assert_eq!(s.flat(), vec![0xB0, 1, 127, 0xB0, 33, 127]);
    }

    #[test]
    fn listing_ports_does_not_error_on_this_host() {
        // Does not assert on contents: a CI box may have zero MIDI ports.
        let _ = MidirSink::list_ports().expect("port enumeration should work");
    }

    #[test]
    fn opening_a_nonexistent_port_reports_available_ports() {
        let err = MidirSink::open("definitely-not-a-real-port-xyz").unwrap_err();
        assert!(matches!(err, MidiError::NoMatchingPort(_, _)));
    }

    #[test]
    fn port_resolution_is_case_insensitive_and_requires_one_match() {
        let names = vec!["Peak MIDI".to_owned(), "RODECaster Duo".to_owned()];
        assert_eq!(
            select_unique_port_name("peak", &names).unwrap(),
            "Peak MIDI"
        );

        let ambiguous = vec!["Peak MIDI".to_owned(), "Peak DIN".to_owned()];
        assert!(matches!(
            select_unique_port_name("peak", &ambiguous),
            Err(MidiError::AmbiguousPort(_, matches)) if matches.len() == 2
        ));
    }

    #[derive(Clone)]
    struct SharedCapture(Arc<Mutex<Vec<Vec<u8>>>>);

    impl MidiSink for SharedCapture {
        fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError> {
            self.0.lock().unwrap().push(bytes.to_vec());
            Ok(())
        }
    }

    struct FailingSink;

    impl MidiSink for FailingSink {
        fn send(&mut self, _bytes: &[u8]) -> Result<(), MidiError> {
            Err(MidiError::Send("fixture disconnected".into()))
        }
    }

    struct SequenceFactory(VecDeque<Result<Box<dyn MidiSink>, MidiError>>);

    impl MidiSinkFactory for SequenceFactory {
        fn open(&mut self) -> Result<Box<dyn MidiSink>, MidiError> {
            self.0
                .pop_front()
                .unwrap_or_else(|| Err(MidiError::Connect("fixture exhausted".into())))
        }
    }

    #[test]
    fn reconnecting_sink_degrades_counts_and_recovers() {
        let capture = Arc::new(Mutex::new(Vec::new()));
        let factory = SequenceFactory(VecDeque::from([
            Ok(Box::new(FailingSink) as Box<dyn MidiSink>),
            Err(MidiError::Connect("still missing".into())),
            Ok(Box::new(SharedCapture(Arc::clone(&capture))) as Box<dyn MidiSink>),
        ]));
        let mut sink = ReconnectingSink::connect(factory, Duration::ZERO).unwrap();
        let status = sink.status();

        sink.send(&[0xb0, 20, 1]).unwrap();
        sink.send(&[0xb0, 20, 2]).unwrap();
        sink.send(&[0xb0, 20, 3]).unwrap();

        assert_eq!(*capture.lock().unwrap(), vec![vec![0xb0, 20, 3]]);
        assert_eq!(
            status.snapshot(),
            MidiConnectionStats {
                connected: true,
                connect_attempts: 3,
                reconnects: 1,
                connect_errors: 1,
                send_errors: 1,
                dropped_messages: 2,
                sent_messages: 1,
            }
        );
    }
}
