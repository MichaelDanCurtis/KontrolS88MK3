use anyhow::{Context, Result, bail};
use controlroom::{config, runtime};
use cr_core::{CompiledGraph, NodeSpec, compile};
use cr_midi::{
    BusLimiterConfig, LimiterConfig, MidiConnectionStatus, MidiError, MidiSink, MidirSink,
    MidirSinkFactory, ReconnectingSink,
};
use cr_packs::{DevicePack, Encoding, NOVATION_PEAK_YAML, PackRegistry};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const CONTROL_RATE_HZ: u64 = 1_000;
const MAX_INTERRUPT_CHUNK_TICKS: u64 = 10;
const MIDI_RECONNECT_INTERVAL: Duration = Duration::from_millis(500);
const DEFAULT_PATCH: &str = "config/lfo-cutoff.yaml";

const HELP: &str = "Control Room semantic MIDI router

Usage: controlroom [OPTIONS] [PATCH]

Arguments:
  [PATCH]  Patch YAML file [default: config/lfo-cutoff.yaml]

Options:
      --dry-run                         Run without opening a MIDI port
      --check                           Validate the patch and exit without opening I/O
      --preflight                       Validate, resolve ports, and probe listeners without sending
      --list-ports                      List MIDI output ports and exit
      --batch-ticks <TICKS>             Override runtime.batch_ticks
      --report-interval-ticks <TICKS>   Override runtime.report_interval_ticks
      --parameter-min-interval-ticks <TICKS>
                                        Override per-parameter MIDI spacing
      --midi-max-messages-per-second <COUNT>
                                        Override the aggregate MIDI bus rate
      --midi-burst-messages <COUNT>     Override the aggregate burst allowance
      --listen <ADDRESS>                Override server.listen
      --state-file <PATH>               Override server.state_file
      --no-server                       Disable HTTP and WebSocket control plane
  -h, --help                            Print help
  -V, --version                         Print version";

#[derive(Debug, PartialEq, Eq)]
struct Cli {
    patch: String,
    dry_run: bool,
    check: bool,
    preflight: bool,
    list_ports: bool,
    batch_ticks: Option<u64>,
    report_interval_ticks: Option<u64>,
    parameter_min_interval_ticks: Option<u64>,
    midi_max_messages_per_second: Option<u64>,
    midi_burst_messages: Option<u64>,
    listen: Option<String>,
    state_file: Option<String>,
    no_server: bool,
    action: Action,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum Action {
    #[default]
    Run,
    Help,
    Version,
}

impl Cli {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut args = args.into_iter();
        let mut cli = Self {
            patch: DEFAULT_PATCH.to_string(),
            dry_run: false,
            check: false,
            preflight: false,
            list_ports: false,
            batch_ticks: None,
            report_interval_ticks: None,
            parameter_min_interval_ticks: None,
            midi_max_messages_per_second: None,
            midi_burst_messages: None,
            listen: None,
            state_file: None,
            no_server: false,
            action: Action::Run,
        };
        let mut patch_seen = false;
        let mut options_done = false;

        while let Some(arg) = args.next() {
            if !options_done {
                match arg.as_str() {
                    "--" => {
                        options_done = true;
                        continue;
                    }
                    "--dry-run" => cli.dry_run = true,
                    "--check" => cli.check = true,
                    "--preflight" => cli.preflight = true,
                    "--list-ports" => cli.list_ports = true,
                    "-h" | "--help" => cli.action = Action::Help,
                    "-V" | "--version" => cli.action = Action::Version,
                    "--batch-ticks" => {
                        cli.batch_ticks =
                            Some(parse_positive_option("--batch-ticks", args.next())?);
                    }
                    "--report-interval-ticks" => {
                        cli.report_interval_ticks = Some(parse_positive_option(
                            "--report-interval-ticks",
                            args.next(),
                        )?);
                    }
                    "--parameter-min-interval-ticks" => {
                        cli.parameter_min_interval_ticks = Some(parse_positive_option(
                            "--parameter-min-interval-ticks",
                            args.next(),
                        )?);
                    }
                    "--midi-max-messages-per-second" => {
                        cli.midi_max_messages_per_second = Some(parse_positive_option(
                            "--midi-max-messages-per-second",
                            args.next(),
                        )?);
                    }
                    "--midi-burst-messages" => {
                        cli.midi_burst_messages =
                            Some(parse_positive_option("--midi-burst-messages", args.next())?);
                    }
                    "--listen" => {
                        cli.listen = Some(parse_string_option("--listen", args.next())?);
                    }
                    "--state-file" => {
                        cli.state_file = Some(parse_string_option("--state-file", args.next())?);
                    }
                    "--no-server" => cli.no_server = true,
                    _ if arg.starts_with('-') => {
                        bail!("unknown option {arg:?}; run `controlroom --help` for usage")
                    }
                    _ => set_patch(&mut cli, &mut patch_seen, arg)?,
                }
            } else {
                set_patch(&mut cli, &mut patch_seen, arg)?;
            }
        }

