# Control Room execution roadmap

**Updated:** 2026-08-25  
**Branch:** `feat/m1-control-room`  
**Policy:** software completion and hardware verification are tracked separately. An automated test never closes a hardware gate.

## Current state

Control Room has a deterministic 1 kHz graph engine, a 64-module server-backed control library with typed patch points, validated semantic device packs, portable CC7/CC14/NRPN output, per-parameter and aggregate MIDI budgeting, multi-device/port routing with output reconnection, strict operational configuration, graceful shutdown, transactional graph commands, browser and native macOS editors, OSC I/O, a built and live-verified eight-lane VCV Rack bridge, a staged Rack AI Copilot, standard-template S88 input with pickup takeover, and appliance packaging.

The workspace test, formatting, and warning-denied Clippy suites are green. The current machine does not expose a Novation Peak MIDI port, so M1's audible hardware acceptance gate remains open.

## Milestone status

| Milestone | Status | Software completion gate | Hardware/manual gate |
|---|---|---|---|
| M1 — engine + MIDI | Software complete | Config → graph → engine → pack → limiter → portable MIDI messages; operational hardening and multi-device routing | Peak responds to the documented controls and the modulation feels smooth |
| M2 — control plane + browser | Software complete | Embedded UI inspects, edits, validates, applies, and persists through the common API; live meters and errors stream over WebSocket | Browser interaction is usable during an actual running hardware patch |
| M2N — native macOS surface | Software complete | SwiftUI signal field connects over the common API, edits every node kind, streams telemetry, and applies transactionally; signed local app bundle builds cleanly | Extended studio-use UX pass on the target Mac and hardware patch |
| M2S — device-aware studio workflow | Software complete | Catalog-backed routing, unified adapter status, validation, undo/redo, atomic patch documents, recents, and explicit crash recovery | Extended studio session on connected instruments |
| M6 — musical modulation toolbox | Software complete | Deterministic sources, finite transforms, ADSR, macros, API/editor parity, persistence, and golden tests | Musical feel and ergonomics during a connected-instrument session |
| M7 — module library + Rack bridge | Software complete | 64 shared module definitions, typed ports, searchable native/browser libraries, direct jack patching, Rack 2.6.6 ARM64 build/render, and live bidirectional OSC session | Exercise the bridge in an extended musical Rack patch and collect subjective latency/feel notes |
| M8 — Rack AI Copilot | Foundation complete; live-GUI audition gate open | Live inventory protocol, first-class OpenAI/Claude/Grok planners, capability-restricted ACP v1, double validation, stale-state protection, provider-aware previews, explicit Apply, guarded native Rack Undo, and provider contract tests | Re-authenticate Claude, activate Rack's normal GUI session, audition real provider proposals, and confirm Apply/Undo feel |
| M3 — OSC | Software complete | OSC input/output are first-class endpoints; bindings, numeric types, ranges, budgets, malformed input, and UDP loopback are tested | Interoperates with the intended external OSC client in the studio |
| M4 — Kontrol S88 surface | Foundation complete; display hardware-gated | Transport-neutral replay, standard-template CC input, reconnect monitoring, module banks, shared command batches, pickup takeover, and no-jump tests | S88 knobs work on hardware; proprietary labels/values require verified captures before activation |
| M5 — appliance | Packaging complete; target gate open | Native aarch64 CI path, service hardening, explicit paths, Avahi mDNS, readiness check, and recoverable lifecycle scripts | Boots unattended on the target appliance and is reachable at `controlroom.local` |
| Post-v1 — MK3 framebuffer | Research gate | Capture/diff tooling and a documented protocol notebook | A test image is rendered safely on MK3 hardware |

## M2 — control plane and browser editor

Implementation order:

1. Add `cr-server` with HTTP health/state endpoints and a versioned WebSocket protocol.
2. Add a runtime control channel; graph batches apply atomically at tick boundaries.
3. Publish coalesced graph values, runtime state, limiter statistics, and command results at approximately 30 Hz. Adapter connection counters remain in structured runtime diagnostics until a unified device-status schema is added.
4. Add persistence using atomic file replacement; never overwrite the last valid patch with an invalid edit.
5. Add an embedded browser client with a node palette, draggable canvas, connections, parameter editing, live meters, validation feedback, explicit Apply, and dirty-state indication.
6. Verify API semantics and the full config/server/runtime/browser path without MIDI hardware.

Completion gate: a user can open the bundled UI, construct the flagship patch by dragging, receive validation feedback, explicitly apply it, reload the page, and observe live values while the original valid graph remains active after any rejected edit.

## M2N — native macOS signal workstation

The native SwiftUI client is the primary close-up studio surface. It uses the same versioned snapshot, WebSocket telemetry, and command-batch endpoints as the browser. The application includes a draggable grid, persisted per-server layouts, semantic module inspector, animated meters, signal and pickup-control wires, endpoint reconnection, stale-revision conflict protection, Apply/Discard, keyboard commands, and deterministic app icon/bundle packaging.

