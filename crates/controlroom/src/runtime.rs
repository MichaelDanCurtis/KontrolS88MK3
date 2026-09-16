//! Wires engine output through the pack, limiter, and encoder to a sink.

use crate::config::DeviceConfig;
use cr_core::{BindingId, CompiledGraph, DeviceId};
use cr_engine::Engine;
use cr_midi::{
    BudgetedLimiter, BudgetedLimiterStats, BusLimiterConfig, LimiterConfig, MidiSink, encode,
    message_count, quantize, resolution_bits,
};
use cr_packs::{DevicePack, Parameter};
use std::collections::HashMap;

/// Everything needed to route one logical device to one MIDI output.
/// Multiple endpoints may use the same pack while retaining distinct IDs,
/// ports, and channels.
pub struct DeviceEndpoint {
    pub config: DeviceConfig,
    pub pack: DevicePack,
    pub sink: Box<dyn MidiSink>,
}

/// A sink resolved against the device pack once, at construction.
struct ResolvedSink {
    device_idx: usize,
    param: Parameter,
    bits: u8,
}

pub struct Runtime {
    engine: Engine,
    devices: Vec<DeviceEndpoint>,
    limiter: BudgetedLimiter,
    midi_burst_messages: u64,
    external_source_index: HashMap<BindingId, usize>,
    /// Indexed by the engine's sink index. `None` means the sink did not
    /// resolve (unknown parameter or device mismatch). `Runtime::new`
    /// refuses to construct a `Runtime` at all when that happens, so by the
    /// time one exists every entry here is `Some` — the `Option` stays only
    /// so `run_ticks` never has to trust that invariant blindly.
    resolved: Vec<Option<ResolvedSink>>,
    /// Scratch buffer reused every tick so encoding never allocates.
    scratch: Vec<u8>,
    /// Parameter names referenced by the graph but absent from the pack.
    /// Collected once at construction, never during a tick. Always empty on
    /// a successfully constructed `Runtime`; a nonempty list makes
    /// construction fail instead.
    pub unknown_params: Vec<String>,
    /// Sink `device` values that do not match the configured
    /// `DeviceConfig.id`. Collected once at construction, never during a
    /// tick. Always empty on a successfully constructed `Runtime`; a
    /// nonempty list makes construction fail instead.
    pub mismatched_devices: Vec<String>,
}

impl Runtime {
    /// Build a runtime from a compiled graph, resolving every sink against
    /// the pack and the configured device up front.
    ///
    /// Fails if any sink names a parameter absent from the pack, or a
    /// device that does not match `device.id`: unresolvable graphs must
    /// never reach the tick thread, so construction is where that
    /// invariant is enforced. A sink naming an unknown device is treated as
    /// unresolved rather than silently routed to the configured device.
    pub fn new(
        graph: CompiledGraph,
        pack: DevicePack,
        device: DeviceConfig,
        sink: Box<dyn MidiSink>,
    ) -> anyhow::Result<Self> {
        Self::new_multi(
            graph,
            vec![DeviceEndpoint {
                config: device,
                pack,
                sink,
            }],
        )
    }

    /// Build a runtime that can route graph sinks to multiple physical
    /// devices. Device and parameter names are resolved once here; the tick
    /// path uses only indices and never hashes or clones names.
    pub fn new_multi(graph: CompiledGraph, devices: Vec<DeviceEndpoint>) -> anyhow::Result<Self> {
        Self::new_multi_with_config(
            graph,
            devices,
            LimiterConfig::default(),
            BusLimiterConfig::default(),
        )
    }

