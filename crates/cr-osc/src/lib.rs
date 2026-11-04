//! OSC adapter for Control Room external graph bindings.

use cr_core::BindingId;
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use rosc::{OscMessage, OscPacket, OscType};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use thiserror::Error;

const DEFAULT_QUEUE_CAPACITY: usize = 256;
const DEFAULT_OUTPUT_INTERVAL_MS: u64 = 10;

fn default_queue_capacity() -> usize {
    DEFAULT_QUEUE_CAPACITY
}

fn default_output_interval_ms() -> u64 {
    DEFAULT_OUTPUT_INTERVAL_MS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Input,
    Output,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingConfig {
    pub id: BindingId,
    pub direction: Direction,
    pub address: String,
    #[serde(default)]
    pub argument: usize,
    pub min: f32,
    pub max: f32,
    /// Required for output bindings; ignored for inputs.
    pub peer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OscConfig {
    pub enabled: bool,
    pub listen: String,
    #[serde(default = "default_queue_capacity")]
    pub queue_capacity: usize,
    #[serde(default = "default_output_interval_ms")]
    pub output_interval_ms: u64,
    pub bindings: Vec<BindingConfig>,
}

impl Default for OscConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: "127.0.0.1:9000".into(),
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            output_interval_ms: DEFAULT_OUTPUT_INTERVAL_MS,
            bindings: Vec::new(),
        }
    }
}

