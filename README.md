# Control Room

Control Room is a headless semantic modulation and routing engine for electronic music hardware. It lets a patch refer to `peak.filter.cutoff` instead of embedding MIDI CC and NRPN numbers throughout the graph.

The current implementation runs a deterministic 1 kHz graph, resolves device parameters through YAML packs, applies per-parameter and aggregate MIDI-bus budgets, and serves native and embedded browser editors. Its server-backed library contains 64 musical control modules across seven categories, built from deterministic LFO, seeded sample-and-hold, step sequence, ADSR, macro, slew, clamp, curve, quantize, scale, arithmetic, pickup, and adapter primitives. OSC and standard MIDI-template controls can enter the graph as external sources; OSC can also receive graph output. The included demo sweeps a Novation Peak filter cutoff with a software triangle LFO.

Kontrol S88 MK3 standard MIDI-template input and appliance packaging are present, while physical keyboard/display, synth, and Raspberry Pi acceptance gates remain explicit. See the live [execution roadmap](docs/ROADMAP.md) for exact status and the [design specification](docs/superpowers/specs/2026-08-10-control-room-design.md) for the architecture.

## Requirements

- Rust 1.96 or newer
- A MIDI output visible to the operating system for live use
- Novation Peak hardware for the included demo's manual acceptance test

## Quick start

```sh
cargo test --workspace
cargo run -p controlroom -- --check config/lfo-cutoff.yaml
cargo run -p controlroom -- --list-ports
cargo run -p controlroom -- --dry-run config/lfo-cutoff.yaml
cargo run -p controlroom -- config/lfo-cutoff.yaml
```

`--dry-run` evaluates and accounts for messages without opening a MIDI port or retaining an ever-growing capture buffer. Stop a running patch with Ctrl-C or a normal termination signal; Control Room prints final limiter statistics before exiting.

`--check` validates the complete graph, packs, external bindings, and runtime budgets without opening any I/O. `--preflight` adds exact MIDI input/output resolution plus temporary OSC/HTTP listener probes; it exits unsuccessfully and names every unavailable requirement instead of treating absent hardware as a pass.

