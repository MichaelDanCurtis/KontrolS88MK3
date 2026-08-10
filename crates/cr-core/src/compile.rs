//! Validation and topological ordering.
//!
//! `CompiledGraph` has private fields and no public constructor, so the only
//! way to obtain one is through `compile()`. The engine therefore cannot be
//! handed an invalid graph.

use crate::NodeId;
use crate::error::CompileError;
use crate::graph::{Graph, NodeSpec};
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
    fn rejects_self_loop() {
        // A node whose `input` is itself: the degenerate one-node cycle.
        let g = Graph {
            nodes: vec![scale("a", "a")],
        };
        assert_eq!(compile(&g), Err(CompileError::Cycle(NodeId::new("a"))));
    }

    #[test]
    fn compiles_fan_out_with_shared_upstream_before_both_dependents() {
        // Two sinks reading the same upstream lfo node.
        let g = Graph {
            nodes: vec![lfo("l"), sink("s1", "l"), sink("s2", "l")],
        };
        let c = compile(&g).expect("fan-out must compile");

        let order = c.order();
        assert_eq!(order.len(), 3, "every node must appear in the order");
        let mut sorted = order.to_vec();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            vec![0, 1, 2],
            "every node must appear exactly once, with no gaps"
        );

        let pos = |idx: usize| order.iter().position(|&x| x == idx).unwrap();
        assert!(pos(0) < pos(1), "shared upstream must precede first sink");
        assert!(pos(0) < pos(2), "shared upstream must precede second sink");
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
        assert!(matches!(
            compile(&g),
            Err(CompileError::InvalidLfoFreq { .. })
        ));
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
}
