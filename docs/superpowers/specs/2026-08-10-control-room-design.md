# Control Room — Design

**Date:** 2026-08-10
**Status:** Approved for planning
**Target hardware:** Native Instruments Kontrol S88 MK3 (firmware 2.x), Novation Peak, Moog

---

## 1. Purpose

Control Room is a headless semantic routing engine for electronic music hardware — a "MIDI control room." It lets a musician patch instruments together **by meaning** ("modulate the Peak's VCA with an LFO") rather than by CC and NRPN numbers, using a drag-and-drop graph in a browser.

It runs as an appliance: today on a laptop, later on a Raspberry Pi or Mac Mini that sits in the rack as "another instrument," with the browser as its only window.

### Why this exists

The Native Instruments Komplete Kontrol client is heavyweight and mandatory for features the hardware can do on its own. Separately, connecting hardware synths to each other means manually reconciling incompatible CC maps. Control Room addresses both by owning the routing layer itself.

### Long-term vision (designed-for, NOT built in v1)

- **NKS without the NI client** — read NKS metadata to auto-generate rich device models
- **AI natural-language patching** — "wire the Novation up to the Moog"
- **VCV Rack bridge** — software modular and hardware in one graph
- **Ableton two-way integration** — see and change device parameters from the keyboard
- **CV / audio-rate signals** via DC-coupled interfaces

These are explicitly out of v1 scope. The architecture exists to make each one an *addition* rather than a rewrite. See §9.

---

## 2. Scope

### In v1

| Capability | Notes |
|---|---|
| Graph engine with continuous modulation | Engine-generated LFOs, envelopes, followers rendered to MIDI/OSC |
| MIDI in/out | Including 14-bit CC and NRPN |
| OSC in/out | First-class signal source and sink |
| Device packs | Hand-authored YAML semantic models; Novation Peak first, Moog next |
| Browser node editor | Drag-and-drop patching, live meters |
| S88 screen modules + knob takeover | Keyboard as a surface onto the engine |
| Pi/appliance deployment | Cross-compiled, systemd, mDNS |

### Not in v1

NKS parsing, AI patching, VCV Rack bridge, Ableton integration, plugin hosting, real CV I/O.

### Success criteria

1. A software LFO modulates the Novation Peak's VCA level, audibly and smoothly, with no NI software running.
2. The patch is built by dragging nodes in a browser, not editing config by hand.
3. The S88 screen shows the patch as a live module; grabbing a knob takes over the modulated parameter without a value jump.
4. The same binary runs headless on a Raspberry Pi, reachable at `controlroom.local`.

---

## 3. Key design decisions

### 3.1 Signal model: engine-generated continuous modulation

An analog LFO inside a Moog is a voltage that never leaves the instrument as MIDI. Therefore "modulate the Peak's VCA with an LFO" is implemented as an **engine-generated** control-rate signal rendered to MIDI/OSC — not as a captured analog signal.

This works with all existing gear and needs no CV hardware. Real CV and VCV Rack become additional *source types* later without changing the graph model.

### 3.2 One command API for every client

**The browser UI, the S88 surface, and the future AI layer all speak the same graph-command API.** The AI gets no privileged path: "wire the Novation to the Moog" compiles into exactly the commands a mouse would issue.

This is the decision that keeps the system modular. It means the AI layer is small when built, and that any future surface is a client rather than a special case.

### 3.3 Semantic device packs, hand-authored first

Device packs are editable YAML mapping semantic parameter names to wire encodings. Hand-authored for the first devices; NKS metadata later *generates* packs rather than replacing the format. This avoids blocking v1 on the hardest reverse-engineering problem, and keeps devices community-extensible.

### 3.4 Headless service, browser as client

A single Rust binary exposing HTTP + WebSocket. Rust for sub-millisecond timing on constrained hardware, single-binary deployment, and aarch64 cross-compilation. The browser UI is bundled into the binary as static assets — an appliance on a closed studio network cannot fetch from a CDN.

---

## 4. Architecture

Five layers, each replaceable:

1. **Adapters (the wire)** — MIDI, OSC, and S88 drivers translate bytes ↔ engine events. Nothing above this layer knows what a CC number is. VCV, CV, and NKS become new adapters.
2. **Device packs (the meaning)** — named parameters with ranges, units, and wire encodings.
3. **Graph engine (the brain)** — sources, transforms, sinks on a fixed control-rate tick.
4. **Control plane** — HTTP/WebSocket; commands in, state and meters out.
5. **Surfaces** — browser and S88, both clients of the same API.

### 4.1 Crates

```
cr-core      types, graph model, command enum — no I/O
cr-engine    tick loop, node implementations — pure, depends only on cr-core
cr-packs     device pack schema, loader, validation
cr-midi      midir adapter; MIDI encode/decode incl. 14-bit CC, NRPN; rate limiting
cr-osc       rosc adapter; address-pattern ↔ signal binding
cr-s88       Kontrol MK3 driver: MIDI templates, DAW-port SysEx, LED control
cr-server    axum HTTP/WS server; serves embedded UI assets
cr-ui        cargo crate embedding the built frontend bundle (rust-embed)
ui/          npm workspace: browser node editor (React Flow); built into cr-ui
controlroom  binary: config, CLI, wiring
```

Strict dependency DAG; every crate independently testable.

### 4.2 Node taxonomy

- **Sources** — LFO, envelope, envelope follower, MIDI input, OSC input, physical knob
- **Transforms** — scale, curve, math, quantize, logic, takeover
- **Sinks** — device parameter, MIDI out, OSC out

