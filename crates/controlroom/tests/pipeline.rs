//! End-to-end: config file -> graph -> engine -> pack -> limiter -> bytes.
//! Runs with no hardware attached.

use controlroom::config::PatchConfig;
use controlroom::runtime::Runtime;
use cr_core::compile;
use cr_midi::{CaptureSink, MidiSink};
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

fn run(ticks: u64) -> (Arc<Mutex<CaptureSink>>, cr_midi::LimiterStats) {
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
        stats.sent + stats.deduped + stats.rate_limited + stats.unknown_slot,
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
