pub mod nodes;

pub use nodes::{
    TICK_HZ, clamp_value, curve_value, lfo_value, math_value, quantize_value, sample_hold_value,
    step_sequence_value,
};

use cr_core::{BindingId, CompiledGraph, DeviceId, NodeSpec, ParamName};

/// Where a sink sends its value. Built once when the graph is applied, never
/// cloned during a tick.
#[derive(Debug, Clone, PartialEq)]
pub struct SinkBinding {
    pub device: DeviceId,
    pub param: ParamName,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExternalBinding {
    pub binding: BindingId,
}

#[derive(Clone, Copy, Default)]
struct TakeoverState {
    active: bool,
    previous_control: Option<f32>,
}

#[derive(Clone, Copy, Default)]
struct SlewState {
    previous: Option<f32>,
    previous_tick: Option<u64>,
}

#[derive(Clone, Copy, Default)]
enum EnvelopePhase {
    #[default]
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Copy, Default)]
struct EnvelopeState {
    phase: EnvelopePhase,
    value: f32,
    previous_gate: bool,
    previous_tick: Option<u64>,
    attack_rate: f32,
    release_rate: f32,
}

/// Evaluates a compiled graph. Holds no clock and allocates nothing per tick.
pub struct Engine {
    graph: CompiledGraph,
    values: Vec<f32>,
    /// Sink destinations, indexed by sink number. Built once in `new`.
    sinks: Vec<SinkBinding>,
    /// `(sink index, value)` produced by the most recent tick. Both fields are
    /// `Copy`, so pushing one never allocates.
    updates: Vec<(usize, f32)>,
    /// Maps node index -> sink index, so the tick needs no name lookup.
    sink_of_node: Vec<Option<usize>>,
    external_sources: Vec<ExternalBinding>,
    external_source_of_node: Vec<Option<usize>>,
    external_inputs: Vec<f32>,
    external_sinks: Vec<ExternalBinding>,
    external_sink_of_node: Vec<Option<usize>>,
    external_updates: Vec<(usize, f32)>,
    takeover: Vec<Option<TakeoverState>>,
    slew: Vec<Option<SlewState>>,
    envelope: Vec<Option<EnvelopeState>>,
}

impl Engine {
    pub fn new(graph: CompiledGraph) -> Self {
        let n = graph.len();
        let mut sinks = Vec::new();
        let mut sink_of_node = vec![None; n];
        let mut external_sources = Vec::new();
        let mut external_source_of_node = vec![None; n];
        let mut external_inputs = Vec::new();
        let mut external_sinks = Vec::new();
        let mut external_sink_of_node = vec![None; n];

        for (i, node) in graph.nodes().iter().enumerate() {
            match &node.spec {
                NodeSpec::Sink { device, param } => {
                    if let Some(slot) = sink_of_node.get_mut(i) {
                        *slot = Some(sinks.len());
                    }
                    sinks.push(SinkBinding {
                        device: device.clone(),
                        param: param.clone(),
                    });
                }
                NodeSpec::ExternalSource { binding, default } => {
                    if let Some(slot) = external_source_of_node.get_mut(i) {
                        *slot = Some(external_sources.len());
                    }
                    external_sources.push(ExternalBinding {
                        binding: binding.clone(),
                    });
                    external_inputs.push(*default);
                }
                NodeSpec::Macro { binding, default } => {
                    if let Some(slot) = external_source_of_node.get_mut(i) {
                        *slot = Some(external_sources.len());
                    }
                    external_sources.push(ExternalBinding {
                        binding: binding.clone(),
                    });
                    external_inputs.push(*default);
                }
                NodeSpec::ExternalSink { binding } => {
                    if let Some(slot) = external_sink_of_node.get_mut(i) {
                        *slot = Some(external_sinks.len());
                    }
                    external_sinks.push(ExternalBinding {
                        binding: binding.clone(),
                    });
                }
                NodeSpec::Lfo { .. }
                | NodeSpec::SampleHold { .. }
                | NodeSpec::StepSequence { .. }
                | NodeSpec::Slew { .. }
                | NodeSpec::Clamp { .. }
                | NodeSpec::Curve { .. }
                | NodeSpec::Quantize { .. }
                | NodeSpec::Math { .. }
                | NodeSpec::Envelope { .. }
                | NodeSpec::Scale { .. }
                | NodeSpec::Takeover { .. } => {}
            }
        }

        let sink_count = sinks.len();
        let takeover = graph
            .nodes()
            .iter()
            .map(|node| {
                matches!(node.spec, NodeSpec::Takeover { .. }).then_some(TakeoverState::default())
            })
            .collect();
        let slew = graph
            .nodes()
            .iter()
            .map(|node| matches!(node.spec, NodeSpec::Slew { .. }).then_some(SlewState::default()))
            .collect();
        let envelope = graph
            .nodes()
            .iter()
            .map(|node| {
                matches!(node.spec, NodeSpec::Envelope { .. }).then_some(EnvelopeState::default())
            })
            .collect();
        Self {
            graph,
            values: vec![0.0; n],
            sinks,
            updates: Vec::with_capacity(sink_count),
            sink_of_node,
            external_inputs,
            external_sources,
            external_source_of_node,
            external_updates: Vec::with_capacity(external_sinks.len()),
            external_sinks,
            external_sink_of_node,
            takeover,
            slew,
            envelope,
        }
    }

