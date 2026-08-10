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
    );
    rt.run_ticks(0, ticks).expect("run must not error");
    let stats = rt.stats();
    assert!(rt.unknown_params.is_empty(), "pack must cover every sink");
    (capture, stats)
}

#[test]
fn emits_cc29_on_channel_1_for_the_demo_patch() {
    let (capture, _) = run(2000);
    let c = capture.lock().unwrap();
    assert!(!c.messages.is_empty(), "expected some MIDI output");

    for m in &c.messages {
        assert_eq!(m.len(), 3, "cutoff is a 7-bit CC: 3 bytes");
        assert_eq!(m[0], 0xB0, "status: CC on channel 1 (wire 0)");
        assert_eq!(m[1], 29, "Peak filter cutoff is CC 29");
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