        if cli.action == Action::Run {
            let exclusive = [cli.check, cli.preflight, cli.dry_run, cli.list_ports]
                .into_iter()
                .filter(|enabled| *enabled)
                .count();
            if exclusive > 1 {
                bail!("--check, --preflight, --dry-run, and --list-ports are mutually exclusive");
            }
        }

        Ok(cli)
    }
}

fn set_patch(cli: &mut Cli, patch_seen: &mut bool, patch: String) -> Result<()> {
    if *patch_seen {
        bail!("multiple patch paths supplied; expected at most one");
    }
    cli.patch = patch;
    *patch_seen = true;
    Ok(())
}

fn parse_positive_option(name: &str, value: Option<String>) -> Result<u64> {
    let value = value.with_context(|| format!("{name} requires a positive integer value"))?;
    let parsed = value.parse::<u64>().with_context(|| {
        format!("invalid value {value:?} for {name}; expected a positive integer")
    })?;
    if parsed == 0 {
        bail!("invalid value 0 for {name}; expected a positive integer");
    }
    Ok(parsed)
}

fn parse_string_option(name: &str, value: Option<String>) -> Result<String> {
    let value = value.with_context(|| format!("{name} requires a value"))?;
    if value.trim().is_empty() {
        bail!("{name} requires a non-empty value");
    }
    Ok(value)
}

/// A bounded dry-run sink: messages are intentionally discarded after the
/// runtime's limiter has counted them. Unlike `CaptureSink`, memory use stays
/// constant regardless of how long the process runs.
#[derive(Debug, Default)]
struct DiscardSink;

impl MidiSink for DiscardSink {
    fn send(&mut self, _bytes: &[u8]) -> Result<(), MidiError> {
        Ok(())
    }
}

/// Cloneable handle onto one physical MIDI port. Logical devices that share a
/// port (usually on different channels) share the same connection rather than
/// opening competing connections.
#[derive(Clone)]
struct SharedSink(Rc<RefCell<Box<dyn MidiSink>>>);

impl MidiSink for SharedSink {
    fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError> {
        self.0
            .try_borrow_mut()
            .map_err(|_| MidiError::Send("shared MIDI output is already borrowed".into()))?
            .send(bytes)
    }
}

struct PortConnection {
    sink: Rc<RefCell<Box<dyn MidiSink>>>,
    status: Option<MidiConnectionStatus>,
}

