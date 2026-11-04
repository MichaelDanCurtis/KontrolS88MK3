//! Patch config files: the authoring surface until the browser UI exists.

use cr_core::{DeviceId, Graph, GraphNode};
use cr_osc::OscConfig;
use cr_s88::S88Config;
use cr_server::rack_copilot::RackCopilotConfig;
#[cfg(test)]
use cr_server::rack_copilot::RackCopilotProvider;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const DEFAULT_BATCH_TICKS: u64 = 10;
const DEFAULT_REPORT_INTERVAL_TICKS: u64 = 5_000;
const DEFAULT_PARAMETER_MIN_INTERVAL_TICKS: u64 = 10;
const DEFAULT_MIDI_MAX_MESSAGES_PER_SECOND: u64 = 300;
const DEFAULT_MIDI_BURST_MESSAGES: u64 = 4;
const DEFAULT_DEVICE_PACK: &str = "novation-peak";
const DEFAULT_SERVER_LISTEN: &str = "127.0.0.1:7777";
const DEFAULT_PUBLISH_INTERVAL_TICKS: u64 = 33;

fn default_batch_ticks() -> u64 {
    DEFAULT_BATCH_TICKS
}

fn default_report_interval_ticks() -> u64 {
    DEFAULT_REPORT_INTERVAL_TICKS
}

fn default_parameter_min_interval_ticks() -> u64 {
    DEFAULT_PARAMETER_MIN_INTERVAL_TICKS
}

fn default_midi_max_messages_per_second() -> u64 {
    DEFAULT_MIDI_MAX_MESSAGES_PER_SECOND
}

fn default_midi_burst_messages() -> u64 {
    DEFAULT_MIDI_BURST_MESSAGES
}

fn default_device_pack() -> String {
    DEFAULT_DEVICE_PACK.to_owned()
}

fn default_server_enabled() -> bool {
    true
}

fn default_server_listen() -> String {
    DEFAULT_SERVER_LISTEN.to_owned()
}

fn default_publish_interval_ticks() -> u64 {
    DEFAULT_PUBLISH_INTERVAL_TICKS
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ControlPlaneConfig {
    #[serde(default = "default_server_enabled")]
    pub enabled: bool,
    #[serde(default = "default_server_listen")]
    pub listen: String,
    /// Optional graph-only state file, resolved beside the patch config.
    pub state_file: Option<String>,
    #[serde(default = "default_publish_interval_ticks")]
    pub publish_interval_ticks: u64,
}

impl Default for ControlPlaneConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            listen: default_server_listen(),
            state_file: None,
            publish_interval_ticks: DEFAULT_PUBLISH_INTERVAL_TICKS,
        }
    }
}

impl ControlPlaneConfig {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.listen.parse::<std::net::SocketAddr>().is_ok(),
            "server.listen must be an IP socket address, got {:?}",
            self.listen
        );
        anyhow::ensure!(
            self.publish_interval_ticks > 0,
            "server.publish_interval_ticks must be greater than zero"
        );
        if let Some(path) = &self.state_file {
            anyhow::ensure!(
                !path.trim().is_empty(),
                "server.state_file must not be empty"
            );
        }
        Ok(())
    }
}

/// Wall-clock pacing and observability settings for the fixed 1 kHz engine.
///
/// These values deliberately use ticks rather than milliseconds: the engine's
/// control rate is an architectural invariant, while batching and reporting
/// are operational choices. Omitting the whole section preserves the M1
/// defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeConfig {
    #[serde(default = "default_batch_ticks")]
    pub batch_ticks: u64,
    #[serde(default = "default_report_interval_ticks")]
    pub report_interval_ticks: u64,
    /// Minimum spacing between updates to the same graph sink.
    #[serde(default = "default_parameter_min_interval_ticks")]
    pub parameter_min_interval_ticks: u64,
    /// Sustained aggregate allowance across the complete MIDI output bus.
    #[serde(default = "default_midi_max_messages_per_second")]
    pub midi_max_messages_per_second: u64,
    /// Short aggregate burst allowance. Four messages permits one NRPN update.
    #[serde(default = "default_midi_burst_messages")]
    pub midi_burst_messages: u64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            batch_ticks: DEFAULT_BATCH_TICKS,
            report_interval_ticks: DEFAULT_REPORT_INTERVAL_TICKS,
            parameter_min_interval_ticks: DEFAULT_PARAMETER_MIN_INTERVAL_TICKS,
            midi_max_messages_per_second: DEFAULT_MIDI_MAX_MESSAGES_PER_SECOND,
            midi_burst_messages: DEFAULT_MIDI_BURST_MESSAGES,
        }
    }
}

