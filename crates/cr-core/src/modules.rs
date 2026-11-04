//! Product-level module library built from the deterministic graph primitives.
//!
//! A module is a named, discoverable patching unit. Multiple modules can share
//! an engine primitive while carrying different musical defaults and intent.

use crate::{BindingId, DeviceId, LfoShape, MathOperation, NodeSpec, ParamName};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortDirection {
    Input,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    Control,
    Gate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModulePort {
    pub id: String,
    pub name: String,
    pub direction: PortDirection,
    pub signal: SignalType,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleDescriptor {
    pub id: String,
    pub name: String,
    pub category: String,
    pub summary: String,
    pub default_spec: NodeSpec,
    pub ports: Vec<ModulePort>,
}

impl ModuleDescriptor {
    fn new(id: &str, name: &str, category: &str, summary: &str, default_spec: NodeSpec) -> Self {
        let ports = ports_for_spec(&default_spec);
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            category: category.to_owned(),
            summary: summary.to_owned(),
            default_spec,
            ports,
        }
    }
}

fn input(id: &str, name: &str, signal: SignalType) -> ModulePort {
    ModulePort {
        id: id.to_owned(),
        name: name.to_owned(),
        direction: PortDirection::Input,
        signal,
        required: true,
    }
}

fn output() -> ModulePort {
    ModulePort {
        id: "out".into(),
        name: "OUT".into(),
        direction: PortDirection::Output,
        signal: SignalType::Control,
        required: false,
    }
}

pub fn ports_for_spec(spec: &NodeSpec) -> Vec<ModulePort> {
    match spec {
        NodeSpec::Lfo { .. }
        | NodeSpec::SampleHold { .. }
        | NodeSpec::StepSequence { .. }
        | NodeSpec::Macro { .. }
        | NodeSpec::ExternalSource { .. } => vec![output()],
        NodeSpec::Envelope { .. } => vec![input("gate", "GATE", SignalType::Gate), output()],
        NodeSpec::Sink { .. } | NodeSpec::ExternalSink { .. } => {
            vec![input("signal", "IN", SignalType::Control)]
        }
        NodeSpec::Takeover { .. } => vec![
            input("signal", "IN", SignalType::Control),
            input("control", "PICKUP", SignalType::Control),
            output(),
        ],
        NodeSpec::Slew { .. }
        | NodeSpec::Clamp { .. }
        | NodeSpec::Curve { .. }
        | NodeSpec::Quantize { .. }
        | NodeSpec::Math { .. }
        | NodeSpec::Scale { .. } => {
            vec![input("signal", "IN", SignalType::Control), output()]
        }
    }
}

/// The built-in musical module library. These are control-rate modules by
/// design; audio-rate synthesis remains in an external host such as VCV Rack.
pub fn builtin_module_catalog() -> Vec<ModuleDescriptor> {
    let mut modules = Vec::with_capacity(60);
    let mut add = |id, name, category, summary, spec| {
        modules.push(ModuleDescriptor::new(id, name, category, summary, spec));
    };

    // Modulation sources.
    for (id, name, shape, freq, summary) in [
        (
            "triangle-glacial",
            "Glacial Triangle",
            LfoShape::Triangle,
            0.02,
            "A fifty-second evolving sweep",
        ),
        (
            "triangle-slow",
            "Slow Triangle",
            LfoShape::Triangle,
            0.08,
            "Broad cyclical motion",
        ),
        (
            "triangle-classic",
            "Classic Triangle",
            LfoShape::Triangle,
            0.3,
            "The flagship three-second sweep",
        ),
        (
            "triangle-fast",
            "Fast Triangle",
            LfoShape::Triangle,
            2.0,
            "Rhythmic bipolar-style motion",
        ),
        (
            "sine-glacial",
            "Glacial Sine",
            LfoShape::Sine,
            0.02,
            "Very slow organic drift",
        ),
        (
            "sine-breathe",
            "Breathing Sine",
            LfoShape::Sine,
            0.12,
            "Gentle breathing movement",
        ),
        (
            "sine-classic",
            "Classic Sine",
            LfoShape::Sine,
            0.5,
            "Smooth two-second cycle",
        ),
        (
            "sine-fast",
            "Fast Sine",
            LfoShape::Sine,
            3.0,
            "Fast continuous modulation",
        ),
    ] {
        add(
            id,
            name,
            "Modulators",
            summary,
            NodeSpec::Lfo {
                shape,
                freq_hz: freq,
            },
        );
    }
    for (id, name, rate, seed, summary) in [
        (
            "random-drift",
            "Random Drift",
            0.15,
            11,
            "Long held random destinations",
        ),
        (
            "random-walk",
            "Random Walk",
            0.5,
            23,
            "Measured evolving steps",
        ),
        (
            "random-pulse",
            "Random Pulse",
            2.0,
            37,
            "Clock-like random control",
        ),
        (
            "random-spark",
            "Random Spark",
            8.0,
            53,
            "Rapid deterministic variation",
        ),
    ] {
        add(
            id,
            name,
            "Modulators",
            summary,
            NodeSpec::SampleHold {
                rate_hz: rate,
                seed,
            },
        );
    }

    // Sequencers and pattern generators.
    for (id, name, rate, values, summary) in [
        (
            "sequence-toggle",
            "Toggle",
            2.0,
            vec![0.0, 1.0],
            "Alternating low and high",
        ),
        (
            "sequence-rise-4",
            "Four Step Rise",
            2.0,
            vec![0.0, 0.33, 0.66, 1.0],
            "Ascending four-step ramp",
        ),
        (
            "sequence-fall-4",
            "Four Step Fall",
            2.0,
            vec![1.0, 0.66, 0.33, 0.0],
            "Descending four-step ramp",
        ),
        (
            "sequence-pendulum",
            "Pendulum",
            3.0,
            vec![0.0, 0.5, 1.0, 0.5],
            "Up-and-down four-step motion",
        ),
        (
            "sequence-euclid-8",
            "Eight Step Rhythm",
            4.0,
            vec![1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0],
            "A useful 3-in-8 control rhythm",
        ),
        (
            "sequence-stair-8",
            "Eight Step Stair",
            4.0,
            vec![0.0, 0.14, 0.28, 0.42, 0.57, 0.71, 0.85, 1.0],
            "Ascending eight-step control",
        ),
        (
            "sequence-wave-8",
            "Eight Step Wave",
            4.0,
            vec![0.0, 0.25, 0.5, 0.75, 1.0, 0.75, 0.5, 0.25],
            "Stepped triangle sequence",
        ),
        (
            "sequence-accent-16",
            "Sixteen Step Accents",
            8.0,
            vec![
                1.0, 0.2, 0.4, 0.2, 0.8, 0.2, 0.4, 0.2, 1.0, 0.2, 0.6, 0.2, 0.8, 0.2, 0.4, 0.2,
            ],
            "Velocity-style accent lane",
        ),
    ] {
        add(
            id,
            name,
            "Sequencers",
            summary,
            NodeSpec::StepSequence {
                rate_hz: rate,
                values,
            },
        );
    }

    // Slew, limiting, curves, and quantization.
    for (id, name, rise, fall, summary) in [
        (
            "slew-snap",
            "Snap Slew",
            20.0,
            20.0,
            "Tames edges without losing impact",
        ),
        (
            "slew-smooth",
            "Smooth Slew",
            2.0,
            2.0,
            "Even bidirectional smoothing",
        ),
        (
            "slew-lag",
            "Long Lag",
            0.25,
            0.25,
            "Four-second full-range glide",
        ),
        (
            "slew-rise",
            "Slow Rise",
            0.2,
            8.0,
            "Lazy attack with a fast fall",
        ),
        (
            "slew-fall",
            "Slow Fall",
            8.0,
            0.2,
            "Fast attack with a lazy fall",
        ),
    ] {
        add(
            id,
            name,
            "Shapers",
            summary,
            NodeSpec::Slew {
                rise_per_second: rise,
                fall_per_second: fall,
            },
        );
    }
    for (id, name, min, max, summary) in [
        (
            "clamp-unipolar",
            "Unipolar Guard",
            0.0,
            1.0,
            "Keeps control in the normalized range",
        ),
        (
            "clamp-bipolar",
            "Bipolar Guard",
            -1.0,
            1.0,
            "Keeps a bipolar signal bounded",
        ),
        (
            "clamp-middle",
            "Middle Band",
            0.25,
            0.75,
            "Restricts modulation to the center",
        ),
        (
            "clamp-positive",
            "Positive Only",
            0.0,
            10.0,
            "Removes negative excursions",
        ),
    ] {
        add(id, name, "Shapers", summary, NodeSpec::Clamp { min, max });
    }
    for (id, name, exponent, summary) in [
        (
            "curve-soft",
            "Soft Curve",
            0.5,
            "Expands quiet control detail",
        ),
        (
            "curve-gentle",
            "Gentle Curve",
            0.75,
            "Subtle low-level emphasis",
        ),
        (
            "curve-linear",
            "Linear Curve",
            1.0,
            "Unity transfer reference",
        ),
        (
            "curve-punch",
            "Punch Curve",
            2.0,
            "Focuses motion near the top",
        ),
        (
            "curve-hard",
            "Hard Curve",
            4.0,
            "Strong exponential shaping",
        ),
    ] {
        add(id, name, "Shapers", summary, NodeSpec::Curve { exponent });
    }
    for (id, name, steps, summary) in [
        (
            "quantize-binary",
            "Binary Quantizer",
            2,
            "Converts control to off/on states",
        ),
        ("quantize-4", "Four Levels", 4, "Coarse four-level stepping"),
        (
            "quantize-8",
            "Eight Levels",
            8,
            "Classic stepped modulation",
        ),
        (
            "quantize-12",
            "Twelve Levels",
            12,
            "Twelve evenly-spaced control levels",
        ),
        (
            "quantize-16",
            "Sixteen Levels",
            16,
            "Fine stepped modulation",
        ),
        (
            "quantize-32",
            "Thirty-Two Levels",
            32,
            "Subtle digital stepping",
        ),
    ] {
        add(id, name, "Shapers", summary, NodeSpec::Quantize { steps });
    }

    // Arithmetic and range utilities.
    for (id, name, operation, operand, summary) in [
        (
            "math-bias-up",
            "Bias +10%",
            MathOperation::Add,
            0.1,
            "Raises a control signal",
        ),
        (
            "math-bias-down",
            "Bias −10%",
            MathOperation::Subtract,
            0.1,
            "Lowers a control signal",
        ),
        (
            "math-invert",
            "Inverter",
            MathOperation::Multiply,
            -1.0,
            "Flips signal polarity",
        ),
        (
            "math-attenuate",
            "Half Attenuator",
            MathOperation::Multiply,
            0.5,
            "Halves modulation depth",
        ),
        (
            "math-amplify",
            "Double Amplifier",
            MathOperation::Multiply,
            2.0,
            "Doubles modulation depth",
        ),
        (
            "math-mute",
            "Control Mute",
            MathOperation::Multiply,
            0.0,
            "Silences a modulation lane",
        ),
        (
            "math-half",
            "Divide by Two",
            MathOperation::Divide,
            2.0,
            "Halves a value arithmetically",
        ),
    ] {
        add(
            id,
            name,
            "Utilities",
            summary,
            NodeSpec::Math { operation, operand },
        );
    }
    for (id, name, min, max, summary) in [
        (
            "scale-midi",
            "MIDI 7-bit",
            0.0,
            127.0,
            "Maps normalized control to MIDI values",
        ),
        (
            "scale-percent",
            "Percentage",
            0.0,
            100.0,
            "Maps normalized control to percent",
        ),
        (
            "scale-bipolar",
            "Bipolar",
            -1.0,
            1.0,
            "Maps normalized control around zero",
        ),
        (
            "scale-semitones",
            "Octave Semitones",
            0.0,
            12.0,
            "Maps control across one octave",
        ),
        (
            "scale-pan",
            "Pan Position",
            -100.0,
            100.0,
            "Maps control to left/right position",
        ),
    ] {
        add(id, name, "Utilities", summary, NodeSpec::Scale { min, max });
    }

    // Envelopes and performance control.
    for (id, name, attack, decay, sustain, release, summary) in [
        (
            "envelope-click",
            "Click Envelope",
            0.0,
            0.03,
            0.0,
            0.03,
            "Tiny percussive control transient",
        ),
        (
            "envelope-pluck",
            "Pluck Envelope",
            0.005,
            0.18,
            0.0,
            0.12,
            "Fast plucked contour",
        ),
        (
            "envelope-punch",
            "Punch Envelope",
            0.01,
            0.12,
            0.55,
            0.18,
            "Immediate musical contour",
        ),
        (
            "envelope-keys",
            "Keys Envelope",
            0.03,
            0.2,
            0.75,
            0.35,
            "General keyboard response",
        ),
        (
            "envelope-pad",
            "Pad Envelope",
            1.5,
            2.0,
            0.7,
            3.0,
            "Slow ambient contour",
        ),
    ] {
        add(
            id,
            name,
            "Envelopes",
            summary,
            NodeSpec::Envelope {
                gate: crate::NodeId::new(""),
                attack_seconds: attack,
                decay_seconds: decay,
                sustain,
                release_seconds: release,
            },
        );
    }
    for (id, name, epsilon, summary) in [
        (
            "pickup-precise",
            "Precise Pickup",
            0.005,
            "Tight hardware takeover",
        ),
        (
            "pickup-balanced",
            "Balanced Pickup",
            0.015,
            "General no-jump takeover",
        ),
        (
            "pickup-wide",
            "Wide Pickup",
            0.04,
            "Forgiving hardware takeover",
        ),
    ] {
        add(
            id,
            name,
            "Performance",
            summary,
            NodeSpec::Takeover {
                control: crate::NodeId::new(""),
                pickup_epsilon: epsilon,
            },
        );
    }

    // Adapter endpoints remain editable because their actual IDs come from the
    // running studio catalog.
    add(
        "macro-control",
        "Macro Control",
        "Performance",
        "Named external performance control",
        NodeSpec::Macro {
            binding: BindingId::new("s88.knob.1"),
            default: 0.0,
        },
    );
    add(
        "control-input",
        "Control Input",
        "I/O",
        "Receives a value from OSC, MIDI, or a surface",
        NodeSpec::ExternalSource {
            binding: BindingId::new("s88.knob.1"),
            default: 0.0,
        },
    );
    add(
        "device-output",
        "Device Output",
        "I/O",
        "Sends control to a semantic hardware parameter",
        NodeSpec::Sink {
            device: DeviceId::new("peak"),
            param: ParamName::new("filter.cutoff"),
        },
    );
    add(
        "signal-output",
        "Signal Output",
        "I/O",
        "Publishes control to an external adapter",
        NodeSpec::ExternalSink {
            binding: BindingId::new("osc.monitor"),
        },
    );

    debug_assert!(modules.len() >= 50);
    debug_assert_eq!(
        modules
            .iter()
            .map(|module| module.id.as_str())
            .collect::<HashSet<_>>()
            .len(),
        modules.len()
    );
    modules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_contains_more_than_fifty_unique_patchable_modules() {
        let modules = builtin_module_catalog();
        assert!(
            modules.len() >= 50,
            "catalog only contains {} modules",
            modules.len()
        );
        let ids: HashSet<_> = modules.iter().map(|module| module.id.as_str()).collect();
        assert_eq!(ids.len(), modules.len());
        assert!(modules.iter().all(|module| !module.name.is_empty()));
        assert!(modules.iter().all(|module| !module.category.is_empty()));
        assert!(modules.iter().all(|module| !module.ports.is_empty()));
    }

    #[test]
    fn port_topology_matches_engine_dependencies() {
        for module in builtin_module_catalog() {
            let inputs = module
                .ports
                .iter()
                .filter(|port| port.direction == PortDirection::Input)
                .count();
            let outputs = module
                .ports
                .iter()
                .filter(|port| port.direction == PortDirection::Output)
                .count();
            match module.default_spec {
                NodeSpec::Sink { .. } | NodeSpec::ExternalSink { .. } => {
                    assert_eq!((inputs, outputs), (1, 0), "{}", module.id);
                }
                NodeSpec::Envelope { .. } => {
                    assert_eq!((inputs, outputs), (1, 1), "{}", module.id);
                }
                NodeSpec::Takeover { .. } => {
                    assert_eq!((inputs, outputs), (2, 1), "{}", module.id);
                }
                NodeSpec::Lfo { .. }
                | NodeSpec::SampleHold { .. }
                | NodeSpec::StepSequence { .. }
                | NodeSpec::Macro { .. }
                | NodeSpec::ExternalSource { .. } => {
                    assert_eq!((inputs, outputs), (0, 1), "{}", module.id);
                }
                _ => assert_eq!((inputs, outputs), (1, 1), "{}", module.id),
            }
        }
    }
}