The browser editor is available at [http://127.0.0.1:7777](http://127.0.0.1:7777) while the process runs. It stages edits locally and only changes the live engine after Apply succeeds at a tick boundary. Run `cargo run -p controlroom -- --help` for timing, persistence, listener, and MIDI-budget overrides.

## Native macOS interface

The flagship desktop surface is a native SwiftUI app in [`native/ControlRoom`](native/ControlRoom). It connects to the same HTTP/WebSocket control plane as the browser, so it can operate a local engine or a remote appliance without embedding the engine in the UI.

Start the engine, build the signed local app bundle, and open it:

```sh
cargo run -p controlroom -- --dry-run config/lfo-cutoff.yaml
scripts/build-native-app.sh
open "target/Control Room.app"
```

The native signal field provides draggable modules, animated live meters, a searchable categorized module library, explicit typed `IN`, `GATE`, `PICKUP`, and `OUT` patch jacks, catalog-backed device/parameter selection, unified MIDI/OSC/S88 status, inline staged-graph validation, bounded undo/redo, keyboard commands, and explicit Apply/Discard. Click an output jack and then an input jack to patch them; clicking a connected input with no pending cable disconnects it. `Command-Return` applies a valid staged patch transactionally. The server address at the bottom left accepts localhost or a studio appliance such as `http://controlroom.local:7777`.

The app remembers its server and per-server canvas layout. If another client advances the patch while local edits are staged, it shows a revision conflict and blocks Apply until the stale draft is discarded. A complete ad-hoc-signed development archive with SHA-256 checksum can be produced with `scripts/package-macos-release.sh`.

The release archive is self-contained for a Mac studio session. It includes the signed native app, engine, MIDI monitor, S88 capture utility, example patches, deployment assets, documentation, and acceptance scripts. After unpacking it, the documented `scripts/acceptance-preflight.sh`, `scripts/peak-acceptance.sh`, `scripts/s88-acceptance.sh`, and `scripts/osc-smoke.sh` commands use the bundled release binaries automatically; a Rust toolchain or source checkout is not required. From a source checkout, verify an archive exactly as CI does:

```sh
scripts/package-macos-release.sh
scripts/verify-macos-release.sh target/dist/controlroom-*.tar.gz
```

Every modulation node is available from the native palette and inspector. Source modules omit irrelevant input routing, envelope and pickup dependencies use distinct dashed wires, step values are edited directly, and invalid rates, ranges, cycles, bindings, ADSR values, or division by zero block Apply before the server performs its authoritative validation.

## VCV Rack bridge

[`integrations/vcv-control-room`](integrations/vcv-control-room) contains an original Rack 2 `Bridge 8` plugin scaffold with eight `0–10 V` send inputs and eight return outputs. It normalizes those lanes to Control Room's `0–1` domain over loopback OSC using [`config/vcv-bridge.yaml`](config/vcv-bridge.yaml). Rack remains the audio-rate synthesis host, so any Rack module can feed or receive Control Room lanes without attempting to load third-party Rack binaries into the Rust process.

On macOS, `scripts/install-vcv-sdk.sh` downloads the official SDK matching the installed Rack app. `scripts/verify-vcv-bridge.sh` then builds Bridge 8 and proves both OSC directions through an isolated headless Rack patch.

```sh
cargo run -p controlroom -- --dry-run config/vcv-bridge.yaml
export RACK_DIR=/absolute/path/to/Rack-SDK
make -C integrations/vcv-control-room
make -C integrations/vcv-control-room install
```

The Rack audio callback performs only atomic control-lane reads/writes; UDP and OSC encoding run on a worker thread. The standalone bridge codec is warning-clean and tested locally. Building the Rack-linked plugin requires the matching official Rack 2 SDK; the install script keeps it in ignored build output rather than vendoring it. See the integration README for ports, protocol, and the explicit distribution-license gate.

## Rack Copilot

Bridge 8 also exposes a localhost-only Rack control agent on TCP `9102`. With Rack open normally and Bridge 8 present in the patch, Control Room can inventory installed Rack models and their semantic ports/parameters, inspect the current modules and cables, and ask an AI service for a constrained command plan. The plan can only add installed modules, set in-range parameters, and connect valid ports. Control Room validates it before preview, Rack validates it again before mutation, and a patch fingerprint rejects proposals made against stale state.

AI is opt-in. Rack Copilot has first-class OpenAI, Claude, and Grok providers, plus a generic [Agent Client Protocol](https://agentclientprotocol.com/) v1 adapter. Provider credentials stay with the selected provider: OpenAI reads its key from the environment, while Claude and Grok use their installed CLI authentication. Credentials are never written into a patch, returned by the API, or sent to Rack. The VCV companion configuration [`config/vcv-bridge.yaml`](config/vcv-bridge.yaml) opts in explicitly:

```yaml
ai:
  enabled: true
  provider: openai
  model: gpt-5.4-mini
  rack_agent: 127.0.0.1:9102
```

Claude Code and Grok Build can be selected without placing provider keys in the patch:

```yaml
# Claude Code 2.1+
ai:
  enabled: true
  provider: claude
  rack_agent: 127.0.0.1:9102
  claude:
    command: claude
    model: sonnet # optional; omit to use the CLI default
  timeout_seconds: 120
```

```yaml
# Grok Build 1.0+
ai:
  enabled: true
  provider: grok
  rack_agent: 127.0.0.1:9102
  grok:
    command: grok
    model: grok-4.6 # optional; omit to use the CLI default
  timeout_seconds: 120
```

Both CLI adapters request JSON-schema structured output and run in dedicated temporary workspaces. Claude uses safe mode with no tools, slash commands, or session persistence. Grok gets an isolated configuration home with vendor compatibility, tools, MCP access, memory, subagents, web access, and update checks disabled. If Grok uses cached CLI authentication, only its authentication file is copied into that owner-only temporary home and removed when the proposal finishes.

The browser and native Copilot panels include a planner picker. **Default** uses `ai.provider` from the patch configuration; OpenAI, Claude, Grok, and ACP can also be selected per Draft without restarting Control Room.

For ACP, select any trusted stdio ACP v1 agent. Current Gemini CLI is the default example:

```yaml
ai:
  enabled: true
  provider: acp
  rack_agent: 127.0.0.1:9102
  acp:
    command: gemini
    args: [--acp]
    # cwd: /an/explicit/isolated/directory
  timeout_seconds: 120
```

Control Room negotiates ACP v1 over newline-delimited JSON-RPC, advertises no filesystem or terminal capabilities, supplies no MCP servers, and cancels permission requests. Unless `cwd` is explicitly configured, it launches the agent in a dedicated temporary directory. The ACP process is still a locally installed program with its own capabilities, so configure only an agent you trust.

The Gemini CLI currently installed on this development Mac is 0.25.1. It exposes only the older `--experimental-acp` mode, and that build did not complete a stable v1 handshake, so upgrade Gemini before using the `gemini --acp` example. This does not affect other conforming ACP v1 agents.

For OpenAI, launch from a shell that has `OPENAI_API_KEY` set. For Claude, run `claude auth login`; for Grok, run `grok login`; for ACP, authenticate the selected agent using its own documented flow. Then open the native app or [browser editor](http://127.0.0.1:7777) and use **Rack Copilot**. Every provider feeds the same `RackPlan` parser, inventory validator, opaque proposal store, and explicit Apply boundary. Reject discards the proposal; Undo is accepted only while the last Rack history action is the Copilot action, so it cannot silently undo later user work. Library suggestions are informational search terms only—Control Room never installs, purchases, or loads a suggested plugin.

Native patch documents use the `.controlroompatch` extension and are written atomically. New/Open/Save/Save As, recent files, staged undo/redo, and interrupted-draft recovery are available from the File and Edit menus. A recovered draft is never applied automatically; Restore only returns it to local staged state for review.

## Patch configuration

The existing single-device form remains valid. Its omitted `pack` defaults to the built-in `novation-peak` pack:

```yaml
device:
  id: peak
  port: Peak
  channel: 1

runtime:
  batch_ticks: 10
  report_interval_ticks: 5000
  parameter_min_interval_ticks: 10
  midi_max_messages_per_second: 300
  midi_burst_messages: 4

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

Additional devices can share a semantic pack, port, or neither:

```yaml
device:
  id: peak-left
  pack: novation-peak
  port: Peak
  channel: 1

devices:
  - id: peak-right
    pack: novation-peak
    port: Peak
    channel: 2
```

See [`config/dual-peak.yaml`](config/dual-peak.yaml) for a runnable dry-run example.

Port substrings must resolve to exactly one output; ambiguous matches are rejected. Devices whose substrings resolve to the same physical port share one connection and route on their configured channels. Device IDs must be unique. A live connection that fails degrades without stopping the engine, retries the exact resolved port every 500 ms, and reports connect, send-error, reconnect, and dropped-message counters.

To use community or local device models, list pack files relative to the patch file and select the pack's `name` from a device:

```yaml
pack_files:
  - packs/my-moog.yaml

device:
  id: moog
  pack: my-moog
  port: Moog
  channel: 1
```

Configuration rejects unknown fields, duplicate device IDs, invalid channels, missing packs, unknown parameters, and invalid graphs before the tick loop starts.

## Graph command API

`cr-core` exposes transport-neutral commands for adding, removing, updating, connecting, and disconnecting nodes. `apply_command` and `apply_commands` stage edits, compile the final graph, and commit only when the whole operation is valid. A failed edit leaves the original graph unchanged.

The HTTP/WebSocket server and S88 surface model use this common seam. It deliberately contains no server or UI assumptions.

## OSC and Kontrol S88

[`config/osc-bridge.yaml`](config/osc-bridge.yaml) demonstrates normalized OSC input and output with bounded queues, deduplication, rate limiting, and visible error accounting.

[`config/s88-takeover.yaml`](config/s88-takeover.yaml) maps an S88 standard MIDI template's eight knobs to CC 20-27. The `Takeover` node follows modulation until the physical knob moves across the current value, then latches to the knob without a jump. Input reconnects after the configured port disappears and returns.

The S88 display is intentionally behind an unverified transport boundary. Labels/banks have a semantic model, but proprietary DAW-port or framebuffer byte sequences will not be enabled until byte-exact hardware captures prove them.

The input-only research CLI records without opening a MIDI output, validates versioned fixtures, replays them through the adapter, and compares changed byte offsets:

```sh
cargo run -p cr-s88 --bin cr-s88-capture -- summarize \
  crates/cr-s88/fixtures/standard-template-cc.synthetic.yaml
cargo run -p cr-s88 --bin cr-s88-capture -- list
```

An S88 hardware capture can be accepted only when all eight consecutive CCs contain min, midpoint, and max evidence:

```sh
scripts/s88-acceptance.sh \
  --port "KONTROL S88 MK3" \
  --firmware 2.x.y \
  --output captures/s88-standard-template.yaml
```

The independent OSC probe verifies the real UDP boundary in a separate process, including raw-domain input normalization, HTTP meter publication, outbound OSC floats, and final adapter counters:

```sh
scripts/osc-smoke.sh
```

See [`docs/s88-protocol-notebook.md`](docs/s88-protocol-notebook.md) for safe capture steps and the manual hardware acceptance sequence.

## Raspberry Pi appliance

On 64-bit Raspberry Pi OS or another Debian aarch64 host, install `build-essential`, `pkg-config`, `libasound2-dev`, `avahi-daemon`, and either `curl` or `wget`, then build and install:

```sh
cargo build --workspace --locked --release
sudo scripts/install.sh --binary target/release/controlroom
```

The installer creates a restricted `controlroom` service account, persistent `/var/lib/controlroom` state, `/etc/controlroom/controlroom.yaml`, a hardened restarting systemd service, an HTTP readiness check, and Avahi advertisement at `controlroom.local:7777`. It preserves an existing configuration. Edit the MIDI port matches, then restart with `sudo systemctl restart controlroom`.

Cross-building from an x86_64 Linux host uses `rustup target add aarch64-unknown-linux-gnu`, an `aarch64-linux-gnu-gcc` toolchain, and target-architecture ALSA development libraries; `scripts/check-aarch64.sh` runs the configured build. CI also performs a native Linux/aarch64 release build.

Upgrades retain timestamped copies of the binary, configuration, and active state:

```sh
sudo scripts/upgrade.sh --binary target/release/controlroom
sudo scripts/uninstall.sh
```

Uninstall moves configuration and state to `/var/backups/controlroom` instead of deleting them. The service's default listener is intentionally studio-LAN HTTP with no authentication; do not expose port 7777 directly to the public internet.

On the installed Pi, produce a read-only acceptance report with:

```sh
scripts/appliance-acceptance.sh --report appliance-before.md
```

For the explicit service-restart test, run as root with `--restart-test`. To prove a real reboot rather than a process restart, record the first report's boot ID, reboot, then pass it to the second run with `--previous-boot-id`. The script checks aarch64, installed configuration, systemd/Avahi state, localhost and `controlroom.local` health, runtime state, and last-known-good state preservation; it leaves physical power, device, network, and musical observations clearly unrecorded.

## Acceptance and evidence

Run the unified preflight from the development checkout:

```sh
scripts/acceptance-preflight.sh --report acceptance.md
```

It validates every shipped patch, inventories MIDI endpoints, runs each hardware connectivity preflight, and creates a non-overwriting Markdown evidence report. Exit code `2` means software is valid but one or more physical requirements are absent. It never promotes an audible, tactile, reboot, or display claim from automation.

For Peak pack verification, the repository now includes a read-only MIDI input monitor and a guided evidence script:

```sh
cargo run -p cr-midi --bin cr-midi-monitor -- list
scripts/peak-acceptance.sh \
  --firmware VERSION \
  --report peak-acceptance.md \
  --run-live
```

The resulting report deliberately keeps every observation `NOT RECORDED` until a person confirms the exact instrument, firmware, CC behavior, sweep timing/range, and musical quality.

With both the Peak and S88 connected, run the complete evidence sequence in one command. `--run-live` is the explicit authorization for the script to transmit the Peak cutoff sweep; omit it for input-only preparation:

```sh
scripts/studio-acceptance.sh \
  --output-dir studio-acceptance-$(date +%Y%m%d) \
  --peak-firmware VERSION \
  --s88-firmware VERSION \
  --run-live
```

The output directory contains unified preflight, OSC, Peak, and S88 machine evidence plus `SESSION.md`. Machine results are recorded automatically, while every audible, tactile, reconnect, and intended-client observation remains explicitly `NOT RECORDED` until a person fills it in.

## Architecture

- `cr-core` — authoring graph, validation, topological compilation, graph commands
- `cr-engine` — deterministic allocation-free node evaluation
- `cr-packs` — semantic device models and pack registry
- `cr-midi` — encoding, output adapters, per-parameter and aggregate budgets
- `cr-osc` — bounded UDP OSC input/output adapter
- `cr-s88` — standard-template input, reconnect lifecycle, surface banks, display boundary
- `cr-server` — HTTP/WebSocket API, persistence, and bundled browser editor
- `native/ControlRoom` — native macOS signal workstation
- `controlroom` — configuration, device/port wiring, pacing, lifecycle, diagnostics

The aggregate token bucket counts actual wire messages: CC7 costs one, CC14 costs two, and NRPN costs four. Its default sustained ceiling is 300 messages per second with a four-message burst, shared across every graph sink. Startup rejects a burst allowance too small to carry any configured encoding atomically.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Automated tests verify the software pipeline and byte output. They do **not** prove that published Peak CC assignments behave correctly on physical hardware; that remains an explicit manual acceptance gate.
