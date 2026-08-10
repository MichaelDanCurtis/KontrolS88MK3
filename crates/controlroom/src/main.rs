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
    let args = std::env::args().skip(1);
    let mut path = None;
    let mut dry_run = false;
    let mut list = false;

    for a in args {
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
    let src =
        std::fs::read_to_string(&path).with_context(|| format!("reading patch config {path}"))?;
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

    let mut rt = runtime::Runtime::new(graph, pack, cfg.device.clone(), sink)
        .context("constructing runtime")?;
    println!("running {path} at 1 kHz; ctrl-c to stop");

    let start = Instant::now();
    let mut tick: u64 = 0;
    loop {
        rt.run_ticks(tick, BATCH)?;
        tick += BATCH;

        // Report the bus budget every 5 seconds. Suppression is never silent.
        if tick.is_multiple_of(5000) {
            let s = rt.stats();
            println!(
                "t={tick} sent={} deduped={} rate_limited={} unknown_slot={}",
                s.sent, s.deduped, s.rate_limited, s.unknown_slot
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