impl OscConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.listen.parse::<SocketAddr>().is_ok(),
            "osc.listen must be an IP socket address, got {:?}",
            self.listen
        );
        anyhow::ensure!(
            self.queue_capacity > 0,
            "osc.queue_capacity must be greater than zero"
        );
        let mut ids = HashSet::new();
        for binding in &self.bindings {
            anyhow::ensure!(
                ids.insert((binding.direction, binding.id.clone())),
                "duplicate {:?} OSC binding `{}`",
                binding.direction,
                binding.id
            );
            anyhow::ensure!(
                binding.address.starts_with('/') && binding.address.len() > 1,
                "OSC binding `{}` address must begin with / and contain a name",
                binding.id
            );
            anyhow::ensure!(
                binding.min.is_finite() && binding.max.is_finite() && binding.min < binding.max,
                "OSC binding `{}` has invalid range {}..{}",
                binding.id,
                binding.min,
                binding.max
            );
            if binding.direction == Direction::Output {
                let peer = binding.peer.as_deref().unwrap_or_default();
                anyhow::ensure!(
                    peer.parse::<SocketAddr>().is_ok(),
                    "output OSC binding `{}` requires an IP peer socket address",
                    binding.id
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OscInput {
    pub binding: BindingId,
    pub value: f32,
}

#[derive(Debug, Clone)]
struct OscOutput {
    binding: BindingId,
    value: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OscStats {
    pub received: u64,
    pub malformed: u64,
    pub type_errors: u64,
    pub unmatched: u64,
    pub input_dropped: u64,
    pub sent: u64,
    pub deduped: u64,
    pub rate_limited: u64,
    pub send_errors: u64,
    pub output_dropped: u64,
}

#[derive(Default)]
struct AtomicStats {
    received: AtomicU64,
    malformed: AtomicU64,
    type_errors: AtomicU64,
    unmatched: AtomicU64,
    input_dropped: AtomicU64,
    sent: AtomicU64,
    deduped: AtomicU64,
    rate_limited: AtomicU64,
    send_errors: AtomicU64,
    output_dropped: AtomicU64,
}

impl AtomicStats {
    fn snapshot(&self) -> OscStats {
        let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
        OscStats {
            received: load(&self.received),
            malformed: load(&self.malformed),
            type_errors: load(&self.type_errors),
            unmatched: load(&self.unmatched),
            input_dropped: load(&self.input_dropped),
            sent: load(&self.sent),
            deduped: load(&self.deduped),
            rate_limited: load(&self.rate_limited),
            send_errors: load(&self.send_errors),
            output_dropped: load(&self.output_dropped),
        }
    }
}

#[derive(Debug, Error)]
pub enum OscSendError {
    #[error("unknown OSC output binding {0}")]
    UnknownBinding(BindingId),
    #[error("OSC output queue is full")]
    QueueFull,
    #[error("OSC adapter has stopped")]
    Stopped,
    #[error("OSC output value must be finite")]
    NonFinite,
}

pub struct OscAdapter {
    local_addr: SocketAddr,
    inputs: Receiver<OscInput>,
    outputs: Sender<OscOutput>,
    input_bindings: HashSet<BindingId>,
    output_bindings: HashSet<BindingId>,
    stats: Arc<AtomicStats>,
    running: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl OscAdapter {
    pub fn start(config: OscConfig) -> anyhow::Result<Self> {
        config.validate()?;
        let socket = UdpSocket::bind(&config.listen)?;
        socket.set_read_timeout(Some(Duration::from_millis(5)))?;
        let local_addr = socket.local_addr()?;
        let (input_tx, inputs) = bounded(config.queue_capacity);
        let (outputs, output_rx) = bounded(config.queue_capacity);
        let input_bindings: HashSet<_> = config
            .bindings
            .iter()
            .filter(|binding| binding.direction == Direction::Input)
            .map(|binding| binding.id.clone())
            .collect();
        let output_bindings: HashSet<_> = config
            .bindings
            .iter()
            .filter(|binding| binding.direction == Direction::Output)
            .map(|binding| binding.id.clone())
            .collect();
        let input_map = build_input_map(&config.bindings);
        let output_map = build_output_map(&config.bindings)?;
        let stats = Arc::new(AtomicStats::default());
        let thread_stats = Arc::clone(&stats);
        let running = Arc::new(AtomicBool::new(true));
        let thread_running = Arc::clone(&running);
        let output_interval = Duration::from_millis(config.output_interval_ms);
        let thread = std::thread::spawn(move || {
            io_loop(
                socket,
                input_map,
                output_map,
                input_tx,
                output_rx,
                output_interval,
                thread_stats,
                thread_running,
            );
        });
        Ok(Self {
            local_addr,
            inputs,
            outputs,
            input_bindings,
            output_bindings,
            stats,
            running,
            thread: Some(thread),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn has_input(&self, binding: &str) -> bool {
        self.input_bindings.contains(&BindingId::new(binding))
    }

    pub fn has_output(&self, binding: &str) -> bool {
        self.output_bindings.contains(&BindingId::new(binding))
    }

    pub fn try_recv(&self) -> Option<OscInput> {
        self.inputs.try_recv().ok()
    }

    pub fn send(&self, binding: &str, value: f32) -> Result<(), OscSendError> {
        if !value.is_finite() {
            return Err(OscSendError::NonFinite);
        }
        let binding = BindingId::new(binding);
        if !self.output_bindings.contains(&binding) {
            return Err(OscSendError::UnknownBinding(binding));
        }
        match self.outputs.try_send(OscOutput { binding, value }) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.stats.output_dropped.fetch_add(1, Ordering::Relaxed);
                Err(OscSendError::QueueFull)
            }
            Err(TrySendError::Disconnected(_)) => Err(OscSendError::Stopped),
        }
    }

    pub fn stats(&self) -> OscStats {
        self.stats.snapshot()
    }

    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for OscAdapter {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone)]
struct InputBinding {
    id: BindingId,
    argument: usize,
    min: f32,
    max: f32,
}

#[derive(Clone)]
struct OutputBinding {
    address: String,
    peer: SocketAddr,
    min: f32,
    max: f32,
}

fn build_input_map(bindings: &[BindingConfig]) -> HashMap<String, Vec<InputBinding>> {
    let mut map: HashMap<String, Vec<InputBinding>> = HashMap::new();
    for binding in bindings
        .iter()
        .filter(|binding| binding.direction == Direction::Input)
    {
        map.entry(binding.address.clone())
            .or_default()
            .push(InputBinding {
                id: binding.id.clone(),
                argument: binding.argument,
                min: binding.min,
                max: binding.max,
            });
    }
    map
}

fn build_output_map(
    bindings: &[BindingConfig],
) -> anyhow::Result<HashMap<BindingId, OutputBinding>> {
    bindings
        .iter()
        .filter(|binding| binding.direction == Direction::Output)
        .map(|binding| {
            Ok((
                binding.id.clone(),
                OutputBinding {
                    address: binding.address.clone(),
                    peer: binding.peer.as_deref().unwrap_or_default().parse()?,
                    min: binding.min,
                    max: binding.max,
                },
            ))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn io_loop(
    socket: UdpSocket,
    input_map: HashMap<String, Vec<InputBinding>>,
    output_map: HashMap<BindingId, OutputBinding>,
    input_tx: Sender<OscInput>,
    output_rx: Receiver<OscOutput>,
    output_interval: Duration,
    stats: Arc<AtomicStats>,
    running: Arc<AtomicBool>,
) {
    let mut buffer = [0u8; rosc::decoder::MTU];
    let mut last_output: HashMap<BindingId, (f32, Instant)> = HashMap::new();
    while running.load(Ordering::Acquire) {
        while let Ok(output) = output_rx.try_recv() {
            let Some(binding) = output_map.get(&output.binding) else {
                continue;
            };
            if let Some((last_value, last_time)) = last_output.get(&output.binding) {
                if *last_value == output.value {
                    stats.deduped.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                if last_time.elapsed() < output_interval {
                    stats.rate_limited.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
            }
            let wire = binding.min + output.value.clamp(0.0, 1.0) * (binding.max - binding.min);
            let packet = OscPacket::Message(OscMessage {
                addr: binding.address.clone(),
                args: vec![OscType::Float(wire)],
            });
            match rosc::encoder::encode(&packet)
                .ok()
                .and_then(|bytes| socket.send_to(&bytes, binding.peer).ok())
            {
                Some(_) => {
                    stats.sent.fetch_add(1, Ordering::Relaxed);
                    last_output.insert(output.binding, (output.value, Instant::now()));
                }
                None => {
                    stats.send_errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        }

        match socket.recv_from(&mut buffer) {
            Ok((length, _peer)) => match rosc::decoder::decode_udp(&buffer[..length]) {
                Ok((_remaining, packet)) => {
                    stats.received.fetch_add(1, Ordering::Relaxed);
                    dispatch_packet(&packet, &input_map, &input_tx, &stats);
                }
                Err(_) => {
                    stats.malformed.fetch_add(1, Ordering::Relaxed);
                }
            },
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => {
                stats.malformed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn dispatch_packet(
    packet: &OscPacket,
    input_map: &HashMap<String, Vec<InputBinding>>,
    input_tx: &Sender<OscInput>,
    stats: &AtomicStats,
) {
    match packet {
        OscPacket::Message(message) => {
            let Some(bindings) = input_map.get(&message.addr) else {
                stats.unmatched.fetch_add(1, Ordering::Relaxed);
                return;
            };
            for binding in bindings {
                let Some(value) = message.args.get(binding.argument).and_then(numeric_value) else {
                    stats.type_errors.fetch_add(1, Ordering::Relaxed);
                    continue;
                };
                let normalized =
                    ((value - binding.min) / (binding.max - binding.min)).clamp(0.0, 1.0);
                if input_tx
                    .try_send(OscInput {
                        binding: binding.id.clone(),
                        value: normalized,
                    })
                    .is_err()
                {
                    stats.input_dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        OscPacket::Bundle(bundle) => {
            for packet in &bundle.content {
                dispatch_packet(packet, input_map, input_tx, stats);
            }
        }
    }
}

fn numeric_value(value: &OscType) -> Option<f32> {
    let value = match value {
        OscType::Float(value) => *value,
        OscType::Double(value) => *value as f32,
        OscType::Int(value) => *value as f32,
        OscType::Long(value) => *value as f32,
        _ => return None,
    };
    value.is_finite().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_binding() -> BindingConfig {
        BindingConfig {
            id: BindingId::new("knob"),
            direction: Direction::Input,
            address: "/control/knob".into(),
            argument: 0,
            min: 0.0,
            max: 127.0,
            peer: None,
        }
    }

    #[test]
    fn validation_rejects_bad_addresses_ranges_and_output_peers() {
        let mut config = OscConfig {
            enabled: true,
            listen: "127.0.0.1:0".into(),
            bindings: vec![input_binding()],
            ..OscConfig::default()
        };
        assert!(config.validate().is_ok());
        config.bindings[0].address = "missing-slash".into();
        assert!(config.validate().is_err());
        config.bindings[0].address = "/ok".into();
        config.bindings[0].min = 10.0;
        config.bindings[0].max = 1.0;
        assert!(config.validate().is_err());

        config.bindings = vec![BindingConfig {
            direction: Direction::Output,
            peer: None,
            ..input_binding()
        }];
        assert!(config.validate().is_err());
    }

    #[test]
    fn udp_loopback_normalizes_input_and_maps_output() {
        let output_receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        output_receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let config = OscConfig {
            enabled: true,
            listen: "127.0.0.1:0".into(),
            queue_capacity: 8,
            output_interval_ms: 0,
            bindings: vec![
                input_binding(),
                BindingConfig {
                    id: BindingId::new("meter"),
                    direction: Direction::Output,
                    address: "/control/meter".into(),
                    argument: 0,
                    min: 10.0,
                    max: 20.0,
                    peer: Some(output_receiver.local_addr().unwrap().to_string()),
                },
            ],
        };
        let adapter = OscAdapter::start(config).unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let input_packet = OscPacket::Message(OscMessage {
            addr: "/control/knob".into(),
            args: vec![OscType::Float(63.5)],
        });
        sender
            .send_to(
                &rosc::encoder::encode(&input_packet).unwrap(),
                adapter.local_addr(),
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(1);
        let input = loop {
            if let Some(input) = adapter.try_recv() {
                break input;
            }
            assert!(Instant::now() < deadline, "OSC input timed out");
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(input.binding, BindingId::new("knob"));
        assert!((input.value - 0.5).abs() < 1e-6);

        adapter.send("meter", 0.25).unwrap();
        let mut bytes = [0u8; 256];
        let (length, _) = output_receiver.recv_from(&mut bytes).unwrap();
        let (_, packet) = rosc::decoder::decode_udp(&bytes[..length]).unwrap();
        let OscPacket::Message(message) = packet else {
            panic!("expected message")
        };
        assert_eq!(message.addr, "/control/meter");
        assert_eq!(message.args, vec![OscType::Float(12.5)]);
        assert_eq!(adapter.stats().sent, 1);
    }

    #[test]
    fn malformed_wrong_type_and_unmatched_packets_are_observable() {
        let adapter = OscAdapter::start(OscConfig {
            enabled: true,
            listen: "127.0.0.1:0".into(),
            bindings: vec![input_binding()],
            ..OscConfig::default()
        })
        .unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        sender.send_to(&[1, 2, 3], adapter.local_addr()).unwrap();
        for packet in [
            OscPacket::Message(OscMessage {
                addr: "/control/knob".into(),
                args: vec![OscType::String("nope".into())],
            }),
            OscPacket::Message(OscMessage {
                addr: "/unknown".into(),
                args: vec![OscType::Float(1.0)],
            }),
        ] {
            sender
                .send_to(
                    &rosc::encoder::encode(&packet).unwrap(),
                    adapter.local_addr(),
                )
                .unwrap();
        }

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let stats = adapter.stats();
            if stats.malformed >= 1 && stats.type_errors >= 1 && stats.unmatched >= 1 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "OSC error counters timed out: {stats:?}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
