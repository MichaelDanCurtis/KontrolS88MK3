//! Kontrol S88 surface boundary.
//!
//! The supported v1 input path is a user-created standard MIDI template whose
//! knobs emit absolute CC values. The proprietary display/DAW protocol stays
//! behind [`DisplayTransport`]; no byte sequence is called verified until it
//! has a hardware capture fixture.

use cr_core::{BindingId, GraphCommand};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use midir::{Ignore, MidiInput};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;
use thiserror::Error;

mod capture;

pub use capture::{
    CaptureDiff, CaptureDirection, CaptureEvent, CaptureFixture, CaptureProtocol,
    CaptureReplayTransport, EventDiff, UsbCaptureAnalysis,
};

const DEFAULT_QUEUE_CAPACITY: usize = 128;
const DEFAULT_RECONNECT_INTERVAL_MS: u64 = 500;

fn default_queue_capacity() -> usize {
    DEFAULT_QUEUE_CAPACITY
}

fn default_reconnect_interval_ms() -> u64 {
    DEFAULT_RECONNECT_INTERVAL_MS
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnobBinding {
    pub id: BindingId,
    pub cc: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct S88Config {
    pub enabled: bool,
    pub port: String,
    /// MIDI channel as printed in the keyboard template, 1-16.
    pub channel: u8,
    #[serde(default = "default_queue_capacity")]
    pub queue_capacity: usize,
    #[serde(default = "default_reconnect_interval_ms")]
    pub reconnect_interval_ms: u64,
    pub knobs: Vec<KnobBinding>,
}

impl Default for S88Config {
    fn default() -> Self {
        Self {
            enabled: false,
            port: "KONTROL S88 MK3".into(),
            channel: 1,
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            reconnect_interval_ms: DEFAULT_RECONNECT_INTERVAL_MS,
            knobs: Vec::new(),
        }
    }
}

impl S88Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=16).contains(&self.channel),
            "s88.channel must be in 1..=16"
        );
        anyhow::ensure!(
            self.queue_capacity > 0,
            "s88.queue_capacity must be greater than zero"
        );
        anyhow::ensure!(
            self.reconnect_interval_ms > 0,
            "s88.reconnect_interval_ms must be greater than zero"
        );
        if self.enabled {
            anyhow::ensure!(!self.port.trim().is_empty(), "s88.port must not be empty");
            anyhow::ensure!(
                !self.knobs.is_empty(),
                "enabled s88 config requires at least one knob"
            );
        }
        let mut ids = HashSet::new();
        let mut controls = HashSet::new();
        for knob in &self.knobs {
            anyhow::ensure!(
                !knob.id.as_str().trim().is_empty(),
                "s88 knob id must not be empty"
            );
            anyhow::ensure!(
                knob.cc <= 127,
                "s88 knob `{}` CC must be in 0..=127",
                knob.id
            );
            anyhow::ensure!(
                ids.insert(knob.id.clone()),
                "duplicate s88 knob id `{}`",
                knob.id
            );
            anyhow::ensure!(
                controls.insert(knob.cc),
                "duplicate s88 knob CC {}",
                knob.cc
            );
        }
        Ok(())
    }

    pub fn has_input(&self, binding: &str) -> bool {
        self.enabled && self.knobs.iter().any(|knob| knob.id.as_str() == binding)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct S88Input {
    pub binding: BindingId,
    pub value: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct S88Stats {
    pub connected: bool,
    pub connect_attempts: u64,
    pub reconnects: u64,
    pub received: u64,
    pub unmatched: u64,
    pub dropped: u64,
}

#[derive(Default)]
pub struct TransportStats {
    connected: AtomicBool,
    connect_attempts: AtomicU64,
    reconnects: AtomicU64,
    received: AtomicU64,
    unmatched: AtomicU64,
    dropped: AtomicU64,
}

impl TransportStats {
    pub fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::Release);
    }

    pub fn record_connect_attempt(&self) {
        self.connect_attempts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_reconnect(&self) {
        self.reconnects.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_received(&self) {
        self.received.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_dropped(&self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> S88Stats {
        let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
        S88Stats {
            connected: self.connected.load(Ordering::Relaxed),
            connect_attempts: load(&self.connect_attempts),
            reconnects: load(&self.reconnects),
            received: load(&self.received),
            unmatched: load(&self.unmatched),
            dropped: load(&self.dropped),
        }
    }
}

/// Raw transport boundary used by live MIDI and capture-replay tests.
pub trait InputTransport: Send + 'static {
    fn run(
        self,
        config: S88Config,
        events: Sender<Vec<u8>>,
        running: Arc<AtomicBool>,
        stats: Arc<TransportStats>,
    );
}

#[derive(Debug, Default)]
pub struct MidirInputTransport;

/// Enumerate the MIDI input endpoints visible to the current host.
pub fn list_input_ports() -> anyhow::Result<Vec<String>> {
    let input = MidiInput::new("control-room-s88-list")?;
    input
        .ports()
        .iter()
        .map(|port| input.port_name(port).map_err(Into::into))
        .collect()
}

/// Resolve a case-insensitive substring to one exact MIDI input endpoint.
pub fn resolve_input_port_name(needle: &str) -> anyhow::Result<String> {
    resolve_port_name_from(list_input_ports()?, needle)
}

fn resolve_port_name_from(names: Vec<String>, needle: &str) -> anyhow::Result<String> {
    let normalized = needle.to_lowercase();
    let matches: Vec<_> = names
        .into_iter()
        .filter(|name| name.to_lowercase().contains(&normalized))
        .collect();
    anyhow::ensure!(!matches.is_empty(), "no MIDI input port matches {needle:?}");
    anyhow::ensure!(
        matches.len() == 1,
        "MIDI input port match {needle:?} is ambiguous; matches: {matches:?}"
    );
    Ok(matches.into_iter().next().unwrap())
}

impl InputTransport for MidirInputTransport {
    fn run(
        self,
        config: S88Config,
        events: Sender<Vec<u8>>,
        running: Arc<AtomicBool>,
        stats: Arc<TransportStats>,
    ) {
        let mut connected_once = false;
        while running.load(Ordering::Acquire) {
            stats.record_connect_attempt();
            let Ok(mut input) = MidiInput::new("control-room-s88") else {
                std::thread::sleep(Duration::from_millis(config.reconnect_interval_ms));
                continue;
            };
            input.ignore(Ignore::None);
            let Ok((port, resolved_name)) = resolve_input_port(&input, &config.port) else {
                std::thread::sleep(Duration::from_millis(config.reconnect_interval_ms));
                continue;
            };
            let callback_events = events.clone();
            let callback_stats = Arc::clone(&stats);
            let Ok(connection) = input.connect(
                &port,
                "control-room-s88-input",
                move |_timestamp, message, _| {
                    callback_stats.record_received();
                    if let Err(error) = callback_events.try_send(message.to_vec())
                        && matches!(error, TrySendError::Full(_))
                    {
                        callback_stats.record_dropped();
                    }
                },
                (),
            ) else {
                std::thread::sleep(Duration::from_millis(config.reconnect_interval_ms));
                continue;
            };
            if connected_once {
                stats.record_reconnect();
            }
            connected_once = true;
            stats.set_connected(true);

            while running.load(Ordering::Acquire)
                && input_port_is_present(&resolved_name).unwrap_or(true)
            {
                std::thread::sleep(Duration::from_millis(config.reconnect_interval_ms));
            }
            stats.set_connected(false);
            drop(connection);
        }
    }
}

fn resolve_input_port(
    input: &MidiInput,
    needle: &str,
) -> anyhow::Result<(midir::MidiInputPort, String)> {
    let needle = needle.to_lowercase();
    let mut matches = Vec::new();
    for port in input.ports() {
        let name = input.port_name(&port)?;
        if name.to_lowercase().contains(&needle) {
            matches.push((port, name));
        }
    }
    anyhow::ensure!(!matches.is_empty(), "no MIDI input port matches {needle:?}");
    anyhow::ensure!(
        matches.len() == 1,
        "MIDI input port match {needle:?} is ambiguous"
    );
    Ok(matches.remove(0))
}

fn input_port_is_present(exact_name: &str) -> anyhow::Result<bool> {
    let input = MidiInput::new("control-room-s88-monitor")?;
    for port in input.ports() {
        if input.port_name(&port)? == exact_name {
            return Ok(true);
        }
    }
    Ok(false)
}

pub struct S88Adapter {
    bindings: HashMap<u8, BindingId>,
    channel: u8,
    events: Receiver<Vec<u8>>,
    running: Arc<AtomicBool>,
    stats: Arc<TransportStats>,
    thread: Option<JoinHandle<()>>,
}

impl S88Adapter {
    pub fn start(config: S88Config) -> anyhow::Result<Self> {
        Self::start_with_transport(config, MidirInputTransport)
    }

    pub fn start_with_transport<T: InputTransport>(
        config: S88Config,
        transport: T,
    ) -> anyhow::Result<Self> {
        config.validate()?;
        let bindings = config
            .knobs
            .iter()
            .map(|knob| (knob.cc, knob.id.clone()))
            .collect();
        let channel = config.channel - 1;
        let (event_tx, events) = bounded(config.queue_capacity);
        let running = Arc::new(AtomicBool::new(true));
        let thread_running = Arc::clone(&running);
        let stats = Arc::new(TransportStats::default());
        let thread_stats = Arc::clone(&stats);
        let thread_config = config;
        let thread = std::thread::spawn(move || {
            transport.run(thread_config, event_tx, thread_running, thread_stats);
        });
        Ok(Self {
            bindings,
            channel,
            events,
            running,
            stats,
            thread: Some(thread),
        })
    }

    pub fn try_recv(&self) -> Option<S88Input> {
        while let Ok(bytes) = self.events.try_recv() {
            if let Some(input) = decode_cc(&bytes, self.channel, &self.bindings) {
                return Some(input);
            }
            self.stats.unmatched.fetch_add(1, Ordering::Relaxed);
        }
        None
    }

    pub fn stats(&self) -> S88Stats {
        self.stats.snapshot()
    }
}

impl Drop for S88Adapter {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn decode_cc(bytes: &[u8], channel: u8, bindings: &HashMap<u8, BindingId>) -> Option<S88Input> {
    let [status, cc, value] = bytes else {
        return None;
    };
    if status & 0xf0 != 0xb0 || status & 0x0f != channel || *value > 127 {
        return None;
    }
    Some(S88Input {
        binding: bindings.get(cc)?.clone(),
        value: f32::from(*value) / 127.0,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceParameter {
    pub label: String,
    pub value: String,
    pub binding: Option<BindingId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceModule {
    pub title: String,
    pub parameters: Vec<SurfaceParameter>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceBank {
    pub name: String,
    pub modules: Vec<SurfaceModule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceFrame {
    pub bank: String,
    pub module: String,
    pub page: usize,
    pub parameters: Vec<SurfaceParameter>,
}

impl SurfaceBank {
    pub fn frame(&self, module: usize, page: usize) -> Option<SurfaceFrame> {
        let module = self.modules.get(module)?;
        let start = page.checked_mul(8)?;
        let parameters = module
            .parameters
            .iter()
            .skip(start)
            .take(8)
            .cloned()
            .collect();
        Some(SurfaceFrame {
            bank: self.name.clone(),
            module: module.title.clone(),
            page,
            parameters,
        })
    }
}

/// Surface edits use the same revisioned graph-command batches as the browser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceCommandBatch {
    pub base_revision: u64,
    pub commands: Vec<GraphCommand>,
}

#[derive(Debug, Error)]
pub enum DisplayError {
    #[error("S88 display protocol has not been verified against hardware captures")]
    UnverifiedProtocol,
    #[error("display transport failed: {0}")]
    Transport(String),
}

/// Semantic boundary for a future verified DAW-port/display implementation.
pub trait DisplayTransport {
    fn render(&mut self, frame: &SurfaceFrame) -> Result<(), DisplayError>;
}

#[derive(Debug, Default)]
pub struct UnverifiedDisplayTransport;

impl DisplayTransport for UnverifiedDisplayTransport {
    fn render(&mut self, _frame: &SurfaceFrame) -> Result<(), DisplayError> {
        Err(DisplayError::UnverifiedProtocol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ReplayTransport(Vec<Vec<u8>>);

    impl InputTransport for ReplayTransport {
        fn run(
            self,
            _config: S88Config,
            events: Sender<Vec<u8>>,
            _running: Arc<AtomicBool>,
            stats: Arc<TransportStats>,
        ) {
            for event in self.0 {
                stats.received.fetch_add(1, Ordering::Relaxed);
                let _ = events.send(event);
            }
        }
    }

    fn config() -> S88Config {
        S88Config {
            enabled: true,
            port: "fixture".into(),
            channel: 2,
            queue_capacity: 8,
            reconnect_interval_ms: 1,
            knobs: vec![KnobBinding {
                id: BindingId::new("s88.knob.1"),
                cc: 20,
            }],
        }
    }

    #[test]
    fn input_port_resolution_is_case_insensitive_and_unambiguous() {
        let ports = vec!["Native Instruments KONTROL S88 MK3 MIDI".to_owned()];
        assert_eq!(
            resolve_port_name_from(ports, "s88 mk3").unwrap(),
            "Native Instruments KONTROL S88 MK3 MIDI"
        );
        assert!(resolve_port_name_from(vec!["Peak".into()], "S88").is_err());
        assert!(resolve_port_name_from(vec!["S88 MIDI".into(), "S88 DAW".into()], "S88").is_err());
    }

    #[test]
    fn replays_byte_exact_standard_midi_template_cc() {
        let adapter =
            S88Adapter::start_with_transport(config(), ReplayTransport(vec![vec![0xb1, 20, 64]]))
                .unwrap();
        let mut input = None;
        for _ in 0..100 {
            input = adapter.try_recv();
            if input.is_some() {
                break;
            }
            std::thread::yield_now();
        }
        let input = input.expect("fixture event should arrive");
        assert_eq!(input.binding, BindingId::new("s88.knob.1"));
        assert!((input.value - (64.0 / 127.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn wrong_channel_and_unknown_cc_are_observable() {
        let adapter = S88Adapter::start_with_transport(
            config(),
            ReplayTransport(vec![vec![0xb0, 20, 64], vec![0xb1, 21, 64]]),
        )
        .unwrap();
        for _ in 0..100 {
            let _ = adapter.try_recv();
            if adapter.stats().unmatched == 2 {
                break;
            }
            std::thread::yield_now();
        }
        assert_eq!(adapter.stats().unmatched, 2);
    }

    #[test]
    fn banks_page_in_groups_of_eight_and_display_is_explicitly_unverified() {
        let bank = SurfaceBank {
            name: "Peak".into(),
            modules: vec![SurfaceModule {
                title: "Filter".into(),
                parameters: (0..10)
                    .map(|index| SurfaceParameter {
                        label: format!("P{index}"),
                        value: index.to_string(),
                        binding: None,
                    })
                    .collect(),
            }],
        };
        assert_eq!(bank.frame(0, 0).unwrap().parameters.len(), 8);
        assert_eq!(bank.frame(0, 1).unwrap().parameters.len(), 2);
        let mut display = UnverifiedDisplayTransport;
        assert!(matches!(
            display.render(&bank.frame(0, 0).unwrap()),
            Err(DisplayError::UnverifiedProtocol)
        ));
    }

    #[test]
    fn config_rejects_duplicate_controls() {
        let mut config = config();
        config.knobs.push(KnobBinding {
            id: BindingId::new("other"),
            cc: 20,
        });
        assert!(config.validate().is_err());
    }
}
