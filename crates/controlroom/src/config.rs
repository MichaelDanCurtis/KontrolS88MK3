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