### 4.3 Threading and timing

- **Control tick: 1 kHz**, dedicated thread. No allocation in the tick; node state is pre-allocated when a patch is applied.
- **Commands** arrive via channel and **apply at tick boundaries**, so the engine never observes a half-built graph.
- **State out** is coalesced to ~30 Hz for meters and screen — the UI does not need 1 kHz.

---

## 5. Data flow — the flagship patch

Target: `LFO(0.3 Hz, triangle) → Scale(0..1 → 20..90) → Sink(peak.vca.level)`

1. Browser drags an LFO node; sends `AddNode`.
2. Command is validated against the pack registry, queued, applied at the next tick boundary.
3. Each tick: LFO computes phase → Scale maps the range → sink asks the Peak pack for the encoding (CC 24, ch 1, 7-bit) and quantizes.
4. Sink emits **only if the quantized value changed** and the rate budget allows; pushes to an output ring buffer.
5. `cr-midi` drains the ring and writes to the port.
6. State publisher coalesces values at 30 Hz → WebSocket → browser meters, and → `cr-s88` → screen module `LFO→VCA 47`.
7. User grabs S88 knob 3 → knob delta → Takeover node hands control to the knob.

**Takeover uses pickup mode in v1:** the modulated parameter keeps following the LFO until the physical knob's position crosses the current modulated value, at which point the knob takes over. This guarantees no value jump. Other takeover modes (crossfade, scaled-relative) are deferred — a node parameter, not a redesign.

### 5.1 MIDI bandwidth is a first-class constraint

**DIN MIDI carries roughly 320 messages per second, total** (31250 baud, 3 bytes per CC). A few modulation sources at full control rate will saturate the bus and smear timing.

Therefore every sink has **per-parameter rate limiting and deduplication**, last-write-wins. When the limiter drops intermediate values, **the UI displays the drop rate**. Silent throttling would be this application's most confusing possible failure mode and is prohibited by design.

---

## 6. Error handling

**Governing invariant: invalid graphs are rejected at the command layer, before the engine sees them.** Type mismatches, cycles, and unknown parameters never reach the tick thread. This is what allows every node to be a total, panic-free function.

Everything else degrades rather than fails:

| Condition | Behavior |
|---|---|
| Device disconnected | Sinks hold last value, node marked `orphaned`, UI shows amber |
| Device replugged | Auto-rebind via stable port identity (name + USB location), resume |
| Malformed device pack | Load-time validation failure with line number; rest of graph runs |
| MIDI bus saturation | Bounded rate limiter, last-write-wins, drop rate surfaced in UI |
| Invalid patch edit | Rejected at command validation with a reason; graph unchanged |

---

## 7. Testing

**The engine is a pure function of (graph, inputs, tick number).** The tick count is an *input*, never a clock read — so engine tests are exact, fast, and free of timing flake.

- `cr-engine` — golden-file tests asserting precise output values over N ticks
- `cr-packs` — schema validation; every shipped pack round-trips
- `cr-midi` — byte-exact encode tests including 14-bit CC and NRPN edge cases; decode against recorded streams
- `cr-s88` — byte-exact replay against captured device traffic
- Integration — start the server, drive commands over WebSocket, assert bytes on a virtual MIDI port

### 7.1 Verification honesty requirement

**The Kontrol MK3 screen and LED protocol details are reverse-engineered and partly unverified.** Known landmines from community research:

- The MK3 uses USB **bulk** transfer, not HID, for LED control
- LED updates are `0x82` packets with 128 note-indexed entries
- Entering legacy LED mode (`A0 00 00`) **disables MIDI** until `A0 01 00` is sent
- Driving LEDs via NI's background hardware service (msgpack-RPC over a Unix socket) preserves MIDI; claiming the bulk interface directly does not
- Arbitrary pixel drawing on the MK3 display is **not publicly reverse-engineered**; v1 uses MIDI Templates and DAW-port SysEx for structured UI only

Anything in `cr-s88` derived from these sources is marked `verify against hardware` in code and docs. **Hardware verification is an explicit manual gate.** A green test suite must never be reported as confirming device behavior.

---

## 8. Milestones

| # | Deliverable | Proves |
|---|---|---|
| M1 | Engine + MIDI + Peak pack; LFO→VCA from a config file, headless | Continuous modulation into real hardware feels good |
| M2 | Server + browser node editor | The patch is buildable by dragging |
| M3 | OSC adapter | Signals cross the network boundary |
| M4 | S88 screen modules + knob takeover | The keyboard is a surface onto the engine |
| M5 | Pi cross-compile, systemd, mDNS | It is an appliance, not a laptop app |

**M1 is the honest proof.** If continuous modulation into real hardware does not feel good there, the rest is not worth building yet.

**Planning scope:** this document specifies the whole system, but implementation planning starts with **M1 only**. Later milestones get their own plans once M1's hardware reality is known.

---

## 9. Extension points

Each deferred feature attaches at exactly one seam:

| Future feature | Attaches as |
|---|---|
| NKS without NI client | Pack *generator* feeding `cr-packs`; new adapter for plugin hosting |
| AI patching | API client emitting the same graph commands as the UI |
| VCV Rack | New adapter + source/sink types |
| Ableton two-way | New adapter (Remote Script) + surface bindings |
| Real CV | New adapter; continuous signals already modeled |

No deferred feature requires changing the graph model, the command API, or the engine.