Completion gate: the packaged app connects to a live runtime, stages and discards graph edits locally, applies a valid edit at a tick boundary, observes the new revision, and remains fully usable without browser rendering. This software gate is complete; extended real-hardware session feedback remains open.

## M2S — device-aware studio workflow

Implementation order:

1. Publish configured devices, semantic parameters, ranges, encodings, and external bindings through a stable catalog endpoint.
2. Normalize MIDI, OSC, and S88 connection/error/drop counters into one snapshot schema and render real adapter health in the native sidebar.
3. Replace free-text device, parameter, and binding entry with catalog-backed native controls while retaining an unavailable-value escape hatch for repair.
4. Validate staged graphs locally and add bounded undo/redo without weakening authoritative server validation or explicit Apply.
5. Add native New/Open/Save As, recent patches, named snapshots, and crash-safe draft recovery. Complete.

Completion gate: a user can discover a configured device, select a semantic parameter without typing IDs, build and validate a patch, undo/redo changes, save it under a chosen name, recover an interrupted draft, and see adapter disconnect/reconnect state without reading logs. The software gate is complete and has a real native interaction pass; connected-instrument studio feedback remains open.

## M6 — musical modulation toolbox

Implementation order:

1. Add deterministic sample-and-hold/random and step-sequence sources. Complete.
2. Add slew, clamp, curve, quantize, and math transforms with total finite behavior. Complete.
3. Add an ADSR envelope driven by any graph signal and macro controls using the existing external-input seam. Complete.
4. Expose every node through the command API, browser, native palette/inspector, validation, persistence, and golden engine tests. Complete.

Completion gate: useful evolving patches can be built without an external sequencer, every new node remains deterministic and allocation-free at tick time, and the native editor can author and validate every parameter without raw configuration editing. The software gate is complete; musical feel remains part of the connected-instrument studio pass.

## M7 — module library and VCV Rack bridge

The product-level module catalog contains 64 named modules across Modulators, Sequencers, Shapers, Utilities, Envelopes, Performance, and I/O. Each module publishes its default graph specification and explicit typed ports through `/api/v1/catalog`; the native and browser editors use the same data rather than maintaining separate inventories. `IN`, `GATE`, `PICKUP`, and `OUT` jacks are visible patch targets, and direct jack patching updates the same staged graph used by inspector routing and transactional Apply.

`integrations/vcv-control-room` is a Rack 2 plugin source tree for Bridge 8. Eight Rack `0–10 V` lanes normalize to Control Room `0–1` OSC inputs on UDP 9100, while eight Control Room outputs return on UDP 9101. Rack's audio callback only touches atomics; a bounded-cadence worker owns networking. `config/vcv-bridge.yaml` declares all sixteen bindings, and the OSC codec/transport builds and tests independently of Rack.

Completion gate: the catalog/API/editors agree on every module and port, a user can patch by jacks in both editors, and Bridge 8 passes an actual Rack 2 build/load plus bidirectional patch session. All software gates are complete: Bridge 8 builds with the official Rack 2.6.6 macOS ARM64 SDK, renders through Rack's module screenshot mode, and passes an isolated headless session with traffic in both directions and zero bridge errors. The integration does not load arbitrary `.vcvplugin` binaries into Control Room.

## M8 — Rack AI Copilot

The first end-to-end Copilot slice is implemented. Bridge 8 exposes a localhost TCP agent that inventories installed Rack models, semantic parameters and ports, current modules, cables, and positions. Network work is isolated from both Rack's audio callback and UI mutation. Control Room can send the live inventory and user intent to OpenAI Responses, capability-restricted Claude Code or Grok Build structured-output processes, or a local ACP v1 agent over stdio JSON-RPC. Every path feeds the same parser, validator, proposal store, and Apply boundary. Only installed-module addition, in-range parameter changes, and valid cable connections are expressible.

Both the browser and native macOS app show the provider, plan, warnings, exact commands, and non-installed library search ideas before Apply. Proposals are stored server-side by opaque ID, so a client cannot swap in arbitrary commands. Apply rechecks the live fingerprint, and Rack performs a second full validation on its UI thread before pushing one native history action. Guarded Undo refuses to touch later user work. OpenAI keys remain in the Control Room process environment. Claude and Grok run in temporary workspaces with tools disabled; Grok additionally receives an isolated configuration home rather than the user's plugin/skill/MCP context. ACP advertises no client filesystem/terminal capabilities, provides no MCP servers, cancels permission requests, and defaults to an isolated temporary working directory. Library suggestions are never installed or purchased automatically.

Software evidence includes structured-plan unit tests, rejection of hallucinated models/out-of-range parameters, provider-specific OpenAI/Claude/Grok mock processes that reach the shared proposal boundary, an ACP v1 negotiation/streaming/permission-cancellation test, warning-denied Rust checks, Rack SDK compilation, and native app tests. A live isolated Grok structured-output call passes. OpenAI cannot run live until `OPENAI_API_KEY` is supplied, and the installed Claude CLI reports a revoked OAuth token; both failures are surfaced without falling back or mutating Rack. Rack's DSP-only headless mode correctly does not run UI mutation. The remaining product gate is a normal activated Rack GUI session with real provider credentials, followed by human audition of the resulting patch and its Undo behavior.

