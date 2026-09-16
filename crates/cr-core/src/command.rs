//! Transport-agnostic commands for editing an authored graph.
//!
//! Commands are applied to a staged copy and the resulting graph is compiled
//! before it replaces the caller's graph. A rejected command or an invalid
//! result therefore never leaves a partially edited graph behind.

use crate::{CompileError, CompiledGraph, Graph, GraphNode, NodeId, NodeSpec, compile};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A single graph-authoring operation.
///
/// Node identifiers are stable: changing an identifier is represented as a
/// remove followed by an add. Connections are edited independently from node
/// specifications so every operation has unambiguous semantics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GraphCommand {
    AddNode { id: NodeId, spec: NodeSpec },
    RemoveNode { id: NodeId },
    UpdateNode { id: NodeId, spec: NodeSpec },
    Connect { source: NodeId, target: NodeId },
    Disconnect { target: NodeId },
}

#[derive(Debug, Error, PartialEq)]
pub enum GraphCommandError {
    #[error("node already exists: {0}")]
    NodeAlreadyExists(NodeId),
    #[error("node does not exist: {0}")]
    NodeNotFound(NodeId),
    #[error("connection source does not exist: {0}")]
    SourceNodeNotFound(NodeId),
    #[error("connection target does not exist: {0}")]
    TargetNodeNotFound(NodeId),
    #[error("node {target} is already connected to {connected_to}")]
    InputAlreadyConnected {
        target: NodeId,
        connected_to: NodeId,
    },
    #[error("node is not connected: {0}")]
    InputNotConnected(NodeId),
    #[error("edited graph is invalid: {0}")]
    InvalidGraph(#[from] CompileError),
}

/// Apply one command atomically and return the already-validated compiled graph.
pub fn apply_command(
    graph: &mut Graph,
    command: GraphCommand,
) -> Result<CompiledGraph, GraphCommandError> {
    apply_commands(graph, std::iter::once(command))
}

/// Apply a sequence of commands as one transaction.
///
/// Intermediate states do not have to compile. This permits operations such as
/// replacing the only node with a remove/add pair while still guaranteeing that
/// the final state is valid. If any command or final validation fails, `graph`
/// remains byte-for-byte equivalent to its value before this call.
pub fn apply_commands(
    graph: &mut Graph,
    commands: impl IntoIterator<Item = GraphCommand>,
) -> Result<CompiledGraph, GraphCommandError> {
    let mut staged = graph.clone();

    for command in commands {
        apply_unchecked(&mut staged, command)?;
    }

    let compiled = compile(&staged)?;
    *graph = staged;
    Ok(compiled)
}

fn apply_unchecked(graph: &mut Graph, command: GraphCommand) -> Result<(), GraphCommandError> {
    match command {
        GraphCommand::AddNode { id, spec } => {
            if graph.nodes.iter().any(|node| node.id == id) {
                return Err(GraphCommandError::NodeAlreadyExists(id));
            }
            graph.nodes.push(GraphNode {
                id,
                spec,
                input: None,
            });
        }
        GraphCommand::RemoveNode { id } => {
            let index = graph
                .nodes
                .iter()
                .position(|node| node.id == id)
                .ok_or_else(|| GraphCommandError::NodeNotFound(id.clone()))?;
            graph.nodes.remove(index);

            // Removing a node also removes its incident connections, matching
            // the behavior expected by graph editors and other control clients.
            for node in &mut graph.nodes {
                if node.input.as_ref() == Some(&id) {
                    node.input = None;
                }
            }
        }
        GraphCommand::UpdateNode { id, spec } => {
            find_node_mut(graph, &id)?.spec = spec;
        }
        GraphCommand::Connect { source, target } => {
            if !graph.nodes.iter().any(|node| node.id == source) {
                return Err(GraphCommandError::SourceNodeNotFound(source));
            }

            let target_node = graph
                .nodes
                .iter_mut()
                .find(|node| node.id == target)
                .ok_or_else(|| GraphCommandError::TargetNodeNotFound(target.clone()))?;
            if let Some(existing) = &target_node.input {
                return Err(GraphCommandError::InputAlreadyConnected {
                    target,
                    connected_to: existing.clone(),
                });
            }
            target_node.input = Some(source);
        }
        GraphCommand::Disconnect { target } => {
            let target_node = find_node_mut(graph, &target)?;
            if target_node.input.take().is_none() {
                return Err(GraphCommandError::InputNotConnected(target));
            }
        }
    }

    Ok(())
}

fn find_node_mut<'a>(
    graph: &'a mut Graph,
    id: &NodeId,
) -> Result<&'a mut GraphNode, GraphCommandError> {
    graph
        .nodes
        .iter_mut()
        .find(|node| node.id == *id)
        .ok_or_else(|| GraphCommandError::NodeNotFound(id.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LfoShape;

    fn lfo_spec(freq_hz: f64) -> NodeSpec {
        NodeSpec::Lfo {
            shape: LfoShape::Triangle,
            freq_hz,
        }
    }

    fn scale_spec(min: f32, max: f32) -> NodeSpec {
        NodeSpec::Scale { min, max }
    }

    fn node(id: &str, spec: NodeSpec, input: Option<&str>) -> GraphNode {
        GraphNode {
            id: NodeId::new(id),
            spec,
            input: input.map(NodeId::new),
        }
    }

    fn chain() -> Graph {
        Graph {
            nodes: vec![
                node("lfo", lfo_spec(1.0), None),
                node("scale", scale_spec(0.0, 1.0), Some("lfo")),
            ],
        }
    }

    #[test]
    fn adds_a_node_and_returns_compiled_result() {
        let mut graph = Graph::default();
        let compiled = apply_command(
            &mut graph,
            GraphCommand::AddNode {
                id: NodeId::new("lfo"),
                spec: lfo_spec(2.0),
            },
        )
        .unwrap();

        assert_eq!(graph.nodes, vec![node("lfo", lfo_spec(2.0), None)]);
        assert_eq!(compiled.len(), 1);
    }

    #[test]
    fn rejects_a_duplicate_add_without_changing_graph() {
        let mut graph = chain();
        let before = graph.clone();

        assert_eq!(
            apply_command(
                &mut graph,
                GraphCommand::AddNode {
                    id: NodeId::new("lfo"),
                    spec: lfo_spec(2.0),
                },
            ),
            Err(GraphCommandError::NodeAlreadyExists(NodeId::new("lfo")))
        );
        assert_eq!(graph, before);
    }

    #[test]
    fn updates_a_node_without_changing_its_connection() {
        let mut graph = chain();
        apply_command(
            &mut graph,
            GraphCommand::UpdateNode {
                id: NodeId::new("scale"),
                spec: scale_spec(-1.0, 1.0),
            },
        )
        .unwrap();

        assert_eq!(graph.nodes[1].spec, scale_spec(-1.0, 1.0));
        assert_eq!(graph.nodes[1].input, Some(NodeId::new("lfo")));
    }

    #[test]
    fn connects_and_disconnects_nodes() {
        let mut graph = Graph {
            nodes: vec![
                node("lfo", lfo_spec(1.0), None),
                node("scale", scale_spec(0.0, 1.0), None),
            ],
        };

        apply_command(
            &mut graph,
            GraphCommand::Connect {
                source: NodeId::new("lfo"),
                target: NodeId::new("scale"),
            },
        )
        .unwrap();
        assert_eq!(graph.nodes[1].input, Some(NodeId::new("lfo")));

        apply_command(
            &mut graph,
            GraphCommand::Disconnect {
                target: NodeId::new("scale"),
            },
        )
        .unwrap();
        assert_eq!(graph.nodes[1].input, None);
    }

    #[test]
    fn remove_clears_connections_to_removed_node() {
        let mut graph = chain();
        apply_command(
            &mut graph,
            GraphCommand::RemoveNode {
                id: NodeId::new("lfo"),
            },
        )
        .unwrap();

        assert_eq!(graph.nodes, vec![node("scale", scale_spec(0.0, 1.0), None)]);
    }

    #[test]
    fn invalid_update_is_rejected_by_compile_and_rolled_back() {
        let mut graph = chain();
        let before = graph.clone();

        assert!(matches!(
            apply_command(
                &mut graph,
                GraphCommand::UpdateNode {
                    id: NodeId::new("lfo"),
                    spec: lfo_spec(f64::NAN),
                },
            ),
            Err(GraphCommandError::InvalidGraph(
                CompileError::InvalidLfoFreq { .. }
            ))
        ));
        assert_eq!(graph, before);
    }

    #[test]
    fn cycle_is_rejected_and_connection_is_rolled_back() {
        let mut graph = chain();
        let before = graph.clone();

        assert!(matches!(
            apply_command(
                &mut graph,
                GraphCommand::Connect {
                    source: NodeId::new("scale"),
                    target: NodeId::new("lfo"),
                },
            ),
            Err(GraphCommandError::InvalidGraph(CompileError::Cycle(_)))
        ));
        assert_eq!(graph, before);
    }

    #[test]
    fn batch_is_committed_only_after_every_command_and_validation_succeed() {
        let mut graph = Graph {
            nodes: vec![node("old", lfo_spec(1.0), None)],
        };

        let compiled = apply_commands(
            &mut graph,
            [
                GraphCommand::RemoveNode {
                    id: NodeId::new("old"),
                },
                GraphCommand::AddNode {
                    id: NodeId::new("new"),
                    spec: lfo_spec(3.0),
                },
            ],
        )
        .unwrap();

        assert_eq!(graph.nodes, vec![node("new", lfo_spec(3.0), None)]);
        assert_eq!(compiled.len(), 1);
    }

    #[test]
    fn command_failure_rolls_back_the_entire_batch() {
        let mut graph = chain();
        let before = graph.clone();

        assert_eq!(
            apply_commands(
                &mut graph,
                [
                    GraphCommand::UpdateNode {
                        id: NodeId::new("lfo"),
                        spec: lfo_spec(4.0),
                    },
                    GraphCommand::Disconnect {
                        target: NodeId::new("lfo"),
                    },
                ],
            ),
            Err(GraphCommandError::InputNotConnected(NodeId::new("lfo")))
        );
        assert_eq!(graph, before);
    }

    #[test]
    fn final_validation_failure_rolls_back_the_entire_batch() {
        let mut graph = chain();
        let before = graph.clone();

        assert!(matches!(
            apply_commands(
                &mut graph,
                [
                    GraphCommand::Disconnect {
                        target: NodeId::new("scale"),
                    },
                    GraphCommand::Connect {
                        source: NodeId::new("scale"),
                        target: NodeId::new("lfo"),
                    },
                    GraphCommand::Connect {
                        source: NodeId::new("lfo"),
                        target: NodeId::new("scale"),
                    },
                ],
            ),
            Err(GraphCommandError::InvalidGraph(CompileError::Cycle(_)))
        ));
        assert_eq!(graph, before);
    }

    #[test]
    fn reports_clear_connection_precondition_errors() {
        let mut graph = chain();
        let before = graph.clone();

        assert_eq!(
            apply_command(
                &mut graph,
                GraphCommand::Connect {
                    source: NodeId::new("missing"),
                    target: NodeId::new("scale"),
                },
            ),
            Err(GraphCommandError::SourceNodeNotFound(NodeId::new(
                "missing"
            )))
        );
        assert_eq!(graph, before);

        assert_eq!(
            apply_command(
                &mut graph,
                GraphCommand::Connect {
                    source: NodeId::new("lfo"),
                    target: NodeId::new("missing"),
                },
            ),
            Err(GraphCommandError::TargetNodeNotFound(NodeId::new(
                "missing"
            )))
        );
        assert_eq!(graph, before);

        assert_eq!(
            apply_command(
                &mut graph,
                GraphCommand::Connect {
                    source: NodeId::new("lfo"),
                    target: NodeId::new("scale"),
                },
            ),
            Err(GraphCommandError::InputAlreadyConnected {
                target: NodeId::new("scale"),
                connected_to: NodeId::new("lfo"),
            })
        );
        assert_eq!(graph, before);
    }

    #[test]
    fn removing_last_node_is_rejected_as_an_invalid_graph() {
        let mut graph = Graph {
            nodes: vec![node("lfo", lfo_spec(1.0), None)],
        };
        let before = graph.clone();

        assert_eq!(
            apply_command(
                &mut graph,
                GraphCommand::RemoveNode {
                    id: NodeId::new("lfo"),
                },
            ),
            Err(GraphCommandError::InvalidGraph(CompileError::EmptyGraph))
        );
        assert_eq!(graph, before);
    }
}