    /// Evaluate every node for the given absolute tick index.
    ///
    /// Allocation-free: `values` is sized once in `new`, and `updates` is
    /// cleared (retaining capacity) rather than reallocated.
    pub fn tick(&mut self, tick: u64) {
        self.updates.clear();
        self.external_updates.clear();

        for &i in self.graph.order() {
            // `order` and `values` are both sized from `graph.len()`, and
            // `compile()` guarantees every input index is in bounds, so these
            // lookups cannot fail. Use `get` anyway to stay panic-free.
            let Some(node) = self.graph.nodes().get(i) else {
                continue;
            };
            let input = match node.input {
                None => 0.0,
                Some(src) => self.values.get(src).copied().unwrap_or(0.0),
            };
            let control = match node.control {
                None => 0.0,
                Some(src) => self.values.get(src).copied().unwrap_or(0.0),
            };

            let out = match &node.spec {
                NodeSpec::Lfo { shape, freq_hz } => lfo_value(*shape, *freq_hz, tick),
                NodeSpec::SampleHold { rate_hz, seed } => sample_hold_value(*rate_hz, *seed, tick),
                NodeSpec::StepSequence { rate_hz, values } => {
                    step_sequence_value(*rate_hz, values, tick)
                }
                NodeSpec::Slew {
                    rise_per_second,
                    fall_per_second,
                } => {
                    let Some(Some(state)) = self.slew.get_mut(i) else {
                        continue;
                    };
                    let output = match (state.previous, state.previous_tick) {
                        (Some(previous), Some(previous_tick)) => {
                            let seconds =
                                tick.saturating_sub(previous_tick) as f32 / TICK_HZ as f32;
                            let delta = input - previous;
                            let limit = if delta >= 0.0 {
                                *rise_per_second * seconds
                            } else {
                                *fall_per_second * seconds
                            };
                            previous + delta.clamp(-limit, limit)
                        }
                        _ => input,
                    };
                    state.previous = Some(if output.is_finite() { output } else { 0.0 });
                    state.previous_tick = Some(tick);
                    state.previous.unwrap_or(0.0)
                }
                NodeSpec::Clamp { min, max } => clamp_value(input, *min, *max),
                NodeSpec::Curve { exponent } => curve_value(input, *exponent),
                NodeSpec::Quantize { steps } => quantize_value(input, *steps),
                NodeSpec::Math { operation, operand } => math_value(input, *operation, *operand),
                NodeSpec::Envelope {
                    attack_seconds,
                    decay_seconds,
                    sustain,
                    release_seconds,
                    ..
                } => {
                    let Some(Some(state)) = self.envelope.get_mut(i) else {
                        continue;
                    };
                    let gate_high = control >= 0.5;
                    if gate_high && !state.previous_gate {
                        state.phase = EnvelopePhase::Attack;
                        state.attack_rate = if *attack_seconds > 0.0 {
                            (1.0 - state.value) / *attack_seconds
                        } else {
                            f32::INFINITY
                        };
                    } else if !gate_high && state.previous_gate {
                        state.phase = EnvelopePhase::Release;
                        state.release_rate = if *release_seconds > 0.0 {
                            state.value / *release_seconds
                        } else {
                            f32::INFINITY
                        };
                    }
                    let elapsed = state
                        .previous_tick
                        .map(|previous| tick.saturating_sub(previous) as f32 / TICK_HZ as f32)
                        .unwrap_or(0.0);
                    match state.phase {
                        EnvelopePhase::Idle => state.value = 0.0,
                        EnvelopePhase::Attack => {
                            if *attack_seconds == 0.0 {
                                state.value = 1.0;
                            } else {
                                state.value = (state.value + state.attack_rate * elapsed).min(1.0);
                            }
                            if state.value >= 1.0 {
                                state.phase = EnvelopePhase::Decay;
                            }
                        }
                        EnvelopePhase::Decay => {
                            if *decay_seconds == 0.0 {
                                state.value = *sustain;
                            } else {
                                let rate = (1.0 - *sustain) / *decay_seconds;
                                state.value = (state.value - rate * elapsed).max(*sustain);
                            }
                            if state.value <= *sustain {
                                state.phase = EnvelopePhase::Sustain;
                            }
                        }
                        EnvelopePhase::Sustain => state.value = *sustain,
                        EnvelopePhase::Release => {
                            if *release_seconds == 0.0 {
                                state.value = 0.0;
                            } else {
                                state.value = (state.value - state.release_rate * elapsed).max(0.0);
                            }
                            if state.value <= 0.0 {
                                state.phase = EnvelopePhase::Idle;
                            }
                        }
                    }
                    state.previous_gate = gate_high;
                    state.previous_tick = Some(tick);
                    if state.value.is_finite() {
                        state.value
                    } else {
                        0.0
                    }
                }
                NodeSpec::Scale { min, max } => min + input.clamp(0.0, 1.0) * (max - min),
                NodeSpec::Sink { .. } => {
                    // Push only Copy data: no allocation, no string cloning.
                    if let Some(Some(sink_idx)) = self.sink_of_node.get(i) {
                        self.updates.push((*sink_idx, input));
                    }
                    input
                }
                NodeSpec::ExternalSource { default, .. } => self
                    .external_source_of_node
                    .get(i)
                    .and_then(|slot| *slot)
                    .and_then(|slot| self.external_inputs.get(slot).copied())
                    .unwrap_or(*default),
                NodeSpec::Macro { default, .. } => self
                    .external_source_of_node
                    .get(i)
                    .and_then(|slot| *slot)
                    .and_then(|slot| self.external_inputs.get(slot).copied())
                    .unwrap_or(*default),
                NodeSpec::ExternalSink { .. } => {
                    if let Some(Some(sink_idx)) = self.external_sink_of_node.get(i) {
                        self.external_updates.push((*sink_idx, input));
                    }
                    input
                }
                NodeSpec::Takeover { pickup_epsilon, .. } => {
                    let Some(Some(state)) = self.takeover.get_mut(i) else {
                        continue;
                    };
                    if !state.active
                        && let Some(previous) = state.previous_control
                        && (control - previous).abs() > f32::EPSILON
                    {
                        let crossed = (previous - input) * (control - input) <= 0.0;
                        if crossed || (control - input).abs() <= *pickup_epsilon {
                            state.active = true;
                        }
                    }
                    state.previous_control = Some(control);
                    if state.active { control } else { input }
                }
            };

            if let Some(slot) = self.values.get_mut(i) {
                *slot = out;
            }
        }
    }

