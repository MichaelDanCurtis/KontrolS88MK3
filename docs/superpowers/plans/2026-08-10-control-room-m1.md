# Control Room M1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A software LFO modulates a real Novation Peak's filter cutoff, headless, from a config file, with no Native Instruments software running.

**Architecture:** A cargo workspace with a strict dependency DAG. `cr-core` holds types and the graph model, and is the *only* place a `CompiledGraph` can be built — validation happens there, so the engine is structurally incapable of receiving an invalid graph. `cr-engine` is a pure function of (graph, tick number) with no allocation in the tick. `cr-packs` maps semantic parameter names to wire encodings. `cr-midi` encodes bytes and enforces bus budget. `controlroom` wires it together and runs the 1 kHz clock.

**Tech Stack:** Rust 1.96 (edition 2024), midir 0.11, serde 1.0, serde_norway 0.9 (YAML), crossbeam-channel 0.5, thiserror 2.0, anyhow 1.0.

## Global Constraints

Every task's requirements implicitly include this section.

- **Rust edition 2024, resolver 3.** Toolchain verified at 1.96.0.
- **YAML uses `serde_norway`, NOT `serde_yaml`.** `serde_yaml` 0.9.34 is published as `0.9.34+deprecated` and is unmaintained. `serde_norway` is the maintained fork with the same API.
- **The engine is a pure function of (graph, inputs, tick number).** The tick count is an **input parameter**, never a clock read. No `Instant::now()`, `SystemTime`, or any clock access inside `cr-engine`. This is what makes golden tests exact and flake-free.
- **No allocation inside the tick.** All node state and output buffers are pre-allocated when a graph is applied. Output buffers use `Vec::clear()` (retains capacity), never reallocation.
- **Invalid graphs are rejected before the engine sees them.** `CompiledGraph` has private fields and no public constructor; it can only be produced by `cr_core::compile()`. This makes the spec's invariant a compile-time guarantee, not a convention.
- **Nodes are total, panic-free functions.** No indexing that can panic, no unwrap, no division without a guarded divisor.
- **MIDI is bandwidth-constrained.** DIN MIDI carries ~320 messages/second total. Every parameter has rate limiting and dedup, and **suppressed updates are counted and surfaced** — silent throttling is prohibited.
- **Float comparison in tests:** assert exact equality only on **quantized integer MIDI values**. Compare float internals with an epsilon of `1e-6`. Never assert exact float equality on LFO output across platforms.
- **Hardware verification is a manual gate (Task 10).** No automated test may be described or reported as confirming real-device behavior.

## Pre-Verified Assumptions

These were compiled and run against Rust 1.96 / edition 2024 before this plan was written, so they are confirmed rather than assumed:

- **YAML tagged-enum syntax parses under `serde_norway`** — both the pack form (`encoding: !cc7 { cc: 29 }`, `!cc14`, `!nrpn`) and the patch form (`spec: !Lfo`, `!Scale`, `!Sink`), including `#[serde(transparent)]` newtypes and `input: null`.
- **Golden LFO values are arithmetically correct** — triangle at ticks 0/250/500/750 gives 0.0/0.5/1.0/0.5; sine at 0/500 gives 0.0/1.0; no drift after 1,000,000 ticks.
- **Golden quantize values are correct** — including the 14-bit midpoint of 8192 and all degenerate-input cases returning 0.
- **The integration goldens hold** — tick 0 of the demo patch produces exactly `[0xB0, 29, 20]`, and a full 0.3 Hz cycle sweeps the quantized range 20..90.
- **Let-chains (`if let … && …`) and tuple let-else compile** under edition 2024.

What is **not** verified, and cannot be until Task 10: whether CC 29 is actually the Peak's filter cutoff. That number came from published documentation, not from the instrument.

---

## File Structure

```
Cargo.toml                              workspace manifest
crates/
  cr-core/
    src/lib.rs                          re-exports
    src/ids.rs                          NodeId, DeviceId, ParamName newtypes
    src/graph.rs                        Graph, NodeSpec, authoring model
    src/compile.rs                      validation + topo sort -> CompiledGraph
    src/error.rs                        CompileError
  cr-engine/
    src/lib.rs                          Engine, tick loop, ParamUpdate
    src/nodes.rs                        lfo/scale/sink evaluation
  cr-packs/
    src/lib.rs                          DevicePack, Encoding, PackRegistry, loader
    packs/novation-peak.yaml            shipped device pack
  cr-midi/
    src/lib.rs                          re-exports, MidiSink trait
    src/encode.rs                       CC7 / CC14 / NRPN byte encoding
    src/limiter.rs                      rate limit, dedup, drop counters
    src/output.rs                       midir-backed sink + capture sink
  controlroom/
    src/main.rs                         CLI entry
    src/config.rs                       patch config file -> Graph
    src/runtime.rs                      1 kHz tick thread
config/
  lfo-cutoff.yaml                       the M1 demo patch
```

**Why single-input nodes in M1:** every M1 node type (`lfo`, `scale`, `sink`) has at most one input, so `CompiledNode.input` is `Option<usize>`. Multi-input nodes (math, logic) arrive with later milestones and will widen this to a small inline array. This is a deliberate M1 simplification, not an oversight.

---

## Task 1: Workspace and core identifiers

**Files:**
- Create: `Cargo.toml`, `crates/cr-core/Cargo.toml`, `crates/cr-core/src/lib.rs`, `crates/cr-core/src/ids.rs`

**Interfaces:**
- Consumes: nothing
- Produces: `NodeId(String)`, `DeviceId(String)`, `ParamName(String)` — all with `new(impl Into<String>)` and `as_str() -> &str`, deriving `Debug, Clone, PartialEq, Eq, Hash`

- [ ] **Step 1: Create the workspace manifest**

Create `Cargo.toml`:

```toml
[workspace]
members = ["crates/cr-core", "crates/cr-engine", "crates/cr-packs", "crates/cr-midi", "crates/controlroom"]
resolver = "3"

[workspace.package]
edition = "2024"
rust-version = "1.96"

[workspace.dependencies]
anyhow = "1.0"
thiserror = "2.0"
serde = { version = "1.0", features = ["derive"] }
serde_norway = "0.9"
crossbeam-channel = "0.5"
midir = "0.11"
```

- [ ] **Step 2: Create the cr-core manifest**

Create `crates/cr-core/Cargo.toml`:

```toml
[package]
name = "cr-core"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true

[dependencies]
thiserror.workspace = true
serde.workspace = true
```

- [ ] **Step 3: Write the failing test**

Create `crates/cr-core/src/ids.rs`:

```rust
//! Newtype identifiers used across the graph.

use serde::{Deserialize, Serialize};

macro_rules! id_newtype {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_newtype!(NodeId);
id_newtype!(DeviceId);
id_newtype!(ParamName);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_distinct_types_with_string_access() {
        let n = NodeId::new("lfo1");
        assert_eq!(n.as_str(), "lfo1");
        assert_eq!(n, NodeId::new("lfo1"));
        assert_ne!(NodeId::new("a"), NodeId::new("b"));
    }
}
```

Create `crates/cr-core/src/lib.rs`:

```rust
pub mod ids;

pub use ids::{DeviceId, NodeId, ParamName};
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p cr-core`
Expected: PASS, 1 test.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/cr-core
git commit -m "feat(core): workspace scaffold and identifier newtypes"
```

---

## Task 2: Graph model and the compile invariant

This task establishes the plan's most important guarantee: `CompiledGraph` cannot be constructed except through validation.

**Files:**
- Create: `crates/cr-core/src/graph.rs`, `crates/cr-core/src/compile.rs`, `crates/cr-core/src/error.rs`
- Modify: `crates/cr-core/src/lib.rs`

**Interfaces:**
- Consumes: `NodeId`, `DeviceId`, `ParamName` from Task 1
- Produces:
  - `enum LfoShape { Triangle, Sine }`
  - `enum NodeSpec { Lfo { shape: LfoShape, freq_hz: f64 }, Scale { min: f32, max: f32 }, Sink { device: DeviceId, param: ParamName } }`
  - `struct GraphNode { pub id: NodeId, pub spec: NodeSpec, pub input: Option<NodeId> }`
  - `struct Graph { pub nodes: Vec<GraphNode> }`
  - `struct CompiledNode { pub spec: NodeSpec, pub input: Option<usize> }`
  - `struct CompiledGraph` with `nodes() -> &[CompiledNode]`, `order() -> &[usize]`, `len() -> usize` — **private fields, no public constructor**
  - `fn compile(graph: &Graph) -> Result<CompiledGraph, CompileError>`
  - `enum CompileError { DuplicateNode, UnknownInput, Cycle, EmptyGraph, InvalidLfoFreq, InvalidScaleRange }`

- [ ] **Step 1: Write the failing tests**

Create `crates/cr-core/src/error.rs`:

```rust
use crate::NodeId;
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum CompileError {
    #[error("graph is empty")]
    EmptyGraph,
    #[error("duplicate node id: {0}")]
    DuplicateNode(NodeId),
    #[error("node {node} references unknown input {input}")]
    UnknownInput { node: NodeId, input: NodeId },
    #[error("graph contains a cycle involving {0}")]
    Cycle(NodeId),
    #[error("node {node} has invalid lfo frequency {freq} (must be > 0 and finite)")]
    InvalidLfoFreq { node: NodeId, freq: f64 },
    #[error("node {node} has invalid scale range: min {min} max {max}")]
    InvalidScaleRange { node: NodeId, min: f32, max: f32 },
}
```

Create `crates/cr-core/src/graph.rs`:

```rust
//! The authoring model: what a user or the UI describes.

