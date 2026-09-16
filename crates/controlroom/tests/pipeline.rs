//! End-to-end: config file -> graph -> engine -> pack -> limiter -> bytes.
//! Runs with no hardware attached.

use controlroom::config::{DeviceConfig, PatchConfig};
use controlroom::runtime::{DeviceEndpoint, Runtime};
use cr_core::{DeviceId, Graph, GraphNode, LfoShape, NodeId, NodeSpec, ParamName, compile};
use cr_midi::{BusLimiterConfig, CaptureSink, LimiterConfig, MidiSink};
use cr_packs::{DevicePack, NOVATION_PEAK_YAML};
use std::sync::{Arc, Mutex};

/// A sink that shares its capture buffer so the test can inspect it after
/// the runtime has taken ownership.
struct SharedSink(Arc<Mutex<CaptureSink>>);

impl MidiSink for SharedSink {
    fn send(&mut self, bytes: &[u8]) -> Result<(), cr_midi::MidiError> {
        self.0.lock().expect("sink lock").send(bytes)
    }
}

fn run(ticks: u64) -> (Arc<Mutex<CaptureSink>>, cr_midi::BudgetedLimiterStats) {
    let cfg = PatchConfig::from_yaml(include_str!("../../../config/lfo-cutoff.yaml"))
        .expect("demo config parses");
    let graph = compile(&cfg.to_graph()).expect("demo graph compiles");
    let pack = DevicePack::from_yaml(NOVATION_PEAK_YAML).expect("peak pack loads");

    let capture = Arc::new(Mutex::new(CaptureSink::new()));
    let mut rt = Runtime::new(
        graph,
        pack,
        cfg.device.clone(),
        Box::new(SharedSink(Arc::clone(&capture))),
    )
    .expect("runtime must construct: pack covers every sink and every device matches");
    rt.run_ticks(0, ticks).expect("run must not error");
    let stats = rt.stats();
    (capture, stats)
}

#[test]
fn emits_cc29_on_channel_1_for_the_demo_patch() {
    let (capture, _) = run(2000);
    let c = capture.lock().unwrap();
    assert!(!c.messages.is_empty(), "expected some MIDI output");

    for m in &c.messages {
        assert_eq!(
            m.len(),
            3,
            "the pack declares cutoff as a 7-bit CC: 3 bytes"
        );
        assert_eq!(m[0], 0xB0, "status: CC on channel 1 (wire 0)");
        assert_eq!(
            m[1], 29,
            "the pack's documented cutoff CC (29) reaches the wire"
        );
        assert!(m[2] < 0x80, "data byte must not set the status bit");
    }
}

#[test]
fn first_message_matches_the_lfo_start_of_cycle() {
    let (capture, _) = run(1);
    let c = capture.lock().unwrap();
    // tick 0: triangle = 0.0 -> scale = 20.0 -> quantize(20, 0..127, 7bit) = 20
    assert_eq!(c.messages[0], vec![0xB0, 29, 20]);
}

#[test]
fn output_sweeps_across_the_configured_range() {
    // 0.3 Hz at 1 kHz => a full cycle is ~3333 ticks.
    let (capture, _) = run(4000);
    let c = capture.lock().unwrap();
    let values: Vec<u8> = c
        .messages
        .iter()
        .filter_map(|m| m.get(2).copied())
        .collect();
    let min = *values.iter().min().expect("some output");
    let max = *values.iter().max().expect("some output");

    // scale is 20..90 over 0..127 => quantized roughly 20..90.
    assert!(min <= 21, "should reach the bottom of the range, got {min}");
    assert!(max >= 89, "should reach the top of the range, got {max}");
}

#[test]
fn rate_limiting_keeps_output_within_the_midi_budget() {
    let ticks = 10_000u64; // 10 seconds at 1 kHz
    let (capture, stats) = run(ticks);
    let sent = capture.lock().unwrap().messages.len() as u64;

    assert_eq!(sent, stats.sent, "every admitted update reached the sink");
    // Default budget is one send per 10 ticks per parameter.
    assert!(
        sent <= ticks / 10 + 1,
        "sent {sent} exceeds the 100 Hz per-parameter budget"
    );
}