impl RuntimeConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.batch_ticks > 0,
            "runtime.batch_ticks must be greater than zero"
        );
        anyhow::ensure!(
            self.report_interval_ticks > 0,
            "runtime.report_interval_ticks must be greater than zero"
        );
        anyhow::ensure!(
            self.parameter_min_interval_ticks > 0,
            "runtime.parameter_min_interval_ticks must be greater than zero"
        );
        anyhow::ensure!(
            self.midi_max_messages_per_second > 0,
            "runtime.midi_max_messages_per_second must be greater than zero"
        );
        anyhow::ensure!(
            self.midi_burst_messages > 0,
            "runtime.midi_burst_messages must be greater than zero"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceConfig {
    pub id: DeviceId,
    /// Stable semantic pack name. The Peak pack is built in for legacy M1
    /// configs; additional packs can be loaded through `pack_files`.
    #[serde(default = "default_device_pack")]
    pub pack: String,
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
#[serde(deny_unknown_fields)]
pub struct PatchConfig {
    /// Primary device retained for backward compatibility with the M1 config.
    pub device: DeviceConfig,
    /// Additional logical devices routed by sink `device` IDs.
    #[serde(default)]
    pub devices: Vec<DeviceConfig>,
    /// YAML device packs to add to the built-in registry. Relative paths are
    /// resolved beside the patch file by the binary.
    #[serde(default)]
    pub pack_files: Vec<String>,
    #[serde(default)]
    pub runtime: RuntimeConfig,
    #[serde(default)]
    pub server: ControlPlaneConfig,
    /// Optional AI planner for the localhost VCV Rack bridge. Disabled unless
    /// explicitly enabled, with credentials read only from the environment.
    #[serde(default)]
    pub ai: RackCopilotConfig,
    #[serde(default)]
    pub osc: OscConfig,
    #[serde(default)]
    pub s88: S88Config,
    pub nodes: Vec<GraphNode>,
}

impl PatchConfig {
    pub fn from_yaml(src: &str) -> anyhow::Result<Self> {
        let config: Self = serde_norway::from_str(src)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        let mut ids = HashSet::new();
        for device in self.all_devices() {
            anyhow::ensure!(
                (1..=16).contains(&device.channel),
                "device `{}` channel must be in 1..=16, got {}",
                device.id,
                device.channel
            );
            anyhow::ensure!(
                !device.id.as_str().trim().is_empty(),
                "device.id must not be empty"
            );
            anyhow::ensure!(
                ids.insert(device.id.clone()),
                "duplicate device id `{}`",
                device.id
            );
            anyhow::ensure!(
                !device.pack.trim().is_empty(),
                "device `{}` pack must not be empty",
                device.id
            );
            anyhow::ensure!(
                !device.port.trim().is_empty(),
                "device `{}` port must not be empty",
                device.id
            );
        }
        for path in &self.pack_files {
            anyhow::ensure!(
                !path.trim().is_empty(),
                "pack_files entries must not be empty"
            );
        }
        self.runtime.validate()?;
        self.server.validate()?;
        self.ai.validate()?;
        self.osc.validate()?;
        self.s88.validate()
    }

    pub fn all_devices(&self) -> impl Iterator<Item = &DeviceConfig> {
        std::iter::once(&self.device).chain(self.devices.iter())
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
    const DUAL_DEMO: &str = include_str!("../../../config/dual-peak.yaml");
    const OSC_DEMO: &str = include_str!("../../../config/osc-bridge.yaml");
    const VCV_DEMO: &str = include_str!("../../../config/vcv-bridge.yaml");
    const APPLIANCE_DEMO: &str = include_str!("../../../deploy/controlroom.yaml");

    #[test]
    fn demo_patch_parses() {
        let cfg = PatchConfig::from_yaml(DEMO).expect("demo config must parse");
        assert_eq!(cfg.device.id, DeviceId::new("peak"));
        assert_eq!(cfg.device.channel, 1);
        assert_eq!(cfg.runtime, RuntimeConfig::default());
        assert_eq!(cfg.server, ControlPlaneConfig::default());
        assert_eq!(cfg.osc, OscConfig::default());
        assert_eq!(cfg.s88, S88Config::default());
        assert_eq!(cfg.nodes.len(), 3);
    }

    #[test]
    fn demo_patch_compiles() {
        let cfg = PatchConfig::from_yaml(DEMO).unwrap();
        let compiled = compile(&cfg.to_graph()).expect("demo graph must compile");
        assert_eq!(compiled.len(), 3);
    }

    #[test]
    fn dual_device_demo_parses_and_compiles() {
        let cfg = PatchConfig::from_yaml(DUAL_DEMO).expect("dual-device demo parses");
        assert_eq!(cfg.all_devices().count(), 2);
        assert_eq!(compile(&cfg.to_graph()).unwrap().len(), 4);
    }

    #[test]
    fn osc_bridge_demo_parses_and_compiles() {
        let cfg = PatchConfig::from_yaml(OSC_DEMO).expect("OSC demo parses");
        assert!(cfg.osc.enabled);
        assert_eq!(cfg.osc.bindings.len(), 2);
        assert_eq!(compile(&cfg.to_graph()).unwrap().len(), 5);
    }

    #[test]
    fn vcv_bridge_exposes_eight_bidirectional_control_lanes() {
        let cfg = PatchConfig::from_yaml(VCV_DEMO).expect("VCV bridge demo parses");
        assert!(cfg.osc.enabled);
        assert_eq!(cfg.ai.provider, RackCopilotProvider::Openai);
        assert_eq!(cfg.osc.bindings.len(), 16);
        assert_eq!(compile(&cfg.to_graph()).unwrap().len(), 3);
    }

    #[test]
    fn planner_configuration_is_strict_and_provider_specific() {
        let configured = DEMO.replacen(
            "nodes:",
            "ai:\n  enabled: true\n  provider: acp\n  acp:\n    command: gemini\n    args: [--acp]\nnodes:",
            1,
        );
        let cfg = PatchConfig::from_yaml(&configured).unwrap();
        assert_eq!(cfg.ai.provider, RackCopilotProvider::Acp);
        assert_eq!(cfg.ai.acp.command, "gemini");
        assert_eq!(cfg.ai.acp.args, ["--acp"]);

        let invalid = configured.replace("command: gemini", "command: ''");
        assert!(
            PatchConfig::from_yaml(&invalid)
                .unwrap_err()
                .to_string()
                .contains("ai.acp.command")
        );

        let claude = DEMO.replacen(
            "nodes:",
            "ai:\n  enabled: true\n  provider: claude\n  claude:\n    model: sonnet\nnodes:",
            1,
        );
        let cfg = PatchConfig::from_yaml(&claude).unwrap();
        assert_eq!(cfg.ai.provider, RackCopilotProvider::Claude);
        assert_eq!(cfg.ai.claude.model.as_deref(), Some("sonnet"));

        let grok = DEMO.replacen(
            "nodes:",
            "ai:\n  enabled: true\n  provider: grok\n  grok:\n    model: grok-4.6\nnodes:",
            1,
        );
        let cfg = PatchConfig::from_yaml(&grok).unwrap();
        assert_eq!(cfg.ai.provider, RackCopilotProvider::Grok);
        assert_eq!(cfg.ai.grok.model.as_deref(), Some("grok-4.6"));
    }

    #[test]
    fn s88_takeover_demo_parses_and_compiles() {
        let cfg = PatchConfig::from_yaml(include_str!("../../../config/s88-takeover.yaml"))
            .expect("S88 demo parses");
        assert!(cfg.s88.enabled);
        assert_eq!(cfg.s88.knobs.len(), 8);
        assert_eq!(compile(&cfg.to_graph()).unwrap().len(), 5);
    }

    #[test]
    fn appliance_config_has_explicit_network_and_state_paths() {
        let cfg = PatchConfig::from_yaml(APPLIANCE_DEMO).expect("appliance config parses");
        assert_eq!(cfg.server.listen, "0.0.0.0:7777");
        assert_eq!(
            cfg.server.state_file.as_deref(),
            Some("/var/lib/controlroom/active.state.yaml")
        );
        assert_eq!(compile(&cfg.to_graph()).unwrap().len(), 3);
    }

    #[test]
    fn channel_converts_from_one_based_to_wire() {
        let d = DeviceConfig {
            id: DeviceId::new("x"),
            pack: "novation-peak".into(),
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

    #[test]
    fn runtime_settings_are_optional_and_configurable() {
        let configured = DEMO.replacen(
            "nodes:",
            "runtime:\n  batch_ticks: 25\n  report_interval_ticks: 1000\n  parameter_min_interval_ticks: 8\n  midi_max_messages_per_second: 240\n  midi_burst_messages: 6\nnodes:",
            1,
        );
        let cfg = PatchConfig::from_yaml(&configured).unwrap();
        assert_eq!(cfg.runtime.batch_ticks, 25);
        assert_eq!(cfg.runtime.report_interval_ticks, 1_000);
        assert_eq!(cfg.runtime.parameter_min_interval_ticks, 8);
        assert_eq!(cfg.runtime.midi_max_messages_per_second, 240);
        assert_eq!(cfg.runtime.midi_burst_messages, 6);
    }

    #[test]
    fn server_settings_are_optional_and_validated() {
        let configured = DEMO.replacen(
            "nodes:",
            "server:\n  enabled: false\n  listen: 0.0.0.0:8888\n  state_file: saved/graph.yaml\n  publish_interval_ticks: 40\nnodes:",
            1,
        );
        let cfg = PatchConfig::from_yaml(&configured).unwrap();
        assert!(!cfg.server.enabled);
        assert_eq!(cfg.server.listen, "0.0.0.0:8888");
        assert_eq!(cfg.server.publish_interval_ticks, 40);

        let invalid = DEMO.replacen("nodes:", "server:\n  listen: localhost:abc\nnodes:", 1);
        assert!(
            PatchConfig::from_yaml(&invalid)
                .unwrap_err()
                .to_string()
                .contains("server.listen")
        );
    }

    #[test]
    fn invalid_operational_and_device_settings_are_rejected() {
        let zero_batch = DEMO.replacen("nodes:", "runtime:\n  batch_ticks: 0\nnodes:", 1);
        let error = PatchConfig::from_yaml(&zero_batch).unwrap_err();
        assert!(error.to_string().contains("batch_ticks"));

        let bad_channel = DEMO.replacen("channel: 1", "channel: 17", 1);
        let error = PatchConfig::from_yaml(&bad_channel).unwrap_err();
        assert!(error.to_string().contains("1..=16"));
    }

    #[test]
    fn unknown_config_fields_are_rejected() {
        let typo = DEMO.replacen("nodes:", "batch_tickz: 10\nnodes:", 1);
        let error = PatchConfig::from_yaml(&typo).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn legacy_device_gets_default_pack_and_additional_devices_validate() {
        let configured = DEMO.replacen(
            "nodes:",
            "devices:\n  - id: peak-right\n    pack: novation-peak\n    port: Peak Right\n    channel: 2\nnodes:",
            1,
        );
        let cfg = PatchConfig::from_yaml(&configured).unwrap();
        assert_eq!(cfg.device.pack, "novation-peak");
        assert_eq!(cfg.all_devices().count(), 2);
    }

    #[test]
    fn duplicate_device_ids_are_rejected() {
        let configured = DEMO.replacen(
            "nodes:",
            "devices:\n  - id: peak\n    port: Another Peak\n    channel: 2\nnodes:",
            1,
        );
        assert!(
            PatchConfig::from_yaml(&configured)
                .unwrap_err()
                .to_string()
                .contains("duplicate device id")
        );
    }
}