    /// `(sink index, value)` pairs produced by the most recent `tick`.
    /// Resolve indices to destinations with `sink()`, outside the hot path.
    pub fn updates(&self) -> &[(usize, f32)] {
        &self.updates
    }

    /// All sink destinations, indexed by sink number.
    pub fn sinks(&self) -> &[SinkBinding] {
        &self.sinks
    }

    pub fn sink(&self, i: usize) -> Option<&SinkBinding> {
        self.sinks.get(i)
    }

    pub fn value(&self, idx: usize) -> f32 {
        self.values.get(idx).copied().unwrap_or(0.0)
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }

    pub fn external_sources(&self) -> &[ExternalBinding] {
        &self.external_sources
    }

    pub fn set_external_source(&mut self, source_idx: usize, value: f32) -> bool {
        let Some(slot) = self.external_inputs.get_mut(source_idx) else {
            return false;
        };
        if !value.is_finite() {
            return false;
        }
        *slot = value;
        true
    }

    pub fn external_sinks(&self) -> &[ExternalBinding] {
        &self.external_sinks
    }

    pub fn external_updates(&self) -> &[(usize, f32)] {
        &self.external_updates
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cr_core::{BindingId, Graph, GraphNode, LfoShape, MathOperation, NodeId, compile};

    const EPS: f32 = 1e-6;

    /// The M1 flagship chain: lfo -> scale(20..90) -> sink(filter.cutoff).
    fn flagship() -> Engine {
        let g = Graph {
            nodes: vec![
                GraphNode {
                    id: NodeId::new("lfo1"),
                    spec: NodeSpec::Lfo {
                        shape: LfoShape::Triangle,
                        freq_hz: 1.0,
                    },
                    input: None,
                },
                GraphNode {
                    id: NodeId::new("scale1"),
                    spec: NodeSpec::Scale {
                        min: 20.0,
                        max: 90.0,
                    },
                    input: Some(NodeId::new("lfo1")),
                },
                GraphNode {
                    id: NodeId::new("cutoff"),
                    spec: NodeSpec::Sink {
                        device: DeviceId::new("peak"),
                        param: ParamName::new("filter.cutoff"),
                    },
                    input: Some(NodeId::new("scale1")),
                },
            ],
        };
        Engine::new(compile(&g).expect("flagship graph must compile"))
    }

    #[test]
    fn golden_flagship_chain_values() {
        let mut e = flagship();

        // tick 0: lfo 0.0 -> scale 20.0
        e.tick(0);
        assert_eq!(e.updates().len(), 1);
        assert!((e.updates()[0].1 - 20.0).abs() < EPS);

        // tick 500: lfo 1.0 -> scale 90.0
        e.tick(500);
        assert!((e.updates()[0].1 - 90.0).abs() < EPS);

        // tick 250: lfo 0.5 -> scale 55.0
        e.tick(250);
        assert!((e.updates()[0].1 - 55.0).abs() < EPS);
    }

    #[test]
    fn sink_index_resolves_to_device_and_param() {
        let mut e = flagship();
        e.tick(0);
        let (sink_idx, _) = e.updates()[0];
        let binding = e.sink(sink_idx).expect("sink index must resolve");
        assert_eq!(binding.device, DeviceId::new("peak"));
        assert_eq!(binding.param, ParamName::new("filter.cutoff"));
    }

    #[test]
    fn updates_carry_only_copy_data() {
        // A compile-time guard on the no-allocation constraint: if the update
        // element type ever regained an owned String, this line would fail to
        // compile, since you cannot move a non-`Copy` value out of a `&[T]`
        // index. `assert_copy::<(usize, f32)>()` below only checks a
        // hardcoded literal type and would compile regardless of what
        // `Engine::updates()` actually returns; the load-bearing check is the
        // `let first: (usize, f32) = e.updates()[0];` move below.
        fn assert_copy<T: Copy>() {}
        assert_copy::<(usize, f32)>();

        let mut e = flagship();
        e.tick(0);
        let first: (usize, f32) = e.updates()[0];
        assert_eq!(first.0, 0);
    }

    #[test]
    fn tick_does_not_allocate_after_warmup() {
        let mut e = flagship();
        e.tick(0);
        let cap = e.updates.capacity();
        let vlen = e.values.len();
        let sinks = e.sinks.len();

        // A capacity comparison alone cannot catch a mutant that
        // reallocates `updates` to the *same* capacity every tick (e.g.
        // `self.updates = Vec::with_capacity(self.sinks.len())` in place of
        // `self.updates.clear()`), since the resulting capacity is
        // identical to the baseline. Capture the buffer's address after
        // warmup and assert it never moves: a reallocation always changes
        // the pointer, even when the capacity it lands on is unchanged.
        //
        // `as_ptr()` on a zero-capacity `Vec` returns a dangling alignment
        // sentinel rather than a heap address, which would make the pointer
        // check pass vacuously (the sentinel never "moves" either). Guard
        // against that: the flagship graph has one sink, so capacity must
        // be nonzero here.
        assert!(
            cap > 0,
            "test graph must have at least one sink, or the pointer check below is vacuous"
        );
        let ptr = e.updates.as_ptr();

        for t in 1..10_000u64 {
            e.tick(t);
        }

        assert_eq!(e.updates.capacity(), cap, "updates buffer reallocated");
        assert_eq!(
            e.updates.as_ptr(),
            ptr,
            "updates buffer moved (reallocated), even though capacity is unchanged"
        );
        assert_eq!(e.values.len(), vlen, "values buffer resized");
        assert_eq!(e.sinks.len(), sinks, "sink table must be built once only");
    }

    #[test]
    fn same_tick_always_yields_same_result() {
        let mut a = flagship();
        let mut b = flagship();
        a.tick(12_345);
        // Drive `b` through unrelated ticks first; history must not matter.
        for t in 0..500u64 {
            b.tick(t);
        }
        b.tick(12_345);
        assert!((a.updates()[0].1 - b.updates()[0].1).abs() < EPS);
    }

    #[test]
    fn new_sources_drive_sinks_with_golden_values() {
        let graph = compile(&Graph {
            nodes: vec![
                GraphNode {
                    id: NodeId::new("steps"),
                    spec: NodeSpec::StepSequence {
                        rate_hz: 4.0,
                        values: vec![0.0, 0.25, 0.75, 1.0],
                    },
                    input: None,
                },
                GraphNode {
                    id: NodeId::new("out"),
                    spec: NodeSpec::Sink {
                        device: DeviceId::new("peak"),
                        param: ParamName::new("filter.cutoff"),
                    },
                    input: Some(NodeId::new("steps")),
                },
            ],
        })
        .unwrap();
        let mut engine = Engine::new(graph);
        for (tick, expected) in [
            (0, 0.0),
            (249, 0.0),
            (250, 0.25),
            (500, 0.75),
            (750, 1.0),
            (1000, 0.0),
        ] {
            engine.tick(tick);
            assert!((engine.updates()[0].1 - expected).abs() < EPS);
        }

        let random = compile(&Graph {
            nodes: vec![GraphNode {
                id: NodeId::new("random"),
                spec: NodeSpec::SampleHold {
                    rate_hz: 8.0,
                    seed: 99,
                },
                input: None,
            }],
        })
        .unwrap();
        let mut first = Engine::new(random.clone());
        let mut second = Engine::new(random);
        first.tick(12_345);
        second.tick(12_345);
        assert_eq!(first.value(0), second.value(0));
    }

    #[test]
    fn transform_chain_has_golden_finite_output() {
        let graph = compile(&Graph {
            nodes: vec![
                external_source("in", "control", 0.6),
                GraphNode {
                    id: NodeId::new("curve"),
                    spec: NodeSpec::Curve { exponent: 2.0 },
                    input: Some(NodeId::new("in")),
                },
                GraphNode {
                    id: NodeId::new("quantize"),
                    spec: NodeSpec::Quantize { steps: 5 },
                    input: Some(NodeId::new("curve")),
                },
                GraphNode {
                    id: NodeId::new("math"),
                    spec: NodeSpec::Math {
                        operation: MathOperation::Multiply,
                        operand: 2.0,
                    },
                    input: Some(NodeId::new("quantize")),
                },
                GraphNode {
                    id: NodeId::new("clamp"),
                    spec: NodeSpec::Clamp { min: 0.0, max: 0.4 },
                    input: Some(NodeId::new("math")),
                },
                external_sink("out", "result", "clamp"),
            ],
        })
        .unwrap();
        let mut engine = Engine::new(graph);
        engine.tick(0);
        assert!((engine.external_updates()[0].1 - 0.4).abs() < EPS);
        assert!(engine.values().iter().all(|value| value.is_finite()));
    }

    #[test]
    fn slew_limits_rise_and_fall_by_elapsed_ticks() {
        let graph = compile(&Graph {
            nodes: vec![
                external_source("in", "control", 0.0),
                GraphNode {
                    id: NodeId::new("slew"),
                    spec: NodeSpec::Slew {
                        rise_per_second: 1.0,
                        fall_per_second: 2.0,
                    },
                    input: Some(NodeId::new("in")),
                },
                external_sink("out", "result", "slew"),
            ],
        })
        .unwrap();
        let mut engine = Engine::new(graph);
        engine.tick(0);
        assert!(engine.set_external_source(0, 1.0));
        engine.tick(100);
        assert!((engine.external_updates()[0].1 - 0.1).abs() < EPS);
        engine.tick(300);
        assert!((engine.external_updates()[0].1 - 0.3).abs() < EPS);
        assert!(engine.set_external_source(0, 0.0));
        engine.tick(400);
        assert!((engine.external_updates()[0].1 - 0.1).abs() < EPS);
    }

    #[test]
    fn envelope_runs_deterministic_adsr_from_macro_gate() {
        let graph = compile(&Graph {
            nodes: vec![
                GraphNode {
                    id: NodeId::new("gate"),
                    spec: NodeSpec::Macro {
                        binding: BindingId::new("gate"),
                        default: 0.0,
                    },
                    input: None,
                },
                GraphNode {
                    id: NodeId::new("env"),
                    spec: NodeSpec::Envelope {
                        gate: NodeId::new("gate"),
                        attack_seconds: 0.1,
                        decay_seconds: 0.1,
                        sustain: 0.5,
                        release_seconds: 0.2,
                    },
                    input: None,
                },
                external_sink("out", "envelope", "env"),
            ],
        })
        .unwrap();
        let mut engine = Engine::new(graph);
        engine.tick(0);
        assert_eq!(engine.external_updates()[0].1, 0.0);
        assert_eq!(engine.external_sources()[0].binding, BindingId::new("gate"));
        assert!(engine.set_external_source(0, 1.0));
        engine.tick(1);
        assert!((engine.external_updates()[0].1 - 0.01).abs() < EPS);
        engine.tick(100);
        assert!((engine.external_updates()[0].1 - 1.0).abs() < EPS);
        engine.tick(150);
        assert!((engine.external_updates()[0].1 - 0.75).abs() < EPS);
        engine.tick(200);
        assert!((engine.external_updates()[0].1 - 0.5).abs() < EPS);
        assert!(engine.set_external_source(0, 0.0));
        engine.tick(201);
        assert!((engine.external_updates()[0].1 - 0.4975).abs() < EPS);
        engine.tick(401);
        assert!(engine.external_updates()[0].1.abs() < EPS);
    }

    #[test]
    fn external_sources_and_sinks_use_stable_indexed_bindings() {
        let graph = compile(&Graph {
            nodes: vec![
                GraphNode {
                    id: NodeId::new("in"),
                    spec: NodeSpec::ExternalSource {
                        binding: BindingId::new("osc.in"),
                        default: 0.25,
                    },
                    input: None,
                },
                GraphNode {
                    id: NodeId::new("scale"),
                    spec: NodeSpec::Scale { min: 0.0, max: 2.0 },
                    input: Some(NodeId::new("in")),
                },
                GraphNode {
                    id: NodeId::new("out"),
                    spec: NodeSpec::ExternalSink {
                        binding: BindingId::new("osc.out"),
                    },
                    input: Some(NodeId::new("scale")),
                },
            ],
        })
        .unwrap();
        let mut engine = Engine::new(graph);
        engine.tick(0);
        assert_eq!(
            engine.external_sources()[0].binding,
            BindingId::new("osc.in")
        );
        assert_eq!(
            engine.external_sinks()[0].binding,
            BindingId::new("osc.out")
        );
        assert!((engine.external_updates()[0].1 - 0.5).abs() < EPS);

        assert!(engine.set_external_source(0, 0.75));
        assert!(!engine.set_external_source(0, f32::NAN));
        engine.tick(1);
        assert!((engine.external_updates()[0].1 - 1.5).abs() < EPS);
    }

    #[test]
    fn takeover_waits_for_a_control_crossing_then_latches() {
        let graph = compile(&Graph {
            nodes: vec![
                external_source("modulation", "modulation", 0.75),
                external_source("knob", "s88.knob.1", 0.1),
                GraphNode {
                    id: NodeId::new("pickup"),
                    spec: NodeSpec::Takeover {
                        control: NodeId::new("knob"),
                        pickup_epsilon: 0.01,
                    },
                    input: Some(NodeId::new("modulation")),
                },
                external_sink("out", "result", "pickup"),
            ],
        })
        .unwrap();
        let mut engine = Engine::new(graph);

        engine.tick(0);
        assert!((engine.external_updates()[0].1 - 0.75).abs() < EPS);

        assert!(engine.set_external_source(1, 0.6));
        engine.tick(1);
        assert!((engine.external_updates()[0].1 - 0.75).abs() < EPS);

        assert!(engine.set_external_source(1, 0.8));
        engine.tick(2);
        assert!((engine.external_updates()[0].1 - 0.8).abs() < EPS);

        assert!(engine.set_external_source(1, 0.3));
        engine.tick(3);
        assert!((engine.external_updates()[0].1 - 0.3).abs() < EPS);
    }

    #[test]
    fn stationary_control_does_not_capture_moving_modulation() {
        let graph = compile(&Graph {
            nodes: vec![
                external_source("modulation", "modulation", 0.2),
                external_source("knob", "s88.knob.1", 0.5),
                GraphNode {
                    id: NodeId::new("pickup"),
                    spec: NodeSpec::Takeover {
                        control: NodeId::new("knob"),
                        pickup_epsilon: 0.01,
                    },
                    input: Some(NodeId::new("modulation")),
                },
                external_sink("out", "result", "pickup"),
            ],
        })
        .unwrap();
        let mut engine = Engine::new(graph);
        engine.tick(0);
        assert!(engine.set_external_source(0, 0.8));
        engine.tick(1);
        assert!((engine.external_updates()[0].1 - 0.8).abs() < EPS);
    }

    fn external_source(id: &str, binding: &str, default: f32) -> GraphNode {
        GraphNode {
            id: NodeId::new(id),
            spec: NodeSpec::ExternalSource {
                binding: BindingId::new(binding),
                default,
            },
            input: None,
        }
    }

    fn external_sink(id: &str, binding: &str, input: &str) -> GraphNode {
        GraphNode {
            id: NodeId::new(id),
            spec: NodeSpec::ExternalSink {
                binding: BindingId::new(binding),
            },
            input: Some(NodeId::new(input)),
        }
    }
}