#[test]
fn every_suppressed_update_is_accounted_for() {
    let ticks = 5000u64;
    let (_, stats) = run(ticks);
    // One sink fires once per tick, so the counters must sum to the tick count.
    assert_eq!(
        stats.sent
            + stats.deduped
            + stats.parameter_rate_limited
            + stats.bus_rate_limited
            + stats.unknown_slot,
        ticks,
        "suppression must never be silent"
    );
    assert_eq!(
        stats.unknown_slot, 0,
        "every sink resolves, so no key should be unknown"
    );
}

/// A minimal one-lfo-into-one-sink patch, with the sink's `device` and
/// `param` filled in by the caller, so each unresolvable-sink test can
/// isolate exactly one bad field.
fn patch_yaml(sink_device: &str, sink_param: &str) -> String {
    format!(
        r#"
device:
  id: peak
  port: Peak
  channel: 1
nodes:
  - id: lfo1
    spec: !Lfo
      shape: triangle
      freq_hz: 0.3
    input: null
  - id: cutoff
    spec: !Sink
      device: {sink_device}
      param: {sink_param}
    input: lfo1
"#
    )
}

#[test]
fn construction_fails_when_sink_names_a_parameter_absent_from_the_pack() {
    let yaml = patch_yaml("peak", "no.such.parameter");
    let cfg = PatchConfig::from_yaml(&yaml).expect("config parses");
    let graph = compile(&cfg.to_graph()).expect("graph compiles");
    let pack = DevicePack::from_yaml(NOVATION_PEAK_YAML).expect("pack loads");

    let err = Runtime::new(
        graph,
        pack,
        cfg.device.clone(),
        Box::new(CaptureSink::new()),
    )
    .err()
    .expect("construction must fail: the pack does not define no.such.parameter");
    let msg = err.to_string();
    assert!(
        msg.contains("no.such.parameter"),
        "error should name the offending parameter, got: {msg}"
    );
}

#[test]
fn construction_fails_when_sink_names_a_device_other_than_the_configured_one() {
    let yaml = patch_yaml("some-totally-different-synth", "filter.cutoff");
    let cfg = PatchConfig::from_yaml(&yaml).expect("config parses");
    let graph = compile(&cfg.to_graph()).expect("graph compiles");
    let pack = DevicePack::from_yaml(NOVATION_PEAK_YAML).expect("pack loads");

    let err = Runtime::new(
        graph,
        pack,
        cfg.device.clone(),
        Box::new(CaptureSink::new()),
    )
    .err()
    .expect("construction must fail: the sink names a device other than the configured peak");
    let msg = err.to_string();
    assert!(
        msg.contains("some-totally-different-synth"),
        "error should name the offending device, got: {msg}"
    );
}

fn two_device_graph() -> cr_core::CompiledGraph {
    compile(&Graph {
        nodes: vec![
            GraphNode {
                id: NodeId::new("lfo"),
                spec: NodeSpec::Lfo {
                    shape: LfoShape::Triangle,
                    freq_hz: 1.0,
                },
                input: None,
            },
            GraphNode {
                id: NodeId::new("scale"),
                spec: NodeSpec::Scale {
                    min: 0.0,
                    max: 127.0,
                },
                input: Some(NodeId::new("lfo")),
            },
            GraphNode {
                id: NodeId::new("left-cutoff"),
                spec: NodeSpec::Sink {
                    device: DeviceId::new("peak-left"),
                    param: ParamName::new("filter.cutoff"),
                },
                input: Some(NodeId::new("scale")),
            },
            GraphNode {
                id: NodeId::new("right-resonance"),
                spec: NodeSpec::Sink {
                    device: DeviceId::new("peak-right"),
                    param: ParamName::new("filter.resonance"),
                },
                input: Some(NodeId::new("scale")),
            },
        ],
    })
    .expect("two-device graph compiles")
}

fn endpoint(id: &str, channel: u8, capture: Arc<Mutex<CaptureSink>>) -> DeviceEndpoint {
    DeviceEndpoint {
        config: DeviceConfig {
            id: DeviceId::new(id),
            pack: "novation-peak".to_owned(),
            port: id.to_owned(),
            channel,
        },
        pack: DevicePack::from_yaml(NOVATION_PEAK_YAML).unwrap(),
        sink: Box::new(SharedSink(capture)),
    }
}