    /// Build a multi-device runtime with explicit per-parameter and aggregate
    /// MIDI budgets. The aggregate limiter is shared by every endpoint because
    /// a MIDI interface or downstream DIN chain may carry several devices.
    pub fn new_multi_with_config(
        graph: CompiledGraph,
        devices: Vec<DeviceEndpoint>,
        parameter_limiter: LimiterConfig,
        bus_limiter: BusLimiterConfig,
    ) -> anyhow::Result<Self> {
        if devices.is_empty() {
            anyhow::bail!("runtime requires at least one device endpoint");
        }

        let mut device_index: HashMap<DeviceId, usize> = HashMap::with_capacity(devices.len());
        for (idx, endpoint) in devices.iter().enumerate() {
            if device_index
                .insert(endpoint.config.id.clone(), idx)
                .is_some()
            {
                anyhow::bail!("duplicate configured device id `{}`", endpoint.config.id);
            }
        }

        let engine = Engine::new(graph);
        let resolved = resolve_sinks(&engine, &devices, bus_limiter.burst_messages)?;
        let external_source_index = external_source_index(&engine)?;

        // One limiter slot per sink, so budgets are per graph sink.
        let mut limiter = BudgetedLimiter::new(parameter_limiter, bus_limiter);
        limiter.reset(resolved.len());

        Ok(Self {
            engine,
            devices,
            limiter,
            midi_burst_messages: bus_limiter.burst_messages,
            external_source_index,
            resolved,
            scratch: Vec::with_capacity(16),
            unknown_params: Vec::new(),
            mismatched_devices: Vec::new(),
        })
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

                if !self
                    .limiter
                    .admit(sink_idx, raw, tick, message_count(&r.param.encoding))
                {
                    continue;
                }

                self.scratch.clear();
                let Some(endpoint) = self.devices.get_mut(r.device_idx) else {
                    continue;
                };
                encode(
                    &r.param.encoding,
                    endpoint.config.wire_channel(),
                    raw,
                    &mut self.scratch,
                );
                // `MidiSink::send` accepts exactly one MIDI message. CC14 and
                // NRPN encode to multiple three-byte CC messages, so emit each
                // one separately while keeping the limiter decision atomic at
                // the semantic-update level.
                for message in self.scratch.chunks_exact(3) {
                    endpoint.sink.send(message)?;
                }
            }
        }
        Ok(())
    }

    pub fn stats(&self) -> BudgetedLimiterStats {
        self.limiter.stats()
    }

    pub fn node_values(&self) -> &[f32] {
        self.engine.values()
    }

    pub fn set_external_input(&mut self, binding: &str, value: f32) -> bool {
        let Some(&source_idx) = self.external_source_index.get(&BindingId::new(binding)) else {
            return false;
        };
        self.engine.set_external_source(source_idx, value)
    }

    pub fn external_outputs(&self) -> impl Iterator<Item = (&str, f32)> {
        self.engine
            .external_updates()
            .iter()
            .filter_map(|&(sink_idx, value)| {
                self.engine
                    .external_sinks()
                    .get(sink_idx)
                    .map(|sink| (sink.binding.as_str(), value))
            })
    }

    pub fn external_source_bindings(&self) -> impl Iterator<Item = &str> {
        self.engine
            .external_sources()
            .iter()
            .map(|source| source.binding.as_str())
    }

    pub fn external_sink_bindings(&self) -> impl Iterator<Item = &str> {
        self.engine
            .external_sinks()
            .iter()
            .map(|sink| sink.binding.as_str())
    }

    /// Replace the compiled graph at a caller-selected tick boundary. All
    /// device/parameter resolution is staged first, so a rejected graph leaves
    /// the active engine and limiter untouched.
    pub fn apply_graph(&mut self, graph: CompiledGraph) -> anyhow::Result<()> {
        let engine = Engine::new(graph);
        let resolved = resolve_sinks(&engine, &self.devices, self.midi_burst_messages)?;
        let external_source_index = external_source_index(&engine)?;
        self.limiter.reset(resolved.len());
        self.engine = engine;
        self.resolved = resolved;
        self.external_source_index = external_source_index;
        Ok(())
    }
}

fn external_source_index(engine: &Engine) -> anyhow::Result<HashMap<BindingId, usize>> {
    let mut index = HashMap::with_capacity(engine.external_sources().len());
    for (source_idx, source) in engine.external_sources().iter().enumerate() {
        if index.insert(source.binding.clone(), source_idx).is_some() {
            anyhow::bail!("duplicate external source binding `{}`", source.binding);
        }
    }
    let mut sink_bindings = std::collections::HashSet::new();
    for sink in engine.external_sinks() {
        if !sink_bindings.insert(&sink.binding) {
            anyhow::bail!("duplicate external sink binding `{}`", sink.binding);
        }
    }
    Ok(index)
}

fn resolve_sinks(
    engine: &Engine,
    devices: &[DeviceEndpoint],
    midi_burst_messages: u64,
) -> anyhow::Result<Vec<Option<ResolvedSink>>> {
    let device_index: HashMap<&DeviceId, usize> = devices
        .iter()
        .enumerate()
        .map(|(idx, endpoint)| (&endpoint.config.id, idx))
        .collect();
    let mut resolved = Vec::with_capacity(engine.sinks().len());
    let mut unknown_params = Vec::new();
    let mut unknown_devices = Vec::new();
    let mut oversized_encodings = Vec::new();

    for binding in engine.sinks() {
        let Some(&device_idx) = device_index.get(&binding.device) else {
            unknown_devices.push(binding.device.as_str().to_owned());
            resolved.push(None);
            continue;
        };
        match devices[device_idx].pack.parameter(binding.param.as_str()) {
            Some(param) => {
                let messages = u64::from(message_count(&param.encoding));
                if messages > midi_burst_messages {
                    oversized_encodings.push(format!(
                        "{}.{} requires {messages} messages",
                        binding.device, binding.param
                    ));
                }
                resolved.push(Some(ResolvedSink {
                    device_idx,
                    param: param.clone(),
                    bits: resolution_bits(&param.encoding),
                }));
            }
            None => {
                unknown_params.push(format!("{}.{}", binding.device, binding.param));
                resolved.push(None);
            }
        }
    }

    if !unknown_params.is_empty() || !unknown_devices.is_empty() {
        anyhow::bail!(
            "patch has unresolvable sinks: unknown parameters {unknown_params:?}, \
             unknown devices {unknown_devices:?}",
        );
    }
    if !oversized_encodings.is_empty() {
        anyhow::bail!(
            "runtime.midi_burst_messages={midi_burst_messages} cannot fit sink encodings: \
             {oversized_encodings:?}"
        );
    }
    Ok(resolved)
}
