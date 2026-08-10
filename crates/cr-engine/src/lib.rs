pub mod nodes;

pub use nodes::{TICK_HZ, lfo_value};

use cr_core::{CompiledGraph, DeviceId, NodeSpec, ParamName};

/// Where a sink sends its value. Built once when the graph is applied, never
/// cloned during a tick.
#[derive(Debug, Clone, PartialEq)]
pub struct SinkBinding {
    pub device: DeviceId,
    pub param: ParamName,
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
}

impl Engine {
    pub fn new(graph: CompiledGraph) -> Self {
        let n = graph.len();
        let mut sinks = Vec::new();
        let mut sink_of_node = vec![None; n];

        for (i, node) in graph.nodes().iter().enumerate() {
            if let NodeSpec::Sink { device, param } = &node.spec {
                if let Some(slot) = sink_of_node.get_mut(i) {
                    *slot = Some(sinks.len());
                }
                sinks.push(SinkBinding {
                    device: device.clone(),
                    param: param.clone(),
                });
            }
        }

        let sink_count = sinks.len();
        Self {
            graph,
            values: vec![0.0; n],
            sinks,
            updates: Vec::with_capacity(sink_count),
            sink_of_node,
        }
    }

    /// Evaluate every node for the given absolute tick index.
    ///
    /// Allocation-free: `values` is sized once in `new`, and `updates` is
    /// cleared (retaining capacity) rather than reallocated.
    pub fn tick(&mut self, tick: u64) {
        self.updates.clear();

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

            let out = match &node.spec {
                NodeSpec::Lfo { shape, freq_hz } => lfo_value(*shape, *freq_hz, tick),
                NodeSpec::Scale { min, max } => min + input.clamp(0.0, 1.0) * (max - min),
                NodeSpec::Sink { .. } => {
                    // Push only Copy data: no allocation, no string cloning.
                    if let Some(Some(sink_idx)) = self.sink_of_node.get(i) {
                        self.updates.push((*sink_idx, input));
                    }
                    input
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use cr_core::{Graph, GraphNode, LfoShape, NodeId, compile};

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
}