#[test]
fn multi_device_runtime_routes_each_sink_to_its_endpoint() {
    let left = Arc::new(Mutex::new(CaptureSink::new()));
    let right = Arc::new(Mutex::new(CaptureSink::new()));
    let mut runtime = Runtime::new_multi(
        two_device_graph(),
        vec![
            endpoint("peak-left", 1, Arc::clone(&left)),
            endpoint("peak-right", 2, Arc::clone(&right)),
        ],
    )
    .expect("both graph devices resolve");

    runtime.run_ticks(0, 1).unwrap();
    assert_eq!(left.lock().unwrap().messages[0], vec![0xB0, 29, 0]);
    assert_eq!(right.lock().unwrap().messages[0], vec![0xB1, 79, 0]);
}

#[test]
fn multi_device_runtime_rejects_duplicate_device_ids() {
    let first = Arc::new(Mutex::new(CaptureSink::new()));
    let second = Arc::new(Mutex::new(CaptureSink::new()));
    let err = Runtime::new_multi(
        two_device_graph(),
        vec![
            endpoint("peak-left", 1, first),
            endpoint("peak-left", 2, second),
        ],
    )
    .err()
    .expect("duplicate logical IDs are ambiguous");
    assert!(
        err.to_string()
            .contains("duplicate configured device id `peak-left`")
    );
}

#[test]
fn runtime_enforces_one_aggregate_budget_across_devices() {
    let left = Arc::new(Mutex::new(CaptureSink::new()));
    let right = Arc::new(Mutex::new(CaptureSink::new()));
    let mut runtime = Runtime::new_multi_with_config(
        two_device_graph(),
        vec![
            endpoint("peak-left", 1, left),
            endpoint("peak-right", 2, right),
        ],
        LimiterConfig {
            min_interval_ticks: 0,
        },
        BusLimiterConfig {
            ticks_per_second: 1_000,
            max_messages_per_second: 100,
            burst_messages: 1,
        },
    )
    .unwrap();

    runtime.run_ticks(0, 1_000).unwrap();
    let stats = runtime.stats();
    assert_eq!(stats.sent_messages, 100);
    assert!(stats.bus_rate_limited > 0);
    assert_eq!(
        stats.sent, stats.sent_messages,
        "both sinks use one-message CC7"
    );
}

const MULTI_MESSAGE_PACK: &str = r#"
name: multi-message-test
display_name: Multi-message test
parameters:
  wide:
    encoding: !cc14 { msb_cc: 1 }
    min: 0.0
    max: 1.0
  nrpn:
    encoding: !nrpn { msb: 2, lsb: 5 }
    min: 0.0
    max: 1.0
"#;

fn multi_message_graph() -> cr_core::CompiledGraph {
    compile(&Graph {
        nodes: vec![
            GraphNode {
                id: NodeId::new("lfo"),
                spec: NodeSpec::Lfo {
                    shape: LfoShape::Triangle,
                    freq_hz: 1.0,
                },
                input: None,
            },
            GraphNode {
                id: NodeId::new("wide"),
                spec: NodeSpec::Sink {
                    device: DeviceId::new("test"),
                    param: ParamName::new("wide"),
                },
                input: Some(NodeId::new("lfo")),
            },
            GraphNode {
                id: NodeId::new("nrpn"),
                spec: NodeSpec::Sink {
                    device: DeviceId::new("test"),
                    param: ParamName::new("nrpn"),
                },
                input: Some(NodeId::new("lfo")),
            },
        ],
    })
    .unwrap()
}

fn multi_message_endpoint(capture: Arc<Mutex<CaptureSink>>) -> DeviceEndpoint {
    DeviceEndpoint {
        config: DeviceConfig {
            id: DeviceId::new("test"),
            pack: "multi-message-test".into(),
            port: "test".into(),
            channel: 1,
        },
        pack: DevicePack::from_yaml(MULTI_MESSAGE_PACK).unwrap(),
        sink: Box::new(SharedSink(capture)),
    }
}