use crate::{DeviceId, NodeId, ParamName};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LfoShape {
    Triangle,
    Sine,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeSpec {
    Lfo { shape: LfoShape, freq_hz: f64 },
    Scale { min: f32, max: f32 },
    Sink { device: DeviceId, param: ParamName },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: NodeId,
    pub spec: NodeSpec,
    pub input: Option<NodeId>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
}
```

Create `crates/cr-core/src/compile.rs`:

```rust
//! Validation and topological ordering.
//!
//! `CompiledGraph` has private fields and no public constructor, so the only
//! way to obtain one is through `compile()`. The engine therefore cannot be
//! handed an invalid graph.

use crate::error::CompileError;
use crate::graph::{Graph, NodeSpec};
use crate::NodeId;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledNode {
    pub spec: NodeSpec,
    pub input: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledGraph {
    nodes: Vec<CompiledNode>,
    order: Vec<usize>,
}

impl CompiledGraph {
    pub fn nodes(&self) -> &[CompiledNode] {
        &self.nodes
    }
    pub fn order(&self) -> &[usize] {
        &self.order
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

pub fn compile(graph: &Graph) -> Result<CompiledGraph, CompileError> {
    if graph.nodes.is_empty() {
        return Err(CompileError::EmptyGraph);
    }

    let mut index: HashMap<&NodeId, usize> = HashMap::new();
    for (i, n) in graph.nodes.iter().enumerate() {
        if index.insert(&n.id, i).is_some() {
            return Err(CompileError::DuplicateNode(n.id.clone()));
        }
    }

    let mut nodes = Vec::with_capacity(graph.nodes.len());
    for n in &graph.nodes {
        match &n.spec {
            NodeSpec::Lfo { freq_hz, .. } => {
                if !freq_hz.is_finite() || *freq_hz <= 0.0 {
                    return Err(CompileError::InvalidLfoFreq {
                        node: n.id.clone(),
                        freq: *freq_hz,
                    });
                }
            }
            NodeSpec::Scale { min, max } => {
                if !min.is_finite() || !max.is_finite() || min >= max {
                    return Err(CompileError::InvalidScaleRange {
                        node: n.id.clone(),
                        min: *min,
                        max: *max,
                    });
                }
            }
            NodeSpec::Sink { .. } => {}
        }

        let input = match &n.input {
            None => None,
            Some(src) => Some(*index.get(src).ok_or_else(|| CompileError::UnknownInput {
                node: n.id.clone(),
                input: src.clone(),
            })?),
        };

        nodes.push(CompiledNode {
            spec: n.spec.clone(),
            input,
        });
    }

    let order = topo_order(&nodes, &graph.nodes)?;
    Ok(CompiledGraph { nodes, order })
}

/// Depth-first topological sort. Each node has at most one input in M1, so
/// this walks a forest of chains; `state` guards against cycles.
fn topo_order(
    nodes: &[CompiledNode],
    original: &[crate::graph::GraphNode],
) -> Result<Vec<usize>, CompileError> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Unvisited,
        InProgress,
        Done,
    }

    let mut state = vec![Mark::Unvisited; nodes.len()];
    let mut order = Vec::with_capacity(nodes.len());

    for start in 0..nodes.len() {
        // Iterative walk up the input chain, then emit on the way back down.
        let mut stack: Vec<usize> = Vec::new();
        let mut cursor = Some(start);

        while let Some(i) = cursor {
            match state[i] {
                Mark::Done => break,
                Mark::InProgress => {
                    return Err(CompileError::Cycle(original[i].id.clone()));
                }
                Mark::Unvisited => {
                    state[i] = Mark::InProgress;
                    stack.push(i);
                    cursor = nodes[i].input;
                }
            }
        }

        while let Some(i) = stack.pop() {
            state[i] = Mark::Done;
            order.push(i);
        }
    }

    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{GraphNode, LfoShape};
    use crate::{DeviceId, ParamName};

    fn lfo(id: &str) -> GraphNode {
        GraphNode {
            id: NodeId::new(id),
            spec: NodeSpec::Lfo {
                shape: LfoShape::Triangle,
                freq_hz: 1.0,
            },
            input: None,
        }
    }

    fn scale(id: &str, input: &str) -> GraphNode {
        GraphNode {
            id: NodeId::new(id),
            spec: NodeSpec::Scale {
                min: 0.0,
                max: 10.0,
            },
            input: Some(NodeId::new(input)),
        }
    }

    fn sink(id: &str, input: &str) -> GraphNode {
        GraphNode {
            id: NodeId::new(id),
            spec: NodeSpec::Sink {
                device: DeviceId::new("peak"),
                param: ParamName::new("filter.cutoff"),
            },
            input: Some(NodeId::new(input)),
        }
    }

    #[test]
    fn compiles_chain_in_dependency_order() {
        // Deliberately authored out of order: sink, scale, lfo.
        let g = Graph {
            nodes: vec![sink("s", "sc"), scale("sc", "l"), lfo("l")],
        };
        let c = compile(&g).expect("should compile");
        assert_eq!(c.len(), 3);

        // The lfo (index 2) must be evaluated before scale (1) before sink (0).
        let pos = |idx: usize| c.order().iter().position(|&x| x == idx).unwrap();
        assert!(pos(2) < pos(1), "lfo must precede scale");
        assert!(pos(1) < pos(0), "scale must precede sink");
    }

    #[test]
    fn rejects_empty_graph() {
        assert_eq!(compile(&Graph::default()), Err(CompileError::EmptyGraph));
    }

    #[test]
    fn rejects_duplicate_node_id() {
        let g = Graph {
            nodes: vec![lfo("l"), lfo("l")],
        };
        assert_eq!(
            compile(&g),
            Err(CompileError::DuplicateNode(NodeId::new("l")))
        );
    }

    #[test]
    fn rejects_unknown_input() {
        let g = Graph {
            nodes: vec![scale("sc", "nope")],
        };
        assert_eq!(
            compile(&g),
            Err(CompileError::UnknownInput {
                node: NodeId::new("sc"),
                input: NodeId::new("nope"),
            })
        );
    }

    #[test]
    fn rejects_cycle() {
        let g = Graph {
            nodes: vec![scale("a", "b"), scale("b", "a")],
        };
        assert!(matches!(compile(&g), Err(CompileError::Cycle(_))));
    }

    #[test]
    fn rejects_non_positive_lfo_freq() {
        let g = Graph {
            nodes: vec![GraphNode {
                id: NodeId::new("l"),
                spec: NodeSpec::Lfo {
                    shape: LfoShape::Triangle,
                    freq_hz: 0.0,
                },
                input: None,
            }],
        };
        assert!(matches!(compile(&g), Err(CompileError::InvalidLfoFreq { .. })));
    }

    #[test]
    fn rejects_inverted_scale_range() {
        let g = Graph {
            nodes: vec![GraphNode {
                id: NodeId::new("sc"),
                spec: NodeSpec::Scale {
                    min: 10.0,
                    max: 1.0,
                },
                input: None,
            }],
        };
        assert!(matches!(
            compile(&g),
            Err(CompileError::InvalidScaleRange { .. })
        ));
    }
}
```

- [ ] **Step 2: Wire up the module exports**

Replace `crates/cr-core/src/lib.rs`:

```rust
pub mod compile;
pub mod error;
pub mod graph;
pub mod ids;

pub use compile::{compile, CompiledGraph, CompiledNode};
pub use error::CompileError;
pub use graph::{Graph, GraphNode, LfoShape, NodeSpec};
pub use ids::{DeviceId, NodeId, ParamName};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p cr-core`
Expected: PASS, 8 tests.

- [ ] **Step 4: Verify the invariant actually holds**

Confirm `CompiledGraph` cannot be built outside `compile()`. Add this to `crates/cr-core/src/compile.rs` tests and confirm it compiles (it should — this documents the accessors) :

```rust
    #[test]
    fn compiled_graph_exposes_read_only_view() {
        let g = Graph {
            nodes: vec![lfo("l")],
        };
        let c = compile(&g).unwrap();
        assert_eq!(c.nodes().len(), 1);
        assert_eq!(c.order(), &[0]);
        assert!(!c.is_empty());
    }
```

Run: `cargo test -p cr-core`
Expected: PASS, 9 tests.

Then confirm by inspection that `CompiledGraph`'s fields are private and there is no `pub fn new`. A downstream crate writing `CompiledGraph { nodes: vec![], order: vec![] }` must fail to compile.

- [ ] **Step 5: Commit**

```bash
git add crates/cr-core
git commit -m "feat(core): graph model with validation-only CompiledGraph construction"
```

---

## Task 3: Deterministic LFO evaluation

**Files:**
- Create: `crates/cr-engine/Cargo.toml`, `crates/cr-engine/src/lib.rs`, `crates/cr-engine/src/nodes.rs`

**Interfaces:**
- Consumes: `LfoShape` from Task 2
- Produces:
  - `const TICK_HZ: f64 = 1000.0`
  - `fn lfo_value(shape: LfoShape, freq_hz: f64, tick: u64) -> f32`

**Scope note:** `struct Engine` is **Task 4's** deliverable, not this one. Task 3
creates the crate and the pure LFO function only.

**Why the tick is a parameter:** phase is computed from the absolute tick index, so there is no accumulated drift and no clock read. The same tick number always yields the same value, which is what makes the golden tests below exact.

- [ ] **Step 1: Create the manifest**

Create `crates/cr-engine/Cargo.toml`:

```toml
[package]
name = "cr-engine"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true

[dependencies]
cr-core = { path = "../cr-core" }
```

- [ ] **Step 2: Write the failing test**

Create `crates/cr-engine/src/nodes.rs`:

```rust
//! Node evaluation. Every function here is total: no panics, no unwraps,
//! no unguarded division, and no clock access.

use cr_core::LfoShape;

/// The engine's fixed control rate.
pub const TICK_HZ: f64 = 1000.0;

/// Compute an LFO output in `0.0..=1.0` from the absolute tick index.
///
/// Phase derives from the tick number rather than accumulating, so there is
/// no drift and the result is reproducible for any tick.
pub fn lfo_value(shape: LfoShape, freq_hz: f64, tick: u64) -> f32 {
    // `compile()` rejects non-positive and non-finite frequencies, but this
    // function stays total regardless of how it is called.
    if !freq_hz.is_finite() || freq_hz <= 0.0 {
        return 0.0;
    }
    let phase = ((tick as f64) * freq_hz / TICK_HZ).rem_euclid(1.0);
    let v = match shape {
        LfoShape::Triangle => {
            if phase < 0.5 {
                2.0 * phase
            } else {
                2.0 - 2.0 * phase
            }
        }
        LfoShape::Sine => 0.5 - 0.5 * (std::f64::consts::TAU * phase).cos(),
    };
    v.clamp(0.0, 1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-6;

    #[test]
    fn triangle_hits_known_points_of_one_cycle() {
        // 1 Hz at 1 kHz => one full cycle per 1000 ticks.
        assert!((lfo_value(LfoShape::Triangle, 1.0, 0) - 0.0).abs() < EPS);
        assert!((lfo_value(LfoShape::Triangle, 1.0, 250) - 0.5).abs() < EPS);
        assert!((lfo_value(LfoShape::Triangle, 1.0, 500) - 1.0).abs() < EPS);
        assert!((lfo_value(LfoShape::Triangle, 1.0, 750) - 0.5).abs() < EPS);
    }

    #[test]
    fn triangle_is_periodic_without_drift() {
        // The same phase 1000 cycles later must be identical, proving the
        // absolute-tick formulation does not accumulate error.
        let a = lfo_value(LfoShape::Triangle, 1.0, 250);
        let b = lfo_value(LfoShape::Triangle, 1.0, 250 + 1_000_000);
        assert!((a - b).abs() < EPS);
    }

    #[test]
    fn sine_hits_known_points_of_one_cycle() {
        assert!((lfo_value(LfoShape::Sine, 1.0, 0) - 0.0).abs() < EPS);
        assert!((lfo_value(LfoShape::Sine, 1.0, 500) - 1.0).abs() < EPS);
    }

    #[test]
    fn output_always_within_unit_range() {
        for tick in 0..5000u64 {
            let v = lfo_value(LfoShape::Sine, 0.3, tick);
            assert!((0.0..=1.0).contains(&v), "out of range at {tick}: {v}");
        }
    }

    #[test]
    fn invalid_frequency_returns_zero_without_panicking() {
        assert_eq!(lfo_value(LfoShape::Triangle, 0.0, 10), 0.0);
        assert_eq!(lfo_value(LfoShape::Triangle, -1.0, 10), 0.0);
        assert_eq!(lfo_value(LfoShape::Triangle, f64::NAN, 10), 0.0);
        assert_eq!(lfo_value(LfoShape::Triangle, f64::INFINITY, 10), 0.0);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p cr-engine`
Expected: FAIL — `crates/cr-engine/src/lib.rs` does not exist yet.

- [ ] **Step 4: Create the lib entry point**

Create `crates/cr-engine/src/lib.rs`:

```rust
pub mod nodes;

pub use nodes::{lfo_value, TICK_HZ};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p cr-engine`
Expected: PASS, 5 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/cr-engine
git commit -m "feat(engine): deterministic drift-free LFO derived from tick index"
```

---

## Task 4: Engine evaluation with no allocation in the tick

**Files:**
- Modify: `crates/cr-engine/src/lib.rs`

**Interfaces:**
- Consumes: `lfo_value`, `TICK_HZ` from Task 3; `CompiledGraph`, `NodeSpec` from Task 2
- Produces:
  - `struct SinkBinding { pub device: DeviceId, pub param: ParamName }`
  - `struct Engine` with `new(CompiledGraph)`, `tick(&mut self, tick: u64)`, `updates(&self) -> &[(usize, f32)]`, `sinks(&self) -> &[SinkBinding]`, `sink(&self, i: usize) -> Option<&SinkBinding>`, `value(&self, idx: usize) -> f32`

**Why updates are `(usize, f32)` and not owned names:** `DeviceId` and `ParamName`
wrap `String`, so cloning them into a per-tick update struct would heap-allocate
twice per sink per tick — 2000 allocations/second at 1 kHz — violating the
no-allocation constraint. Instead `Engine::new` builds the `SinkBinding` table
once, and the tick emits `(sink_index, value)` pairs, which are `Copy`.
Consumers resolve names through `sink(i)` outside the hot path.

- [ ] **Step 1: Write the failing test**

Replace `crates/cr-engine/src/lib.rs`:

```rust
pub mod nodes;

pub use nodes::{lfo_value, TICK_HZ};

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
                sink_of_node[i] = Some(sinks.len());
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
    use cr_core::{compile, Graph, GraphNode, LfoShape, NodeId};

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
        // type ever regains an owned String, `Copy` stops being satisfied and
        // this fails to build.
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

        for t in 1..10_000u64 {
            e.tick(t);
        }

        assert_eq!(e.updates.capacity(), cap, "updates buffer reallocated");
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
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p cr-engine`
Expected: PASS, 9 tests total (5 from Task 3, 4 new).

- [ ] **Step 3: Commit**

```bash
git add crates/cr-engine
git commit -m "feat(engine): allocation-free graph evaluation with golden chain tests"
```

---

## Task 5: Device packs and the Novation Peak pack

**Files:**
- Create: `crates/cr-packs/Cargo.toml`, `crates/cr-packs/src/lib.rs`, `crates/cr-packs/packs/novation-peak.yaml`

**Interfaces:**
- Consumes: nothing from earlier tasks
- Produces:
  - `enum Encoding { Cc7 { cc: u8 }, Cc14 { msb_cc: u8 }, Nrpn { msb: u8, lsb: u8 } }`
  - `struct Parameter { pub encoding: Encoding, pub min: f32, pub max: f32 }`
  - `struct DevicePack { pub name: String, pub display_name: String, pub parameters: HashMap<String, Parameter> }`
  - `impl DevicePack { fn from_yaml(&str) -> Result<Self, PackError>; fn parameter(&self, &str) -> Option<&Parameter> }`
  - `enum PackError { Yaml, InvalidRange, InvalidCc, Cc14LsbOverflow, NoParameters }`

**Source of the CC numbers:** the published Novation Summit/Peak MIDI map. Filter cutoff CC 29, resonance CC 79, amp envelope attack/decay/sustain/release CC 86/87/88/89, LFO 1 rate CC 30. **These are treated as unverified until Task 10 confirms them on the instrument.**

- [ ] **Step 1: Create the manifest**

Create `crates/cr-packs/Cargo.toml`:

```toml
[package]
name = "cr-packs"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true

[dependencies]
serde.workspace = true
serde_norway.workspace = true
thiserror.workspace = true
```

- [ ] **Step 2: Write the pack file**

Create `crates/cr-packs/packs/novation-peak.yaml`:

```yaml
# Novation Peak.
# CC numbers taken from the published Summit/Peak MIDI map.
# STATUS: verify against hardware (see plan Task 10). Do not treat as
# confirmed until an instrument has responded to each one.
name: novation-peak
display_name: Novation Peak
parameters:
  filter.cutoff:
    encoding: !cc7 { cc: 29 }
    min: 0.0
    max: 127.0
  filter.resonance:
    encoding: !cc7 { cc: 79 }
    min: 0.0
    max: 127.0
  amp_env.attack:
    encoding: !cc7 { cc: 86 }
    min: 0.0
    max: 127.0
  amp_env.decay:
    encoding: !cc7 { cc: 87 }
    min: 0.0
    max: 127.0
  amp_env.sustain:
    encoding: !cc7 { cc: 88 }
    min: 0.0
    max: 127.0
  amp_env.release:
    encoding: !cc7 { cc: 89 }
    min: 0.0
    max: 127.0
  lfo1.rate:
    encoding: !cc7 { cc: 30 }
    min: 0.0
    max: 127.0
```

- [ ] **Step 3: Write the failing test**

Create `crates/cr-packs/src/lib.rs`:

```rust
//! Semantic device models: named parameters mapped to wire encodings.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PackError {
    #[error("failed to parse pack yaml: {0}")]
    Yaml(#[from] serde_norway::Error),
    #[error("parameter {name} has invalid range: min {min} max {max}")]
    InvalidRange { name: String, min: f32, max: f32 },
    #[error("parameter {name} has out-of-range cc number {cc} (must be 0-127)")]
    InvalidCc { name: String, cc: u8 },
    // A 14-bit pair puts the LSB at msb_cc + 32. Reusing InvalidCc here would
    // print "must be 0-127" for a value that IS inside 0-127, naming a bound
    // the input does not violate and hiding the real +32 offset rule.
    #[error(
        "parameter {name} uses 14-bit cc {msb_cc}, whose LSB would land on cc {lsb} \
         (exceeds 127); msb_cc must be <= 95"
    )]
    Cc14LsbOverflow { name: String, msb_cc: u8, lsb: u16 },
    #[error("pack has no parameters")]
    NoParameters,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    /// Single 7-bit control change.
    Cc7 { cc: u8 },
    /// 14-bit pair: MSB on `msb_cc`, LSB on `msb_cc + 32`.
    Cc14 { msb_cc: u8 },
    /// Non-registered parameter number, 14-bit data entry.
    Nrpn { msb: u8, lsb: u8 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub encoding: Encoding,
    pub min: f32,
    pub max: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevicePack {
    pub name: String,
    pub display_name: String,
    pub parameters: HashMap<String, Parameter>,
}

impl DevicePack {
    pub fn from_yaml(src: &str) -> Result<Self, PackError> {
        let pack: DevicePack = serde_norway::from_str(src)?;
        pack.validate()?;
        Ok(pack)
    }

    pub fn parameter(&self, name: &str) -> Option<&Parameter> {
        self.parameters.get(name)
    }

    fn validate(&self) -> Result<(), PackError> {
        if self.parameters.is_empty() {
            return Err(PackError::NoParameters);
        }
        for (name, p) in &self.parameters {
            if !p.min.is_finite() || !p.max.is_finite() || p.min >= p.max {
                return Err(PackError::InvalidRange {
                    name: name.clone(),
                    min: p.min,
                    max: p.max,
                });
            }
            let ccs: &[u8] = match &p.encoding {
                Encoding::Cc7 { cc } => &[*cc],
                Encoding::Cc14 { msb_cc } => &[*msb_cc],
                Encoding::Nrpn { msb, lsb } => &[*msb, *lsb],
            };
            for &cc in ccs {
                if cc > 127 {
                    return Err(PackError::InvalidCc {
                        name: name.clone(),
                        cc,
                    });
                }
            }
            // A 14-bit CC pair puts the LSB at cc+32, which must also fit.
            if let Encoding::Cc14 { msb_cc } = &p.encoding
                && *msb_cc > 95
            {
                return Err(PackError::Cc14LsbOverflow {
                    name: name.clone(),
                    msb_cc: *msb_cc,
                    lsb: *msb_cc as u16 + 32,
                });
            }
        }
        Ok(())
    }
}

/// The Peak pack shipped with the binary.
pub const NOVATION_PEAK_YAML: &str = include_str!("../packs/novation-peak.yaml");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_peak_pack_loads_and_validates() {
        let pack = DevicePack::from_yaml(NOVATION_PEAK_YAML).expect("shipped pack must load");
        assert_eq!(pack.name, "novation-peak");
        assert_eq!(pack.display_name, "Novation Peak");
    }

    #[test]
    fn peak_pack_exposes_documented_cc_numbers() {
        let pack = DevicePack::from_yaml(NOVATION_PEAK_YAML).unwrap();
        let cutoff = pack.parameter("filter.cutoff").expect("cutoff present");
        assert_eq!(cutoff.encoding, Encoding::Cc7 { cc: 29 });

        let attack = pack.parameter("amp_env.attack").expect("attack present");
        assert_eq!(attack.encoding, Encoding::Cc7 { cc: 86 });

        let release = pack.parameter("amp_env.release").expect("release present");
        assert_eq!(release.encoding, Encoding::Cc7 { cc: 89 });
    }

    #[test]
    fn unknown_parameter_returns_none() {
        let pack = DevicePack::from_yaml(NOVATION_PEAK_YAML).unwrap();
        assert!(pack.parameter("no.such.param").is_none());
    }

    #[test]
    fn rejects_inverted_range() {
        let src = r#"
name: bad
display_name: Bad
parameters:
  a.b:
    encoding: !cc7 { cc: 1 }
    min: 10.0
    max: 1.0
"#;
        assert!(matches!(
            DevicePack::from_yaml(src),
            Err(PackError::InvalidRange { .. })
        ));
    }

    #[test]
    fn rejects_cc14_whose_lsb_would_overflow() {
        let src = r#"
name: bad
display_name: Bad
parameters:
  a.b:
    encoding: !cc14 { msb_cc: 100 }
    min: 0.0
    max: 1.0
"#;
        // 100 passes the generic `cc > 127` check, so this genuinely exercises
        // the Cc14-specific bound rather than the shared one.
        assert!(matches!(
            DevicePack::from_yaml(src),
            Err(PackError::Cc14LsbOverflow { msb_cc: 100, lsb: 132, .. })
        ));
    }

    #[test]
    fn rejects_pack_with_no_parameters() {
        let src = "name: empty\ndisplay_name: Empty\nparameters: {}\n";
        assert!(matches!(
            DevicePack::from_yaml(src),
            Err(PackError::NoParameters)
        ));
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p cr-packs`
Expected: PASS, 6 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/cr-packs
git commit -m "feat(packs): device pack schema and Novation Peak pack"
```

---

## Task 6: Byte-exact MIDI encoding

**Files:**
- Create: `crates/cr-midi/Cargo.toml`, `crates/cr-midi/src/lib.rs`, `crates/cr-midi/src/encode.rs`

**Interfaces:**
- Consumes: `Encoding` from Task 5
- Produces:
  - `fn quantize(value: f32, min: f32, max: f32, bits: u8) -> u16`
  - `fn encode(enc: &Encoding, channel: u8, raw: u16, out: &mut Vec<u8>)`
  - `fn resolution_bits(enc: &Encoding) -> u8`

**Encoding reference (standard MIDI):**
- 7-bit CC: `[0xB0 | ch, cc, value]`
- 14-bit CC: MSB on `cc`, LSB on `cc + 32` — two messages
- NRPN: CC 99 = param MSB, CC 98 = param LSB, CC 6 = data MSB, CC 38 = data LSB

- [ ] **Step 1: Create the manifest**

Create `crates/cr-midi/Cargo.toml`:

```toml
[package]
name = "cr-midi"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true

[dependencies]
cr-packs = { path = "../cr-packs" }
midir.workspace = true
thiserror.workspace = true
```

- [ ] **Step 2: Write the failing test**

Create `crates/cr-midi/src/encode.rs`:

```rust
//! Wire encoding. Every function is total and appends to a caller-owned
//! buffer so the hot path never allocates.

use cr_packs::Encoding;

/// Bit depth a given encoding transmits.
pub fn resolution_bits(enc: &Encoding) -> u8 {
    match enc {
        Encoding::Cc7 { .. } => 7,
        Encoding::Cc14 { .. } | Encoding::Nrpn { .. } => 14,
    }
}

/// Map a real value in `min..=max` onto an unsigned integer of `bits` width.
pub fn quantize(value: f32, min: f32, max: f32, bits: u8) -> u16 {
    let max_raw: u16 = if bits >= 14 { 16383 } else { 127 };
    if !value.is_finite() || !min.is_finite() || !max.is_finite() || min >= max {
        return 0;
    }
    let t = ((value - min) / (max - min)).clamp(0.0, 1.0);
    // `round` then clamp keeps the result inside the representable range even
    // for float edge cases.
    let scaled = (t * max_raw as f32).round();
    scaled.clamp(0.0, max_raw as f32) as u16
}

/// Append the MIDI bytes for `raw` on `channel` (0-15) to `out`.
pub fn encode(enc: &Encoding, channel: u8, raw: u16, out: &mut Vec<u8>) {
    let ch = channel & 0x0F;
    let status = 0xB0 | ch;
    match enc {
        Encoding::Cc7 { cc } => {
            out.extend_from_slice(&[status, cc & 0x7F, (raw & 0x7F) as u8]);
        }
        Encoding::Cc14 { msb_cc } => {
            let msb = ((raw >> 7) & 0x7F) as u8;
            let lsb = (raw & 0x7F) as u8;
            out.extend_from_slice(&[status, msb_cc & 0x7F, msb]);
            out.extend_from_slice(&[status, (msb_cc.wrapping_add(32)) & 0x7F, lsb]);
        }
        Encoding::Nrpn { msb, lsb } => {
            let data_msb = ((raw >> 7) & 0x7F) as u8;
            let data_lsb = (raw & 0x7F) as u8;
            out.extend_from_slice(&[status, 99, msb & 0x7F]);
            out.extend_from_slice(&[status, 98, lsb & 0x7F]);
            out.extend_from_slice(&[status, 6, data_msb]);
            out.extend_from_slice(&[status, 38, data_lsb]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cc7_is_three_bytes_on_the_right_channel() {
        let mut out = Vec::new();
        encode(&Encoding::Cc7 { cc: 29 }, 0, 64, &mut out);
        assert_eq!(out, vec![0xB0, 29, 64]);

        out.clear();
        encode(&Encoding::Cc7 { cc: 29 }, 15, 127, &mut out);
        assert_eq!(out, vec![0xBF, 29, 127]);
    }

    #[test]
    fn cc14_emits_msb_then_lsb_at_offset_32() {
        let mut out = Vec::new();
        // 16383 == all 14 bits set => msb 127, lsb 127
        encode(&Encoding::Cc14 { msb_cc: 1 }, 0, 16383, &mut out);
        assert_eq!(out, vec![0xB0, 1, 127, 0xB0, 33, 127]);

        out.clear();
        // 128 == msb 1, lsb 0
        encode(&Encoding::Cc14 { msb_cc: 1 }, 0, 128, &mut out);
        assert_eq!(out, vec![0xB0, 1, 1, 0xB0, 33, 0]);
    }

    #[test]
    fn nrpn_emits_param_then_data_in_standard_order() {
        let mut out = Vec::new();
        encode(&Encoding::Nrpn { msb: 2, lsb: 5 }, 0, 300, &mut out);
        // 300 = 0b100101100 -> data msb 2, data lsb 44
        assert_eq!(
            out,
            vec![
                0xB0, 99, 2, // NRPN param MSB
                0xB0, 98, 5, // NRPN param LSB
                0xB0, 6, 2, // data entry MSB
                0xB0, 38, 44, // data entry LSB
            ]
        );
    }

    #[test]
    fn quantize_covers_full_7bit_range() {
        assert_eq!(quantize(0.0, 0.0, 127.0, 7), 0);
        assert_eq!(quantize(127.0, 0.0, 127.0, 7), 127);
        assert_eq!(quantize(63.5, 0.0, 127.0, 7), 64);
    }

    #[test]
    fn quantize_covers_full_14bit_range() {
        assert_eq!(quantize(0.0, 0.0, 1.0, 14), 0);
        assert_eq!(quantize(1.0, 0.0, 1.0, 14), 16383);
        assert_eq!(quantize(0.5, 0.0, 1.0, 14), 8192);
    }

    #[test]
    fn quantize_clamps_out_of_range_input() {
        assert_eq!(quantize(-50.0, 0.0, 127.0, 7), 0);
        assert_eq!(quantize(500.0, 0.0, 127.0, 7), 127);
    }

    #[test]
    fn quantize_is_total_for_degenerate_input() {
        assert_eq!(quantize(f32::NAN, 0.0, 1.0, 7), 0);
        assert_eq!(quantize(0.5, 1.0, 1.0, 7), 0);
        assert_eq!(quantize(0.5, f32::INFINITY, 1.0, 7), 0);
    }

    #[test]
    fn no_encoded_data_byte_ever_sets_the_status_bit() {
        // A data byte >= 0x80 would be interpreted as a new status byte and
        // would corrupt the stream.
        let encodings = [
            Encoding::Cc7 { cc: 29 },
            Encoding::Cc14 { msb_cc: 1 },
            Encoding::Nrpn { msb: 2, lsb: 5 },
        ];
        for enc in &encodings {
            for raw in [0u16, 1, 127, 128, 8192, 16383] {
                let mut out = Vec::new();
                encode(enc, 0, raw, &mut out);
                for (i, b) in out.iter().enumerate() {
                    if i % 3 == 0 {
                        assert!(*b >= 0x80, "byte {i} should be a status byte");
                    } else {
                        assert!(*b < 0x80, "data byte {i} has status bit set: {b:#04x}");
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 3: Create the lib entry point**

Create `crates/cr-midi/src/lib.rs`:

```rust
pub mod encode;

pub use encode::{encode, quantize, resolution_bits};
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p cr-midi`
Expected: PASS, 8 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/cr-midi
git commit -m "feat(midi): byte-exact CC7, 14-bit CC, and NRPN encoding"
```

---

## Task 7: Rate limiting, dedup, and visible drop counts

The spec forbids silent throttling. This task makes suppression observable.

**Files:**
- Create: `crates/cr-midi/src/limiter.rs`
- Modify: `crates/cr-midi/src/lib.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks
- Produces:
  - `struct LimiterConfig { pub min_interval_ticks: u64 }` with `Default` of 10 (100 Hz per parameter at a 1 kHz tick)
  - `struct Limiter` with `new(LimiterConfig)`, `admit(&mut self, key: usize, raw: u16, tick: u64) -> bool`, `stats(&self) -> LimiterStats`, `reset(&mut self, slots: usize)`
  - `struct LimiterStats { pub sent: u64, pub deduped: u64, pub rate_limited: u64 }`

**Budget rationale:** DIN MIDI carries roughly 320 three-byte messages per second. The 10-tick default caps a single parameter at 100 messages/second, so three continuously-moving parameters approach the bus ceiling. Exceeding it does not corrupt output — it inflates `rate_limited`, which the runtime reports.

- [ ] **Step 1: Write the failing test**

Create `crates/cr-midi/src/limiter.rs`:

```rust
//! Per-parameter send budget.
//!
//! Two independent suppression reasons are tracked separately because they
//! mean different things to a user: `deduped` means the value genuinely did
//! not change, while `rate_limited` means real movement was discarded to fit
//! the bus budget. The latter is the number that should worry someone.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimiterConfig {
    /// Minimum ticks between two sends for the same parameter.
    pub min_interval_ticks: u64,
}

impl Default for LimiterConfig {
    fn default() -> Self {
        // 10 ticks at 1 kHz => 100 messages/second/parameter.
        Self {
            min_interval_ticks: 10,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LimiterStats {
    pub sent: u64,
    pub deduped: u64,
    pub rate_limited: u64,
}

#[derive(Debug, Clone, Copy)]
struct SlotState {
    last_raw: Option<u16>,
    last_sent_tick: Option<u64>,
}

pub struct Limiter {
    config: LimiterConfig,
    slots: Vec<SlotState>,
    stats: LimiterStats,
}

impl Limiter {
    pub fn new(config: LimiterConfig) -> Self {
        Self {
            config,
            slots: Vec::new(),
            stats: LimiterStats::default(),
        }
    }

    /// Size the slot table. Call once per graph apply, never inside the tick.
    pub fn reset(&mut self, slots: usize) {
        self.slots = vec![
            SlotState {
                last_raw: None,
                last_sent_tick: None,
            };
            slots
        ];
        self.stats = LimiterStats::default();
    }

    /// Decide whether a parameter update should reach the wire.
    ///
    /// Returns `true` to send. Allocation-free.
    pub fn admit(&mut self, key: usize, raw: u16, tick: u64) -> bool {
        let Some(slot) = self.slots.get_mut(key) else {
            // Unknown slot: fail closed rather than flooding the bus.
            return false;
        };

        if slot.last_raw == Some(raw) {
            self.stats.deduped += 1;
            return false;
        }

        if let Some(last) = slot.last_sent_tick
            && tick.saturating_sub(last) < self.config.min_interval_ticks
        {
            self.stats.rate_limited += 1;
            return false;
        }

        slot.last_raw = Some(raw);
        slot.last_sent_tick = Some(tick);
        self.stats.sent += 1;
        true
    }

    pub fn stats(&self) -> LimiterStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(interval: u64) -> Limiter {
        let mut l = Limiter::new(LimiterConfig {
            min_interval_ticks: interval,
        });
        l.reset(1);
        l
    }

    #[test]
    fn first_update_is_always_admitted() {
        let mut l = limiter(10);
        assert!(l.admit(0, 64, 0));
        assert_eq!(l.stats().sent, 1);
    }

    #[test]
    fn identical_value_is_deduped_not_rate_limited() {
        let mut l = limiter(0);
        assert!(l.admit(0, 64, 0));
        assert!(!l.admit(0, 64, 100));
        assert_eq!(l.stats().deduped, 1);
        assert_eq!(l.stats().rate_limited, 0);
    }

    #[test]
    fn changed_value_too_soon_is_rate_limited_and_counted() {
        let mut l = limiter(10);
        assert!(l.admit(0, 64, 0));
        assert!(!l.admit(0, 65, 5), "5 ticks < 10 tick interval");
        assert_eq!(l.stats().rate_limited, 1);
        assert_eq!(l.stats().deduped, 0);
    }

    #[test]
    fn changed_value_after_interval_is_admitted() {
        let mut l = limiter(10);
        assert!(l.admit(0, 64, 0));
        assert!(l.admit(0, 65, 10));
        assert_eq!(l.stats().sent, 2);
    }

    #[test]
    fn suppressed_values_are_never_silently_lost() {
        // Every rejected admit must land in exactly one counter, so
        // sent + deduped + rate_limited accounts for every call.
        let mut l = limiter(10);
        let mut calls = 0u64;
        for tick in 0..1000u64 {
            l.admit(0, (tick % 128) as u16, tick);
            calls += 1;
        }
        let s = l.stats();
        assert_eq!(s.sent + s.deduped + s.rate_limited, calls);
    }

    #[test]
    fn parameters_have_independent_budgets() {
        let mut l = Limiter::new(LimiterConfig {
            min_interval_ticks: 10,
        });
        l.reset(2);
        assert!(l.admit(0, 1, 0));
        assert!(l.admit(1, 1, 0), "second parameter has its own budget");
    }

    #[test]
    fn unknown_slot_fails_closed() {
        let mut l = limiter(10);
        assert!(!l.admit(99, 1, 0));
        assert_eq!(l.stats().sent, 0);
    }
}
```

- [ ] **Step 2: Update the lib entry point**

Replace `crates/cr-midi/src/lib.rs`:

```rust
pub mod encode;
pub mod limiter;

pub use encode::{encode, quantize, resolution_bits};
pub use limiter::{Limiter, LimiterConfig, LimiterStats};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p cr-midi`
Expected: PASS, 15 tests.

- [ ] **Step 4: Commit**

```bash
git add crates/cr-midi
git commit -m "feat(midi): per-parameter rate limiting with observable drop counts"
```

---

## Task 8: MIDI output sinks

**Files:**
- Create: `crates/cr-midi/src/output.rs`
- Modify: `crates/cr-midi/src/lib.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks
- Produces:
  - `trait MidiSink { fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError>; }`
  - `struct CaptureSink { pub messages: Vec<Vec<u8>> }` implementing `MidiSink`
  - `struct MidirSink` with `open(port_substring: &str) -> Result<Self, MidiError>` and `list_ports() -> Result<Vec<String>, MidiError>`
  - `enum MidiError { Init, NoMatchingPort, Connect, Send }`

- [ ] **Step 1: Write the failing test**

Create `crates/cr-midi/src/output.rs`:

```rust
//! Output sinks. `CaptureSink` makes the whole pipeline testable without
//! hardware; `MidirSink` talks to a real port.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MidiError {
    #[error("could not initialise midi backend: {0}")]
    Init(String),
    #[error("no midi output port matching {0:?}; available: {1:?}")]
    NoMatchingPort(String, Vec<String>),
    #[error("could not connect to midi port: {0}")]
    Connect(String),
    #[error("could not send midi bytes: {0}")]
    Send(String),
}

pub trait MidiSink {
    fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError>;
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

impl MidirSink {
    pub fn list_ports() -> Result<Vec<String>, MidiError> {
        let out = midir::MidiOutput::new("control-room")
            .map_err(|e| MidiError::Init(e.to_string()))?;
        Ok(out
            .ports()
            .iter()
            .filter_map(|p| out.port_name(p).ok())
            .collect())
    }

    /// Open the first output port whose name contains `port_substring`
    /// (case-insensitive).
    pub fn open(port_substring: &str) -> Result<Self, MidiError> {
        let out = midir::MidiOutput::new("control-room")
            .map_err(|e| MidiError::Init(e.to_string()))?;
        let needle = port_substring.to_lowercase();

        let ports = out.ports();
        let mut names = Vec::with_capacity(ports.len());
        let mut found = None;
        for p in &ports {
            let name = out.port_name(p).unwrap_or_default();
            if found.is_none() && name.to_lowercase().contains(&needle) {
                found = Some(p.clone());
            }
            names.push(name);
        }

        let port = found.ok_or_else(|| {
            MidiError::NoMatchingPort(port_substring.to_string(), names)
        })?;
        let conn = out
            .connect(&port, "control-room-out")
            .map_err(|e| MidiError::Connect(e.to_string()))?;
        Ok(Self { conn })
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
}
```

- [ ] **Step 2: Update the lib entry point**

Replace `crates/cr-midi/src/lib.rs`:

```rust
pub mod encode;
pub mod limiter;
pub mod output;

pub use encode::{encode, quantize, resolution_bits};
pub use limiter::{Limiter, LimiterConfig, LimiterStats};
pub use output::{CaptureSink, MidiError, MidiSink, MidirSink};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p cr-midi`
Expected: PASS, 19 tests.

- [ ] **Step 4: Commit**

```bash
git add crates/cr-midi
git commit -m "feat(midi): midir output sink and capture sink for testing"
```

---

## Task 9: The binary — config, wiring, and the 1 kHz runtime

**Files:**
- Create: `crates/controlroom/Cargo.toml`, `crates/controlroom/src/main.rs`, `crates/controlroom/src/config.rs`, `crates/controlroom/src/runtime.rs`, `config/lfo-cutoff.yaml`

**Interfaces:**
- Consumes: everything above
- Produces:
  - `struct PatchConfig { pub device: DeviceConfig, pub nodes: Vec<GraphNode> }`
  - `struct DeviceConfig { pub id: DeviceId, pub port: String, pub channel: u8 }`
  - `impl PatchConfig { fn from_yaml(&str) -> anyhow::Result<Self>; fn to_graph(&self) -> Graph }`
  - `struct Runtime` with `new(CompiledGraph, DevicePack, DeviceConfig, Box<dyn MidiSink>) -> Runtime`, `fn run_ticks(&mut self, from: u64, count: u64) -> anyhow::Result<()>`, `fn stats(&self) -> LimiterStats`

**Design note:** `run_ticks` takes an explicit tick range rather than reading a clock, so the entire pipeline is testable end-to-end without real time passing. `main` supplies wall-clock pacing; the runtime itself stays deterministic.

- [ ] **Step 1: Create the manifest**

Create `crates/controlroom/Cargo.toml`:

```toml
[package]
name = "controlroom"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true

[dependencies]
cr-core = { path = "../cr-core" }
cr-engine = { path = "../cr-engine" }
cr-midi = { path = "../cr-midi" }
cr-packs = { path = "../cr-packs" }
anyhow.workspace = true
serde.workspace = true
serde_norway.workspace = true
```

- [ ] **Step 2: Write the demo patch config**

Create `config/lfo-cutoff.yaml`:

```yaml
# The M1 demo patch: a slow triangle LFO sweeping the Peak's filter cutoff.
# Cutoff is the verification target because the result is unmistakably audible.
device:
  id: peak
  port: Peak          # case-insensitive substring match on the output port name
  channel: 1          # 1-16 as printed on the instrument
nodes:
  - id: lfo1
    spec: !Lfo
      shape: triangle
      freq_hz: 0.3
    input: null
  - id: scale1
    spec: !Scale
      min: 20.0
      max: 90.0
    input: lfo1
  - id: cutoff
    spec: !Sink
      device: peak
      param: filter.cutoff
    input: scale1
```

- [ ] **Step 3: Write the config loader with its failing test**

Create `crates/controlroom/src/config.rs`:

```rust
//! Patch config files: the authoring surface until the browser UI exists.

use cr_core::{DeviceId, Graph, GraphNode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub id: DeviceId,
    pub port: String,
    /// MIDI channel as printed on hardware, 1-16.
    pub channel: u8,
}

impl DeviceConfig {
    /// Convert the user-facing 1-16 channel to the 0-15 wire value.
    pub fn wire_channel(&self) -> u8 {
        self.channel.saturating_sub(1).min(15)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchConfig {
    pub device: DeviceConfig,
    pub nodes: Vec<GraphNode>,
}

impl PatchConfig {
    pub fn from_yaml(src: &str) -> anyhow::Result<Self> {
        Ok(serde_norway::from_str(src)?)
    }

    pub fn to_graph(&self) -> Graph {
        Graph {
            nodes: self.nodes.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cr_core::compile;

    const DEMO: &str = include_str!("../../../config/lfo-cutoff.yaml");

    #[test]
    fn demo_patch_parses() {
        let cfg = PatchConfig::from_yaml(DEMO).expect("demo config must parse");
        assert_eq!(cfg.device.id, DeviceId::new("peak"));
        assert_eq!(cfg.device.channel, 1);
        assert_eq!(cfg.nodes.len(), 3);
    }

    #[test]
    fn demo_patch_compiles() {
        let cfg = PatchConfig::from_yaml(DEMO).unwrap();
        let compiled = compile(&cfg.to_graph()).expect("demo graph must compile");
        assert_eq!(compiled.len(), 3);
    }

    #[test]
    fn channel_converts_from_one_based_to_wire() {
        let d = DeviceConfig {
            id: DeviceId::new("x"),
            port: "p".into(),
            channel: 1,
        };
        assert_eq!(d.wire_channel(), 0);

        let d16 = DeviceConfig {
            channel: 16,
            ..d.clone()
        };
        assert_eq!(d16.wire_channel(), 15);

        // Out-of-range input must not panic or wrap.
        let d0 = DeviceConfig { channel: 0, ..d };
        assert_eq!(d0.wire_channel(), 0);
    }
}
```

- [ ] **Step 4: Write the runtime with its failing test**

Create `crates/controlroom/src/runtime.rs`:

```rust
//! Wires engine output through the pack, limiter, and encoder to a sink.

use crate::config::DeviceConfig;
use cr_core::CompiledGraph;
use cr_engine::Engine;
use cr_midi::{encode, quantize, resolution_bits, Limiter, LimiterConfig, LimiterStats, MidiSink};
use cr_packs::{DevicePack, Parameter};

/// A sink resolved against the device pack once, at construction.
struct ResolvedSink {
    param: Parameter,
    bits: u8,
}

pub struct Runtime {
    engine: Engine,
    device: DeviceConfig,
    sink: Box<dyn MidiSink>,
    limiter: Limiter,
    /// Indexed by the engine's sink index. `None` means the graph referenced a
    /// parameter the pack does not define.
    resolved: Vec<Option<ResolvedSink>>,
    /// Scratch buffer reused every tick so encoding never allocates.
    scratch: Vec<u8>,
    /// Parameter names referenced by the graph but absent from the pack.
    /// Collected once at construction, never during a tick.
    pub unknown_params: Vec<String>,
}

impl Runtime {
    pub fn new(
        graph: CompiledGraph,
        pack: DevicePack,
        device: DeviceConfig,
        sink: Box<dyn MidiSink>,
    ) -> Self {
        let engine = Engine::new(graph);

        // Resolve every sink against the pack up front. After this, the tick
        // does no name lookup, no hashing, and no allocation.
        let mut resolved = Vec::with_capacity(engine.sinks().len());
        let mut unknown_params = Vec::new();
        for binding in engine.sinks() {
            match pack.parameter(binding.param.as_str()) {
                Some(p) => resolved.push(Some(ResolvedSink {
                    param: p.clone(),
                    bits: resolution_bits(&p.encoding),
                })),
                None => {
                    unknown_params.push(binding.param.as_str().to_string());
                    resolved.push(None);
                }
            }
        }

        // One limiter slot per sink, so budgets are per graph sink.
        let mut limiter = Limiter::new(LimiterConfig::default());
        limiter.reset(resolved.len());

        Self {
            engine,
            device,
            sink,
            limiter,
            resolved,
            scratch: Vec::with_capacity(16),
            unknown_params,
        }
    }

    /// Run `count` ticks starting at `from`. Deterministic: reads no clock.
    pub fn run_ticks(&mut self, from: u64, count: u64) -> anyhow::Result<()> {
        for tick in from..from.saturating_add(count) {
            self.engine.tick(tick);

            for &(sink_idx, value) in self.engine.updates() {
                let Some(Some(r)) = self.resolved.get(sink_idx) else {
                    // Unknown parameter: reported via `unknown_params`.
                    continue;
                };

                let raw = quantize(value, r.param.min, r.param.max, r.bits);

                if !self.limiter.admit(sink_idx, raw, tick) {
                    continue;
                }

                self.scratch.clear();
                encode(
                    &r.param.encoding,
                    self.device.wire_channel(),
                    raw,
                    &mut self.scratch,
                );
                self.sink.send(&self.scratch)?;
            }
        }
        Ok(())
    }

    pub fn stats(&self) -> LimiterStats {
        self.limiter.stats()
    }
}
```

- [ ] **Step 5: Write the end-to-end integration test**

Create `crates/controlroom/tests/pipeline.rs`:

```rust
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
    let values: Vec<u8> = c.messages.iter().filter_map(|m| m.get(2).copied()).collect();
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
        stats.sent + stats.deduped + stats.rate_limited,
        ticks,
        "suppression must never be silent"
    );
}
```

- [ ] **Step 6: Add the lib target before writing the binary**

The integration test in Step 5 imports `controlroom::config`, so the package
needs a library target as well as a binary. Create `crates/controlroom/src/lib.rs`:

```rust
pub mod config;
pub mod runtime;
```

Then append to `crates/controlroom/Cargo.toml`:

```toml
[lib]
name = "controlroom"
path = "src/lib.rs"

[[bin]]
name = "controlroom"
path = "src/main.rs"
```

- [ ] **Step 7: Write main.rs**

Create `crates/controlroom/src/main.rs`:

```rust
use anyhow::{Context, Result};
use controlroom::{config, runtime};
use cr_core::compile;
use cr_midi::{CaptureSink, MidiSink, MidirSink};
use cr_packs::{DevicePack, NOVATION_PEAK_YAML};
use std::time::{Duration, Instant};

/// Milliseconds per tick. The engine runs at 1 kHz.
const TICK_MS: u64 = 1;
/// Ticks per pacing batch. Sleeping once per batch rather than once per tick
/// keeps timing steady without burning a core.
const BATCH: u64 = 10;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    let mut dry_run = false;
    let mut list = false;

    while let Some(a) = args.next() {
        match a.as_str() {
            "--dry-run" => dry_run = true,
            "--list-ports" => list = true,
            other => path = Some(other.to_string()),
        }
    }

    if list {
        for name in MidirSink::list_ports()? {
            println!("{name}");
        }
        return Ok(());
    }

    let path = path.unwrap_or_else(|| "config/lfo-cutoff.yaml".to_string());
    let src = std::fs::read_to_string(&path)
        .with_context(|| format!("reading patch config {path}"))?;
    let cfg = config::PatchConfig::from_yaml(&src)
        .with_context(|| format!("parsing patch config {path}"))?;
    let graph = compile(&cfg.to_graph()).context("compiling patch graph")?;
    let pack = DevicePack::from_yaml(NOVATION_PEAK_YAML).context("loading device pack")?;

    let sink: Box<dyn MidiSink> = if dry_run {
        println!("dry run: no MIDI port will be opened");
        Box::new(CaptureSink::new())
    } else {
        println!("opening MIDI port matching {:?}", cfg.device.port);
        Box::new(MidirSink::open(&cfg.device.port)?)
    };

    let mut rt = runtime::Runtime::new(graph, pack, cfg.device.clone(), sink);
    println!("running {path} at 1 kHz; ctrl-c to stop");

    let start = Instant::now();
    let mut tick: u64 = 0;
    loop {
        rt.run_ticks(tick, BATCH)?;
        tick += BATCH;

        if !rt.unknown_params.is_empty() {
            anyhow::bail!(
                "patch references parameters missing from the pack: {:?}",
                rt.unknown_params
            );
        }

        // Report the bus budget every 5 seconds. Suppression is never silent.
        if tick % 5000 == 0 {
            let s = rt.stats();
            println!(
                "t={tick} sent={} deduped={} rate_limited={}",
                s.sent, s.deduped, s.rate_limited
            );
        }

        // Pace against elapsed wall time so drift does not accumulate.
        // `Duration::from_millis` takes u64, so this stays correct for an
        // appliance running for months. (`TICK * tick as u32` would overflow
        // after roughly 49 days.)
        let target = start + Duration::from_millis(tick * TICK_MS);
        if let Some(delay) = target.checked_duration_since(Instant::now()) {
            std::thread::sleep(delay);
        }
    }
}
```

- [ ] **Step 8: Run the whole suite**

Run: `cargo test --workspace`
Expected: PASS. All unit tests plus 5 integration tests in `pipeline.rs`.

- [ ] **Step 9: Verify the binary runs without hardware**

Run: `cargo run -p controlroom -- --dry-run config/lfo-cutoff.yaml`
Expected: prints the dry-run notice, then periodic `sent=… deduped=… rate_limited=…` lines. Stop with ctrl-c.

Run: `cargo run -p controlroom -- --list-ports`
Expected: prints available MIDI output port names (possibly none).

- [ ] **Step 10: Check lints and formatting**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings.

Run: `cargo fmt --all`

Note: `cargo fmt --all` formats the whole workspace, which is correct here since every file is new in this plan.

- [ ] **Step 11: Commit**

```bash
git add crates/controlroom config
git commit -m "feat(controlroom): patch config, deterministic runtime, and 1 kHz binary"
```

---

## Task 10: Hardware verification gate (MANUAL — cannot be automated)

**This task is performed by a human with the instrument connected. No automated test may substitute for it, and its result must never be inferred from a passing suite.**

Everything up to here proves the software is internally consistent. It proves *nothing* about whether CC 29 is really the Peak's filter cutoff, because that fact came from published documentation rather than from the instrument.

**Files:**
- Modify: `crates/cr-packs/packs/novation-peak.yaml` (status comment only, after verification)
- Create: `docs/verification/2026-08-10-peak-m1.md`

- [ ] **Step 1: Confirm the port is visible**

Connect the Peak by USB or DIN. Run:

```bash
cargo run -p controlroom -- --list-ports
```

Expected: a port whose name contains "Peak". If absent, stop — nothing below can pass. If the name differs, update `port:` in `config/lfo-cutoff.yaml`.

- [ ] **Step 2: Confirm the MIDI channel**

On the Peak, check the global MIDI receive channel and make `channel:` in `config/lfo-cutoff.yaml` match it (1-16 as printed).

- [ ] **Step 3: Set up an audible test**

On the Peak: load a patch with a sustained, bright sound (a saw pad works well), and set the filter cutoff roughly to the middle by hand. You must be able to hear cutoff changes clearly.

- [ ] **Step 4: Run the patch and listen**

```bash
cargo run -p controlroom -- config/lfo-cutoff.yaml
```

Expected: the filter sweeps up and down roughly every 3.3 seconds (0.3 Hz), and the Peak's own cutoff display or knob position tracks the sweep.

- [ ] **Step 5: Record the outcome honestly**

Create `docs/verification/2026-08-10-peak-m1.md` recording, for each item, PASS or FAIL with what was actually observed:

```markdown
# Peak M1 Hardware Verification

Date:
Firmware version:
Connection (USB / DIN):
MIDI channel:

| Check | Result | Observed |
|---|---|---|
| Port appears in --list-ports | | |
| CC 29 moves filter cutoff | | |
| Sweep period is ~3.3s at 0.3 Hz | | |
| Sweep spans the configured 20-90 range | | |
| No stuck or jittery values | | |
| No audible stepping from rate limiting | | |

Unverified parameters (still documentation-derived, not confirmed):
- filter.resonance (CC 79)
- amp_env.attack/decay/sustain/release (CC 86/87/88/89)
- lfo1.rate (CC 30)

Notes:
```

- [ ] **Step 6: If cutoff did NOT respond, diagnose in this order**

1. Wrong channel — try `channel: 1` through `16`, or set the Peak to omni.
2. Wrong CC — the published map may be wrong for this firmware. Confirm by moving the Peak's own cutoff knob while monitoring its MIDI output with an external monitor, and read the CC number it transmits. Update the pack to match, and note in the pack comment that the value is **hardware-derived** rather than documentation-derived.
3. Peak ignoring CC — check its global settings for CC receive enabled.

- [ ] **Step 7: Update the pack status comment**

Only for parameters actually confirmed, change the pack's header comment from `STATUS: verify against hardware` to a specific record, for example:

```yaml
# STATUS: filter.cutoff (CC 29) confirmed on hardware 2026-08-10, firmware <version>.
# All other parameters remain documentation-derived and UNVERIFIED.
```

Do not mark parameters verified that were not individually tested.

- [ ] **Step 8: Commit**

```bash
git add docs/verification crates/cr-packs/packs/novation-peak.yaml config
git commit -m "docs: record Peak M1 hardware verification results"
```

---

## M1 Completion Criteria

M1 is done when all of the following hold:

1. `cargo test --workspace` passes.
2. `cargo clippy --workspace --all-targets -- -D warnings` is clean.
3. `cargo run -p controlroom -- --dry-run config/lfo-cutoff.yaml` runs and reports limiter stats.
4. **Task 10 is complete and `docs/verification/2026-08-10-peak-m1.md` records a real PASS for the cutoff sweep**, observed by a human hearing the instrument.

Criterion 4 is the one that matters. The first three can all pass while the software does nothing audible.

---

## Self-Review Notes

**Spec coverage** — every M1 requirement in the design spec maps to a task:

| Spec requirement | Task |
|---|---|
| Engine pure in (graph, inputs, tick), tick as input | 3, 4 |
| No allocation in tick | 4 (test), 9 (scratch buffer) |
| Invalid graphs rejected before engine | 2 (private-field `CompiledGraph`) |
| Nodes total and panic-free | 3, 4 (degenerate-input tests) |
| Semantic device packs, YAML, Peak first | 5 |
| 14-bit CC and NRPN encoding | 6 |
| Per-parameter rate limiting and dedup | 7 |
| Drop rate surfaced, never silent | 7 (test), 9 (periodic report) |
| Golden-file engine tests | 4 |
| Byte-exact MIDI tests | 6, 9 |
| Hardware verification as manual gate | 10 |

**Deferred to later milestones, deliberately:** the WebSocket command API and browser UI (M2), OSC (M3), S88 surface (M4), Pi packaging (M5). M1 authors patches through a config file, which is why `PatchConfig` exists.

**Known M1 simplifications, each documented at its site:** single-input nodes; no runtime graph edits (the engine is built once at startup, so the tick-boundary command queue arrives with M2); a global rather than per-device limiter budget.
