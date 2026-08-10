//! Wires engine output through the pack, limiter, and encoder to a sink.

use crate::config::DeviceConfig;
use cr_core::CompiledGraph;
use cr_engine::Engine;
use cr_midi::{Limiter, LimiterConfig, LimiterStats, MidiSink, encode, quantize, resolution_bits};
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
        let engine = Engine::new(graph);

        // Resolve every sink against the pack and device up front. After
        // this, the tick does no name lookup, no hashing, and no
        // allocation.
        let mut resolved = Vec::with_capacity(engine.sinks().len());
        let mut unknown_params = Vec::new();
        let mut mismatched_devices = Vec::new();
        for binding in engine.sinks() {
            if binding.device.as_str() != device.id.as_str() {
                mismatched_devices.push(binding.device.as_str().to_string());
                resolved.push(None);
                continue;
            }
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

        if !unknown_params.is_empty() || !mismatched_devices.is_empty() {
            anyhow::bail!(
                "patch has unresolvable sinks for configured device `{}`: \
                 unknown parameters {unknown_params:?}, mismatched devices {mismatched_devices:?}",
                device.id,
            );
        }

        // One limiter slot per sink, so budgets are per graph sink.
        let mut limiter = Limiter::new(LimiterConfig::default());
        limiter.reset(resolved.len());

        Ok(Self {
            engine,
            device,
            sink,
            limiter,
            resolved,
            scratch: Vec::with_capacity(16),
            unknown_params,
            mismatched_devices,
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