struct MidiMonitor {
    device_id: String,
    endpoint: String,
    status: Option<MidiConnectionStatus>,
    dry_run: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("event=fatal error={error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse(std::env::args().skip(1))?;

    match cli.action {
        Action::Help => {
            println!("{HELP}");
            return Ok(());
        }
        Action::Version => {
            println!("controlroom {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Action::Run => {}
    }

    if cli.list_ports {
        let ports = MidirSink::list_ports().context("enumerating MIDI output ports")?;
        if ports.is_empty() {
            eprintln!("event=ports status=empty count=0");
        } else {
            for name in ports {
                println!("{name}");
            }
        }
        return Ok(());
    }

    let src = std::fs::read_to_string(&cli.patch)
        .with_context(|| format!("reading patch config {:?}", cli.patch))?;
    let cfg = config::PatchConfig::from_yaml(&src)
        .with_context(|| format!("parsing patch config {:?}", cli.patch))?;
    let patch_path = Path::new(&cli.patch);
    let patch_dir = patch_path.parent().unwrap_or_else(|| Path::new("."));
    let mut server_config = cfg.server.clone();
    if cli.no_server {
        server_config.enabled = false;
    }
    if let Some(listen) = &cli.listen {
        server_config.listen = listen.clone();
    }
    if let Some(state_file) = &cli.state_file {
        server_config.state_file = Some(state_file.clone());
    }
    let state_path = server_config
        .state_file
        .as_deref()
        .map(|path| resolve_beside(patch_dir, path))
        .unwrap_or_else(|| patch_path.with_extension("state.yaml"));

    let mut authored_graph = cfg.to_graph();
    if server_config.enabled && !cli.check && !cli.preflight {
        match cr_server::load_persisted_graph(&state_path) {
            Ok(Some(persisted)) => match compile(&persisted) {
                Ok(_) => {
                    eprintln!("event=persistence status=loaded path={state_path:?}");
                    authored_graph = persisted;
                }
                Err(error) => eprintln!(
                    "event=persistence status=ignored path={state_path:?} reason={error:?}"
                ),
            },
            Ok(None) => {}
            Err(error) => {
                eprintln!("event=persistence status=ignored path={state_path:?} reason={error:#}")
            }
        }
    }
    let graph = compile(&authored_graph).context("compiling active patch graph")?;
    let peak = DevicePack::from_yaml(NOVATION_PEAK_YAML).context("loading built-in Peak pack")?;
    let mut packs = PackRegistry::with_packs([peak]).context("building device pack registry")?;
    for configured_path in &cfg.pack_files {
        let path = resolve_beside(patch_dir, configured_path);
        let source = std::fs::read_to_string(&path)
            .with_context(|| format!("reading device pack {path:?}"))?;
        let pack = DevicePack::from_yaml(&source)
            .with_context(|| format!("parsing device pack {path:?}"))?;
        packs
            .insert(pack)
            .with_context(|| format!("registering device pack {path:?}"))?;
    }

    let mut timing = cfg.runtime.clone();
    if let Some(batch_ticks) = cli.batch_ticks {
        timing.batch_ticks = batch_ticks;
    }
    if let Some(report_interval_ticks) = cli.report_interval_ticks {
        timing.report_interval_ticks = report_interval_ticks;
    }
    if let Some(parameter_min_interval_ticks) = cli.parameter_min_interval_ticks {
        timing.parameter_min_interval_ticks = parameter_min_interval_ticks;
    }
    if let Some(midi_max_messages_per_second) = cli.midi_max_messages_per_second {
        timing.midi_max_messages_per_second = midi_max_messages_per_second;
    }
    if let Some(midi_burst_messages) = cli.midi_burst_messages {
        timing.midi_burst_messages = midi_burst_messages;
    }
    timing.validate().context("validating runtime settings")?;

    if cli.check || cli.preflight {
        let parameter_limiter = LimiterConfig {
            min_interval_ticks: timing.parameter_min_interval_ticks,
        };
        let bus_limiter = BusLimiterConfig {
            ticks_per_second: CONTROL_RATE_HZ,
            max_messages_per_second: timing.midi_max_messages_per_second,
            burst_messages: timing.midi_burst_messages,
        };
        let endpoints = cfg
            .all_devices()
            .map(|device| {
                Ok(runtime::DeviceEndpoint {
                    config: device.clone(),
                    pack: packs
                        .get(&device.pack)
                        .with_context(|| format!("resolving pack for device `{}`", device.id))?
                        .clone(),
                    sink: Box::new(DiscardSink) as Box<dyn MidiSink>,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let runtime = runtime::Runtime::new_multi_with_config(
            graph.clone(),
            endpoints,
            parameter_limiter,
            bus_limiter,
        )
        .context("constructing checked runtime")?;
        validate_configured_external_bindings(&runtime, &cfg.osc, &cfg.s88)?;
        if cli.preflight {
            let mut failures = 0usize;
            for device in cfg.all_devices() {
                match MidirSink::resolve_port_name(&device.port) {
                    Ok(resolved) => println!(
                        "event=preflight transport=midi_out status=ok device={} match={:?} resolved={:?}",
                        device.id, device.port, resolved
                    ),
                    Err(error) => {
                        failures += 1;
                        println!(
                            "event=preflight transport=midi_out status=missing device={} match={:?} error={error:?}",
                            device.id, device.port
                        );
                    }
                }
            }
            if cfg.s88.enabled {
                match cr_s88::resolve_input_port_name(&cfg.s88.port) {
                    Ok(resolved) => println!(
                        "event=preflight transport=s88_input status=ok match={:?} resolved={resolved:?}",
                        cfg.s88.port
                    ),
                    Err(error) => {
                        failures += 1;
                        println!(
                            "event=preflight transport=s88_input status=missing match={:?} error={error:?}",
                            cfg.s88.port
                        );
                    }
                }
            }
            if cfg.osc.enabled {
                match std::net::UdpSocket::bind(&cfg.osc.listen) {
                    Ok(socket) => println!(
                        "event=preflight transport=osc status=ok listen={}",
                        socket.local_addr()?
                    ),
                    Err(error) => {
                        failures += 1;
                        println!(
                            "event=preflight transport=osc status=unavailable listen={} error={error:?}",
                            cfg.osc.listen
                        );
                    }
                }
            }
            if server_config.enabled {
                match std::net::TcpListener::bind(&server_config.listen) {
                    Ok(listener) => println!(
                        "event=preflight transport=http status=ok listen={}",
                        listener.local_addr()?
                    ),
                    Err(error) => {
                        failures += 1;
                        println!(
                            "event=preflight transport=http status=unavailable listen={} error={error:?}",
                            server_config.listen
                        );
                    }
                }
            }
            if failures > 0 {
                bail!("hardware preflight found {failures} unavailable requirement(s)");
            }
        }
        println!(
            "event={} status=ok patch={:?} nodes={} devices={} sinks={} external_inputs={} external_outputs={} osc_enabled={} s88_enabled={}",
            if cli.preflight { "preflight" } else { "check" },
            cli.patch,
            graph.len(),
            cfg.all_devices().count(),
            graph
                .nodes()
                .iter()
                .filter(|node| matches!(node.spec, NodeSpec::Sink { .. }))
                .count(),
            runtime.external_source_bindings().count(),
            runtime.external_sink_bindings().count(),
            cfg.osc.enabled,
            cfg.s88.enabled,
        );
        return Ok(());
    }

    let mut ports: HashMap<String, PortConnection> = HashMap::new();
    let mut midi_monitors = Vec::new();
    let mut endpoints = Vec::new();
    for device in cfg.all_devices() {
        let resolved_port = if cli.dry_run {
            device.port.clone()
        } else {
            MidirSink::resolve_port_name(&device.port).with_context(|| {
                format!(
                    "resolving MIDI output for device `{}` matching {:?}",
                    device.id, device.port
                )
            })?
        };
        let port_key = resolved_port.to_lowercase();
        let (shared, status) = if let Some(existing) = ports.get(&port_key) {
            (Rc::clone(&existing.sink), existing.status.clone())
        } else {
            let (sink, status): (Box<dyn MidiSink>, Option<MidiConnectionStatus>) = if cli.dry_run {
                eprintln!(
                    "event=midi mode=dry-run status=discarding device={} port_match={:?}",
                    device.id, device.port
                );
                (Box::new(DiscardSink), None)
            } else {
                eprintln!(
                    "event=midi mode=live status=opening device={} port_match={:?} resolved_port={:?}",
                    device.id, device.port, resolved_port
                );
                let reconnecting = ReconnectingSink::connect(
                    MidirSinkFactory::new(&resolved_port),
                    MIDI_RECONNECT_INTERVAL,
                )
                .with_context(|| {
                    format!(
                        "opening resolved MIDI output for device `{}` at {:?}",
                        device.id, resolved_port
                    )
                })?;
                let status = reconnecting.status();
                (Box::new(reconnecting), Some(status))
            };
            let shared = Rc::new(RefCell::new(sink));
            ports.insert(
                port_key,
                PortConnection {
                    sink: Rc::clone(&shared),
                    status: status.clone(),
                },
            );
            (shared, status)
        };

        midi_monitors.push(MidiMonitor {
            device_id: device.id.as_str().to_owned(),
            endpoint: resolved_port,
            status,
            dry_run: cli.dry_run,
        });

        let pack = packs
            .get(&device.pack)
            .with_context(|| format!("resolving pack for device `{}`", device.id))?
            .clone();
        endpoints.push(runtime::DeviceEndpoint {
            config: device.clone(),
            pack,
            sink: Box::new(SharedSink(shared)),
        });
    }

    let parameter_limiter = LimiterConfig {
        min_interval_ticks: timing.parameter_min_interval_ticks,
    };
    let bus_limiter = BusLimiterConfig {
        ticks_per_second: CONTROL_RATE_HZ,
        max_messages_per_second: timing.midi_max_messages_per_second,
        burst_messages: timing.midi_burst_messages,
    };
    let mut rt =
        runtime::Runtime::new_multi_with_config(graph, endpoints, parameter_limiter, bus_limiter)
            .context("constructing runtime")?;
    let osc_adapter = if cfg.osc.enabled {
        let adapter = cr_osc::OscAdapter::start(cfg.osc.clone()).context("starting OSC adapter")?;
        eprintln!(
            "event=osc status=started listen={} inputs={} outputs={}",
            adapter.local_addr(),
            cfg.osc
                .bindings
                .iter()
                .filter(|binding| binding.direction == cr_osc::Direction::Input)
                .count(),
            cfg.osc
                .bindings
                .iter()
                .filter(|binding| binding.direction == cr_osc::Direction::Output)
                .count()
        );
        Some(adapter)
    } else {
        None
    };
    let s88_adapter = if cfg.s88.enabled && !cli.dry_run {
        let adapter =
            cr_s88::S88Adapter::start(cfg.s88.clone()).context("starting Kontrol S88 adapter")?;
        eprintln!(
            "event=s88 status=started port_match={:?} channel={} knobs={}",
            cfg.s88.port,
            cfg.s88.channel,
            cfg.s88.knobs.len()
        );
        Some(adapter)
    } else {
        if cfg.s88.enabled {
            eprintln!(
                "event=s88 status=dry-run port_match={:?} knobs={}",
                cfg.s88.port,
                cfg.s88.knobs.len()
            );
        }
        None
    };
    validate_active_external_bindings(&rt, osc_adapter.as_ref(), &cfg.s88)?;
    let running = Arc::new(AtomicBool::new(true));
    let signal_flag = Arc::clone(&running);
    let runtime_thread = std::thread::current();
    let signal_runtime_thread = runtime_thread.clone();
    ctrlc::set_handler(move || {
        signal_flag.store(false, Ordering::Release);
        signal_runtime_thread.unpark();
    })
    .context("installing Ctrl-C handler")?;

    let (runtime_request_tx, runtime_request_rx) = crossbeam_channel::bounded(8);
    let mut server_handle = None;
    let mut server_thread = None;
    if server_config.enabled {
        let listen = server_config
            .listen
            .parse::<std::net::SocketAddr>()
            .with_context(|| format!("parsing server listen address {:?}", server_config.listen))?;
        let listener = std::net::TcpListener::bind(listen)
            .with_context(|| format!("binding control plane to {listen}"))?;
        listener
            .set_nonblocking(true)
            .context("configuring nonblocking control-plane listener")?;
        let local_addr = listener.local_addr()?;
        let (router, handle) = cr_server::build(
            authored_graph,
            runtime_request_tx,
            cr_server::ServerConfig {
                persistence_path: Some(state_path.clone()),
                runtime_waker: Some(runtime_thread.clone()),
                catalog: build_catalog(&cfg, &packs)?,
                rack_copilot: cfg.ai.clone(),
            },
        );
        handle.update_io(collect_io_telemetry(
            &midi_monitors,
            osc_adapter.as_ref(),
            s88_adapter.as_ref(),
            &cfg,
            cli.dry_run,
        ));
        handle.publish_runtime(0, rt.node_values(), rt.stats(), true);
        server_thread = Some(cr_server::spawn(router, listener, Arc::clone(&running)));
        server_handle = Some(handle);
        eprintln!("event=server status=started listen={local_addr} state_file={state_path:?}");
    } else {
        drop(runtime_request_tx);
        eprintln!("event=server status=disabled");
    }

    eprintln!(
        "event=runtime status=started patch={:?} control_rate_hz={CONTROL_RATE_HZ} batch_ticks={} report_interval_ticks={} parameter_min_interval_ticks={} midi_max_messages_per_second={} midi_burst_messages={} mode={}",
        cli.patch,
        timing.batch_ticks,
        timing.report_interval_ticks,
        timing.parameter_min_interval_ticks,
        timing.midi_max_messages_per_second,
        timing.midi_burst_messages,
        if cli.dry_run { "dry-run" } else { "live" }
    );

    let start = Instant::now();
    let mut tick: u64 = 0;
    let mut next_report = timing.report_interval_ticks;
    let mut next_publish = server_config.publish_interval_ticks;
    let runtime_result = (|| -> Result<()> {
        while running.load(Ordering::Acquire) {
            let batch_end = tick.saturating_add(timing.batch_ticks);
            while tick < batch_end && running.load(Ordering::Acquire) {
                while let Ok(request) = runtime_request_rx.try_recv() {
                    let result = validate_compiled_external_bindings(
                        &request.compiled,
                        osc_adapter.as_ref(),
                        &cfg.s88,
                    )
                    .and_then(|()| rt.apply_graph(request.compiled))
                    .map_err(|e| e.to_string());
                    let _ = request.reply.send(result);
                }

                if let Some(adapter) = &osc_adapter {
                    while let Some(input) = adapter.try_recv() {
                        let _ = rt.set_external_input(input.binding.as_str(), input.value);
                    }
                }
                if let Some(adapter) = &s88_adapter {
                    while let Some(input) = adapter.try_recv() {
                        let _ = rt.set_external_input(input.binding.as_str(), input.value);
                    }
                }

                let count = (batch_end - tick).min(MAX_INTERRUPT_CHUNK_TICKS);
                rt.run_ticks(tick, count)?;
                tick = tick.saturating_add(count);

                if let Some(adapter) = &osc_adapter {
                    for (binding, value) in rt.external_outputs() {
                        let _ = adapter.send(binding, value);
                    }
                }

                if tick >= next_publish {
                    if let Some(handle) = &server_handle {
                        handle.update_io(collect_io_telemetry(
                            &midi_monitors,
                            osc_adapter.as_ref(),
                            s88_adapter.as_ref(),
                            &cfg,
                            cli.dry_run,
                        ));
                        handle.publish_runtime(tick, rt.node_values(), rt.stats(), true);
                    }
                    next_publish = next_deadline(tick, server_config.publish_interval_ticks);
                }
            }

            if !running.load(Ordering::Acquire) {
                break;
            }

            if tick >= next_report {
                report_stats("running", tick, rt.stats());
                if let Some(adapter) = &osc_adapter {
                    report_osc_stats("running", tick, adapter.stats());
                }
                if let Some(adapter) = &s88_adapter {
                    report_s88_stats("running", tick, adapter.stats());
                }
                for monitor in &midi_monitors {
                    if let Some(status) = &monitor.status {
                        report_midi_connection(
                            "running",
                            tick,
                            &monitor.endpoint,
                            status.snapshot(),
                        );
                    }
                }
                next_report = next_deadline(tick, timing.report_interval_ticks);
            }

            // Pace against elapsed wall time so drift does not accumulate. The
            // engine's fixed 1 kHz rate means one tick is exactly one millisecond.
            let target = start
                .checked_add(Duration::from_millis(tick))
                .context("runtime duration exceeded the platform clock range")?;
            if let Some(delay) = target.checked_duration_since(Instant::now()) {
                std::thread::park_timeout(delay);
            }
        }
        Ok(())
    })();

    running.store(false, Ordering::Release);
    if let Some(handle) = &server_handle {
        handle.update_io(collect_io_telemetry(
            &midi_monitors,
            osc_adapter.as_ref(),
            s88_adapter.as_ref(),
            &cfg,
            cli.dry_run,
        ));
        handle.publish_runtime(tick, rt.node_values(), rt.stats(), false);
    }
    if let Some(thread) = server_thread {
        match thread.join() {
            Ok(result) => result.context("control-plane server failed")?,
            Err(_) => bail!("control-plane server thread panicked"),
        }
    }
    report_stats("stopped", tick, rt.stats());
    if let Some(adapter) = &osc_adapter {
        report_osc_stats("stopped", tick, adapter.stats());
    }
    if let Some(adapter) = &s88_adapter {
        report_s88_stats("stopped", tick, adapter.stats());
    }
    for monitor in &midi_monitors {
        if let Some(status) = &monitor.status {
            report_midi_connection("stopped", tick, &monitor.endpoint, status.snapshot());
        }
    }
    runtime_result
}

fn resolve_beside(base: &Path, configured: &str) -> PathBuf {
    let path = Path::new(configured);
    if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    }
}

fn next_deadline(current: u64, interval: u64) -> u64 {
    current
        .checked_div(interval)
        .and_then(|period| period.checked_add(1))
        .and_then(|period| period.checked_mul(interval))
        .unwrap_or(u64::MAX)
}

fn report_stats(status: &str, tick: u64, stats: cr_midi::BudgetedLimiterStats) {
    eprintln!(
        "event=runtime status={status} tick={tick} sent_updates={} sent_messages={} deduped={} parameter_rate_limited={} bus_rate_limited={} bus_rate_limited_messages={} unknown_slot={}",
        stats.sent,
        stats.sent_messages,
        stats.deduped,
        stats.parameter_rate_limited,
        stats.bus_rate_limited,
        stats.bus_rate_limited_messages,
        stats.unknown_slot
    );
}

fn build_catalog(cfg: &config::PatchConfig, packs: &PackRegistry) -> Result<cr_server::Catalog> {
    let mut devices = Vec::new();
    for device in cfg.all_devices() {
        let pack = packs
            .get(&device.pack)
            .with_context(|| format!("building catalog for device `{}`", device.id))?;
        let mut parameters: Vec<_> = pack
            .parameters
            .iter()
            .map(|(id, parameter)| cr_server::ParameterCatalogEntry {
                id: id.clone(),
                min: parameter.min,
                max: parameter.max,
                encoding: match parameter.encoding {
                    Encoding::Cc7 { .. } => "cc7",
                    Encoding::Cc14 { .. } => "cc14",
                    Encoding::Nrpn { .. } => "nrpn",
                }
                .to_owned(),
            })
            .collect();
        parameters.sort_by(|left, right| left.id.cmp(&right.id));
        devices.push(cr_server::DeviceCatalogEntry {
            id: device.id.as_str().to_owned(),
            display_name: pack.display_name.clone(),
            pack: pack.name.clone(),
            port: device.port.clone(),
            channel: device.channel,
            parameters,
        });
    }
    devices.sort_by(|left, right| left.id.cmp(&right.id));

    let mut external_inputs = Vec::new();
    let mut external_outputs = Vec::new();
    for binding in &cfg.osc.bindings {
        let entry = cr_server::BindingCatalogEntry {
            id: binding.id.as_str().to_owned(),
            transport: "osc".into(),
            min: binding.min,
            max: binding.max,
        };
        match binding.direction {
            cr_osc::Direction::Input => external_inputs.push(entry),
            cr_osc::Direction::Output => external_outputs.push(entry),
        }
    }
    if cfg.s88.enabled {
        external_inputs.extend(
            cfg.s88
                .knobs
                .iter()
                .map(|knob| cr_server::BindingCatalogEntry {
                    id: knob.id.as_str().to_owned(),
                    transport: "s88".into(),
                    min: 0.0,
                    max: 1.0,
                }),
        );
    }
    external_inputs.sort_by(|left, right| left.id.cmp(&right.id));
    external_outputs.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(cr_server::Catalog {
        modules: cr_core::builtin_module_catalog(),
        devices,
        external_inputs,
        external_outputs,
    })
}

fn collect_io_telemetry(
    midi: &[MidiMonitor],
    osc: Option<&cr_osc::OscAdapter>,
    s88: Option<&cr_s88::S88Adapter>,
    cfg: &config::PatchConfig,
    dry_run: bool,
) -> Vec<cr_server::IoTelemetry> {
    let mut telemetry = Vec::new();
    for monitor in midi {
        let stats = monitor.status.as_ref().map(MidiConnectionStatus::snapshot);
        telemetry.push(cr_server::IoTelemetry {
            id: monitor.device_id.clone(),
            label: monitor.device_id.clone(),
            transport: "midi_out".into(),
            endpoint: monitor.endpoint.clone(),
            state: if monitor.dry_run {
                cr_server::IoConnectionState::DryRun
            } else if stats.is_some_and(|stats| stats.connected) {
                cr_server::IoConnectionState::Connected
            } else {
                cr_server::IoConnectionState::Disconnected
            },
            connect_attempts: stats.map_or(0, |stats| stats.connect_attempts),
            reconnects: stats.map_or(0, |stats| stats.reconnects),
            received: 0,
            sent: stats.map_or(0, |stats| stats.sent_messages),
            errors: stats.map_or(0, |stats| stats.connect_errors + stats.send_errors),
            dropped: stats.map_or(0, |stats| stats.dropped_messages),
        });
    }
    if cfg.osc.enabled {
        let stats = osc.map(cr_osc::OscAdapter::stats).unwrap_or_default();
        telemetry.push(cr_server::IoTelemetry {
            id: "osc".into(),
            label: "OSC UDP".into(),
            transport: "osc".into(),
            endpoint: osc
                .map(|adapter| adapter.local_addr().to_string())
                .unwrap_or_else(|| cfg.osc.listen.clone()),
            state: if osc.is_some() {
                cr_server::IoConnectionState::Connected
            } else {
                cr_server::IoConnectionState::Disconnected
            },
            connect_attempts: u64::from(osc.is_some()),
            reconnects: 0,
            received: stats.received,
            sent: stats.sent,
            errors: stats.malformed + stats.type_errors + stats.send_errors,
            dropped: stats.input_dropped + stats.output_dropped,
        });
    }
    if cfg.s88.enabled {
        let stats = s88.map(cr_s88::S88Adapter::stats).unwrap_or_default();
        telemetry.push(cr_server::IoTelemetry {
            id: "s88".into(),
            label: "Kontrol S88 MK3".into(),
            transport: "s88".into(),
            endpoint: cfg.s88.port.clone(),
            state: if dry_run {
                cr_server::IoConnectionState::DryRun
            } else if stats.connected {
                cr_server::IoConnectionState::Connected
            } else {
                cr_server::IoConnectionState::Disconnected
            },
            connect_attempts: stats.connect_attempts,
            reconnects: stats.reconnects,
            received: stats.received,
            sent: 0,
            errors: stats.unmatched,
            dropped: stats.dropped,
        });
    }
    telemetry.sort_by(|left, right| left.id.cmp(&right.id));
    telemetry
}

fn report_osc_stats(status: &str, tick: u64, stats: cr_osc::OscStats) {
    eprintln!(
        "event=osc status={status} tick={tick} received={} malformed={} type_errors={} unmatched={} input_dropped={} sent={} deduped={} rate_limited={} send_errors={} output_dropped={}",
        stats.received,
        stats.malformed,
        stats.type_errors,
        stats.unmatched,
        stats.input_dropped,
        stats.sent,
        stats.deduped,
        stats.rate_limited,
        stats.send_errors,
        stats.output_dropped,
    );
}

fn report_s88_stats(status: &str, tick: u64, stats: cr_s88::S88Stats) {
    eprintln!(
        "event=s88 status={status} tick={tick} connected={} connect_attempts={} reconnects={} received={} unmatched={} dropped={}",
        stats.connected,
        stats.connect_attempts,
        stats.reconnects,
        stats.received,
        stats.unmatched,
        stats.dropped,
    );
}

fn report_midi_connection(
    status: &str,
    tick: u64,
    port: &str,
    stats: cr_midi::MidiConnectionStats,
) {
    eprintln!(
        "event=midi status={status} tick={tick} port={port:?} connected={} connect_attempts={} reconnects={} connect_errors={} send_errors={} dropped_messages={}",
        stats.connected,
        stats.connect_attempts,
        stats.reconnects,
        stats.connect_errors,
        stats.send_errors,
        stats.dropped_messages,
    );
}

fn validate_active_external_bindings(
    runtime: &runtime::Runtime,
    adapter: Option<&cr_osc::OscAdapter>,
    s88: &cr_s88::S88Config,
) -> Result<()> {
    for binding in runtime.external_source_bindings() {
        anyhow::ensure!(
            adapter.is_some_and(|adapter| adapter.has_input(binding)) || s88.has_input(binding),
            "external source binding `{binding}` has no OSC or S88 input configuration"
        );
    }
    for binding in runtime.external_sink_bindings() {
        anyhow::ensure!(
            adapter.is_some_and(|adapter| adapter.has_output(binding)),
            "external sink binding `{binding}` has no OSC output configuration"
        );
    }
    Ok(())
}

fn validate_configured_external_bindings(
    runtime: &runtime::Runtime,
    osc: &cr_osc::OscConfig,
    s88: &cr_s88::S88Config,
) -> Result<()> {
    for binding in runtime.external_source_bindings() {
        let has_osc_input = osc.enabled
            && osc.bindings.iter().any(|candidate| {
                candidate.direction == cr_osc::Direction::Input && candidate.id.as_str() == binding
            });
        anyhow::ensure!(
            has_osc_input || s88.has_input(binding),
            "external source binding `{binding}` has no enabled OSC or S88 input configuration"
        );
    }
    for binding in runtime.external_sink_bindings() {
        let has_osc_output = osc.enabled
            && osc.bindings.iter().any(|candidate| {
                candidate.direction == cr_osc::Direction::Output && candidate.id.as_str() == binding
            });
        anyhow::ensure!(
            has_osc_output,
            "external sink binding `{binding}` has no enabled OSC output configuration"
        );
    }
    Ok(())
}

fn validate_compiled_external_bindings(
    graph: &CompiledGraph,
    adapter: Option<&cr_osc::OscAdapter>,
    s88: &cr_s88::S88Config,
) -> Result<()> {
    for node in graph.nodes() {
        match &node.spec {
            NodeSpec::ExternalSource { binding, .. } => anyhow::ensure!(
                adapter.is_some_and(|adapter| adapter.has_input(binding.as_str()))
                    || s88.has_input(binding.as_str()),
                "external source binding `{binding}` has no OSC or S88 input configuration"
            ),
            NodeSpec::Macro { binding, .. } => anyhow::ensure!(
                adapter.is_some_and(|adapter| adapter.has_input(binding.as_str()))
                    || s88.has_input(binding.as_str()),
                "macro binding `{binding}` has no OSC or S88 input configuration"
            ),
            NodeSpec::ExternalSink { binding } => anyhow::ensure!(
                adapter.is_some_and(|adapter| adapter.has_output(binding.as_str())),
                "external sink binding `{binding}` has no OSC output configuration"
            ),
            NodeSpec::Lfo { .. }
            | NodeSpec::SampleHold { .. }
            | NodeSpec::StepSequence { .. }
            | NodeSpec::Slew { .. }
            | NodeSpec::Clamp { .. }
            | NodeSpec::Curve { .. }
            | NodeSpec::Quantize { .. }
            | NodeSpec::Math { .. }
            | NodeSpec::Envelope { .. }
            | NodeSpec::Scale { .. }
            | NodeSpec::Sink { .. }
            | NodeSpec::Takeover { .. } => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli> {
        Cli::parse(args.iter().map(|arg| (*arg).to_string()))
    }

    #[test]
    fn preserves_default_and_positional_config_invocation() {
        assert_eq!(parse(&[]).unwrap().patch, DEFAULT_PATCH);
        assert_eq!(parse(&["custom.yaml"]).unwrap().patch, "custom.yaml");
        assert_eq!(
            parse(&["--dry-run", "custom.yaml"]).unwrap(),
            Cli {
                patch: "custom.yaml".into(),
                dry_run: true,
                check: false,
                preflight: false,
                list_ports: false,
                batch_ticks: None,
                report_interval_ticks: None,
                parameter_min_interval_ticks: None,
                midi_max_messages_per_second: None,
                midi_burst_messages: None,
                listen: None,
                state_file: None,
                no_server: false,
                action: Action::Run,
            }
        );
    }

    #[test]
    fn parses_operational_overrides() {
        let cli = parse(&[
            "--batch-ticks",
            "20",
            "--report-interval-ticks",
            "2000",
            "--parameter-min-interval-ticks",
            "7",
            "--midi-max-messages-per-second",
            "250",
            "--midi-burst-messages",
            "5",
            "--listen",
            "127.0.0.1:9999",
            "--state-file",
            "saved.yaml",
            "--no-server",
        ])
        .unwrap();
        assert_eq!(cli.batch_ticks, Some(20));
        assert_eq!(cli.report_interval_ticks, Some(2_000));
        assert_eq!(cli.parameter_min_interval_ticks, Some(7));
        assert_eq!(cli.midi_max_messages_per_second, Some(250));
        assert_eq!(cli.midi_burst_messages, Some(5));
        assert_eq!(cli.listen.as_deref(), Some("127.0.0.1:9999"));
        assert_eq!(cli.state_file.as_deref(), Some("saved.yaml"));
        assert!(cli.no_server);
    }

    #[test]
    fn rejects_unknown_conflicting_and_malformed_arguments() {
        assert!(
            parse(&["--wat"])
                .unwrap_err()
                .to_string()
                .contains("unknown option")
        );
        assert!(
            parse(&["a.yaml", "b.yaml"])
                .unwrap_err()
                .to_string()
                .contains("multiple")
        );
        assert!(
            parse(&["--batch-ticks", "0"])
                .unwrap_err()
                .to_string()
                .contains("positive")
        );
        assert!(
            parse(&["--dry-run", "--list-ports"])
                .unwrap_err()
                .to_string()
                .contains("mutually exclusive")
        );
        assert!(parse(&["--check", "--dry-run"]).is_err());
        assert!(parse(&["--preflight", "--list-ports"]).is_err());
        assert!(
            parse(&["--listen"])
                .unwrap_err()
                .to_string()
                .contains("requires")
        );
    }

    #[test]
    fn double_dash_allows_a_dash_prefixed_patch_path() {
        assert_eq!(parse(&["--", "-patch.yaml"]).unwrap().patch, "-patch.yaml");
    }

    #[test]
    fn parses_non_io_check_mode() {
        let cli = parse(&["--check", "config/osc-bridge.yaml"]).unwrap();
        assert!(cli.check);
        assert!(!cli.dry_run);
        assert!(!cli.list_ports);
        assert!(!cli.preflight);
    }

    #[test]
    fn parses_hardware_preflight_mode() {
        let cli = parse(&["--preflight", "config/s88-takeover.yaml"]).unwrap();
        assert!(cli.preflight);
        assert!(!cli.check);
        assert!(!cli.dry_run);
    }

    #[test]
    fn discard_sink_does_not_retain_messages() {
        let mut sink = DiscardSink;
        for _ in 0..10_000 {
            sink.send(&[0xb0, 29, 64]).unwrap();
        }
        assert_eq!(std::mem::size_of_val(&sink), 0);
    }
}