#[test]
fn multi_message_encodings_are_sent_as_individual_midi_messages() {
    let capture = Arc::new(Mutex::new(CaptureSink::new()));
    let mut runtime = Runtime::new_multi_with_config(
        multi_message_graph(),
        vec![multi_message_endpoint(Arc::clone(&capture))],
        LimiterConfig::default(),
        BusLimiterConfig {
            burst_messages: 6,
            ..BusLimiterConfig::default()
        },
    )
    .unwrap();

    runtime.run_ticks(0, 1).unwrap();
    let messages = &capture.lock().unwrap().messages;
    assert_eq!(messages.len(), 6, "CC14 emits two and NRPN emits four");
    assert!(messages.iter().all(|message| message.len() == 3));
    assert_eq!(messages[0], vec![0xB0, 1, 0]);
    assert_eq!(messages[1], vec![0xB0, 33, 0]);
    assert_eq!(messages[2], vec![0xB0, 99, 2]);
}

#[test]
fn runtime_rejects_a_burst_too_small_for_an_atomic_encoding() {
    let capture = Arc::new(Mutex::new(CaptureSink::new()));
    let err = Runtime::new_multi_with_config(
        multi_message_graph(),
        vec![multi_message_endpoint(capture)],
        LimiterConfig::default(),
        BusLimiterConfig {
            burst_messages: 3,
            ..BusLimiterConfig::default()
        },
    )
    .err()
    .expect("NRPN needs a four-message atomic allowance");
    let message = err.to_string();
    assert!(message.contains("midi_burst_messages=3"));
    assert!(message.contains("test.nrpn requires 4 messages"));
}

#[test]
fn graph_apply_rejects_unresolvable_sinks_without_replacing_active_graph() {
    let cfg = PatchConfig::from_yaml(include_str!("../../../config/lfo-cutoff.yaml")).unwrap();
    let graph = compile(&cfg.to_graph()).unwrap();
    let capture = Arc::new(Mutex::new(CaptureSink::new()));
    let mut runtime = Runtime::new(
        graph,
        DevicePack::from_yaml(NOVATION_PEAK_YAML).unwrap(),
        cfg.device.clone(),
        Box::new(SharedSink(Arc::clone(&capture))),
    )
    .unwrap();

    let invalid = compile(&Graph {
        nodes: vec![
            GraphNode {
                id: NodeId::new("lfo"),
                spec: NodeSpec::Lfo {
                    shape: LfoShape::Triangle,
                    freq_hz: 1.0,
                },
                input: None,
            },
            GraphNode {
                id: NodeId::new("bad"),
                spec: NodeSpec::Sink {
                    device: DeviceId::new("peak"),
                    param: ParamName::new("not.in.pack"),
                },
                input: Some(NodeId::new("lfo")),
            },
        ],
    })
    .unwrap();
    assert!(runtime.apply_graph(invalid).is_err());

    runtime.run_ticks(0, 1).unwrap();
    assert_eq!(capture.lock().unwrap().messages[0][1], 29);
}

#[test]
fn graph_apply_switches_outputs_and_resets_runtime_state() {
    let cfg = PatchConfig::from_yaml(include_str!("../../../config/lfo-cutoff.yaml")).unwrap();
    let capture = Arc::new(Mutex::new(CaptureSink::new()));
    let mut runtime = Runtime::new(
        compile(&cfg.to_graph()).unwrap(),
        DevicePack::from_yaml(NOVATION_PEAK_YAML).unwrap(),
        cfg.device.clone(),
        Box::new(SharedSink(Arc::clone(&capture))),
    )
    .unwrap();
    runtime.run_ticks(0, 1).unwrap();

    let mut replacement = cfg.to_graph();
    let sink = replacement
        .nodes
        .iter_mut()
        .find(|node| matches!(node.spec, NodeSpec::Sink { .. }))
        .unwrap();
    sink.spec = NodeSpec::Sink {
        device: DeviceId::new("peak"),
        param: ParamName::new("filter.resonance"),
    };
    runtime.apply_graph(compile(&replacement).unwrap()).unwrap();
    runtime.run_ticks(0, 1).unwrap();

    let messages = &capture.lock().unwrap().messages;
    assert_eq!(messages[0][1], 29);
    assert_eq!(messages[1][1], 79);
    assert_eq!(runtime.node_values().len(), replacement.nodes.len());
}