## M3 — OSC adapter

Implementation order:

1. Add `cr-osc` using a bounded nonblocking input queue and a dedicated UDP I/O task.
2. Add OSC source and sink node specifications without leaking OSC types into the engine.
3. Define pack/config bindings for address, numeric argument index, direction, range, and peer.
4. Apply deduplication and output budgets with visible suppression accounting.
5. Cover encode/decode, malformed packets, wrong typetags, queue saturation, and UDP loopback.

Completion gate: an external OSC value can drive a graph and an engine signal can reach an external OSC listener, with deterministic tests and observable failure states.

Acceptance evidence: `scripts/osc-smoke.sh` now exercises the OSC client as an independent process, proves raw-range input normalization through the HTTP meter, receives outbound floats, and checks final adapter counters. Interoperation with the chosen studio client and network topology remains a manual session gate.

## M4 — Kontrol S88 MK3 surface

Implementation order:

1. Add `cr-s88` behind explicit transport traits so captured traffic can be replayed without hardware.
2. Implement device discovery/reconnect and the verified standard MIDI-template subset first. Keep the DAW-port/display path disabled behind the unverified transport boundary.
3. Model surface modules and banks as clients of the same graph command/state API used by the browser.
4. Implement pickup takeover as an engine transform with explicit ownership state and no-jump tests.
5. Capture/replay tooling, a versioned fixture schema, byte-offset diffing, and the protocol notebook
   are complete. The checked-in example is explicitly synthetic; physical byte-exact fixtures remain
   hardware-gated, and every inferred sequence stays labeled `verify against hardware`.
6. Perform the manual S88 acceptance sequence before claiming completion.

Readiness evidence: `controlroom --preflight config/s88-takeover.yaml` resolves both physical ports without opening them; `scripts/s88-acceptance.sh` records input-only evidence and rejects captures missing any of the eight knobs' min/mid/max positions. Pickup feel, banking, reconnect, and display activation remain direct-observation gates.

Completion gate: eight knobs show meaningful parameter labels/values, bank predictably, and take over modulated values without a jump. The software model, standard MIDI input, and takeover are complete; display bytes and the end-to-end claim remain blocked on hardware capture. Pixel-level framebuffer work remains post-v1.

## M5 — appliance deployment

Implementation order:

1. Make all filesystem paths and network listeners explicit and relocatable.
2. Add Linux/aarch64 build verification and document required ALSA/udev dependencies.
3. Add a hardened systemd unit with restart policy, readiness/health checks, restricted privileges, and persistent configuration/state directories.
4. Add mDNS advertisement for `controlroom.local` and the HTTP service.
5. Add idempotent install, upgrade, and uninstall scripts with recoverable backups.
6. Keep the engine alive across MIDI output send failures, retry the exact resolved port, and report reconnect/drop counters.
7. Run an unattended reboot/reconnect acceptance test on the target appliance.

Completion gate: after power-on, Control Room reconnects its devices, serves the UI at `controlroom.local`, reports health, and survives process/device/network restarts without corrupting the last valid patch.

Acceptance evidence: `scripts/appliance-acceptance.sh` records the target architecture, boot ID, installed-config validation, systemd/Avahi state, localhost and mDNS health, runtime state, optional service restart, and state checksum preservation. Supplying the previous boot ID proves an actual reboot occurred; physical power/device/network recovery and musical correctness remain human observations.

## Cross-cutting release gates

- Invalid graph edits are rejected before the engine and leave the active graph unchanged.
- MIDI/OSC/S88 I/O is bounded; suppression and disconnection are observable.
- The 1 kHz engine path remains deterministic, panic-free, and allocation-free after graph application.
- UI and surface proposals require explicit Apply; clients do not silently change the active patch.
- Persistence uses full-file atomic replacement and retains a last-known-good patch.
- Tests, formatting, warning-denied Clippy, and relevant smoke checks pass at every milestone.
- Hardware claims name the exact device and verification procedure.
- `controlroom --check PATCH` is the zero-I/O configuration gate; `--preflight PATCH` additionally resolves all configured physical ports and probes listeners without sending MIDI or OSC.
- `scripts/acceptance-preflight.sh --report FILE` produces non-overwriting readiness evidence and returns exit code 2 when software passes but hardware is absent.
- `scripts/studio-acceptance.sh` runs the connected Mac preflight, OSC, Peak, and S88 evidence sequence into one non-overwriting directory; `--run-live` is required before it transmits the Peak sweep, and direct observations remain manual.
- `scripts/verify-macos-release.sh ARCHIVE` verifies the checksum, safe archive layout, signed native app, bundled utilities, configuration checks, and independent OSC boundary from a clean extraction.
