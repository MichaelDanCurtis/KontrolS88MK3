# Kontrol S88 MK3 protocol notebook

This notebook is the evidence boundary for S88 protocol work. A byte sequence is supported only
after a physical S88 MK3 capture reproduces it. Synthetic fixtures are useful for deterministic
software tests, but they are never evidence that a proprietary display or DAW-port command is safe.

## Safety rules

- Capture input before transmitting anything. The `cr-s88-capture record` command opens a MIDI
  input only and never creates an output connection.
- Never replay an undocumented SysEx or USB payload merely because it resembles an MK2 message.
- Keep the Native Instruments process, keyboard firmware version, host OS, port name, and exact UI
  action in the fixture metadata or adjacent experiment notes.
- Change one visible value at a time. Keep a baseline capture with no interaction for comparison.
- Preserve captures as immutable evidence. Make annotations in a copied fixture.
- A fixture with `hardware_capture: false` must retain “verify against hardware” in its source.

## Capture fixture format

Fixtures use schema version 1 and live under `crates/cr-s88/fixtures/` once scrubbed of irrelevant
traffic. Each event contains a monotonic microsecond timestamp, direction, complete message bytes,
and an optional note. MIDI fixtures require exactly one valid MIDI message per event; USB bulk
fixtures preserve one observed transfer per event and may contain arbitrary bytes.

```yaml
schema_version: 1
device: Native Instruments Kontrol S88 MK3
firmware: 2.x.y
protocol: midi # or usb_bulk
hardware_capture: true
source: macOS MIDI port name and experiment identifier
events:
- at_us: 1204
  direction: input
  bytes: [176, 20, 64]
  note: Standard-template knob 1 moved to midpoint
```

The checked-in `standard-template-cc.synthetic.yaml` is deliberately synthetic. It tests the
fixture tooling and replay path; it does not close the hardware acceptance gate.

## Tool workflow

Build and inspect the tool:

```sh
cargo run -p cr-s88 --bin cr-s88-capture -- summarize \
  crates/cr-s88/fixtures/standard-template-cc.synthetic.yaml
cargo run -p cr-s88 --bin cr-s88-capture -- list
cargo run -p cr-s88 --bin cr-s88-capture -- validate FILE.yaml
cargo run -p cr-s88 --bin cr-s88-capture -- diff BASELINE.yaml ACTION.yaml
cargo run -p cr-s88 --bin cr-s88-capture -- analyze-usb USB_CAPTURE.yaml
```

Capture a verified MIDI input trace without transmitting:

```sh
cargo run -p cr-s88 --bin cr-s88-capture -- record \
  --port "KONTROL S88 MK3" \
  --output captures/knob-1-midpoint.yaml \
  --seconds 10 \
  --firmware 2.x.y
```

The diff command exits 0 for identical captures and 1 when differences are present, so it can be
used as a research assertion in scripts. It reports event indices and changed byte offsets.

For the complete standard-template evidence pass, prefer the guided wrapper:

```sh
scripts/s88-acceptance.sh \
  --port "KONTROL S88 MK3" \
  --firmware 2.x.y \
  --output captures/s88-standard-template.yaml
```

It records input only and then runs `verify-template`. Verification refuses synthetic fixtures and
requires every configured knob CC to contain values at minimum (`0…2`), midpoint (`60…68`), and
maximum (`125…127`) on the selected channel. This proves capture completeness, not pickup feel,
bank behavior, reconnect behavior, or any display protocol.

`analyze-usb` is read-only. It reports direction counts, transfer-length histograms, how often a
four-byte little-endian length header matches the remaining payload, and the common payload prefix
after that header. It never interprets a candidate command and cannot transmit captured bytes.

## Capturing USB on macOS

The display/DAW-port protocol lives on the USB bulk endpoints, not MIDI, so it has to be captured
with Wireshark/tshark against the USB host controller interface (the `XHC`/`XHCn` pseudo-interface
that macOS exposes for `usbmon`-style captures), not a network interface. This requires elevated
privileges: either run `tshark`/`Wireshark` with `sudo`, or grant the `ChmodBPF` helper (installed
with Wireshark) access so your user can read `/dev/bpf*`.

**`tshark` is not currently installed on this development machine.** The `import-usb` field mapping
below (see "Field mapping is unverified") was written from general knowledge of Wireshark's USB
dissector, not from a real export, and must be checked against an actual capture before it is
trusted.

General shape of a capture session:

```sh
# List candidate interfaces; look for the XHC/XHCn entry matching the bus the S88 is on.
tshark -D

# Capture bulk traffic on that interface while driving the NI client, then export as JSON.
sudo tshark -i XHC20 -w s88-display-experiment.pcapng
tshark -r s88-display-experiment.pcapng -T json > s88-display-experiment.json
```

Keep the same one-visible-value-at-a-time discipline as MIDI captures: baseline first, one change,
stop, export, and note the exact UI action next to the JSON export before importing it.

## The `import-usb` command

`cr-s88-capture import-usb` turns a `tshark -T json` export into a `usb_bulk` `CaptureFixture`. It
deliberately consumes tshark's JSON rather than parsing pcapng or the `DLT_USB_DARWIN` binary layout
itself — tshark does the dissection, so this tool is not guessing at binary structures nobody here
can test against hardware.

```sh
cargo run -p cr-s88 --bin cr-s88-capture -- import-usb s88-display-experiment.json \
  --output crates/cr-s88/fixtures/display-experiment.usb.yaml \
  --device "Native Instruments Kontrol S88 MK3" \
  --firmware 2.x.y \
  --source "tshark capture on XHC20, label-length experiment" \
  --hardware-capture
```

Flags:

- `INPUT.json` (positional, required) — a `tshark -T json` export.
- `--output FILE` (required) — where to write the resulting YAML fixture.
- `--device NAME` (required) — recorded verbatim in the fixture; never inferred from the capture.
- `--firmware VERSION` (optional) — recorded verbatim if supplied.
- `--source TEXT` (optional) — defaults to a string naming the input file if omitted.
- `--hardware-capture` (optional, off by default) — sets `hardware_capture: true`. This is never
  inferred from the file; you assert it because you know the bytes came from the physical S88.
  Without it, the fixture keeps `hardware_capture: false` and `import-usb` does not rewrite
  `--source` to hide that — per the rule above, a non-hardware fixture's source should still say
  "verify against hardware" if it is meant to stand in for a pending capture.
- `--all-transfer-types` (optional, off by default) — by default only USB bulk transfers are kept;
  control, interrupt, and isochronous transfers are dropped. Pass this to keep everything.

Direction is mapped so that host→device (OUT) transfers become `output` events and device→host (IN)
transfers become `input` events. For framebuffer research the host→device transfers are the
interesting ones, so double-check this mapping against `summarize`/`analyze-usb` output on the first
real capture — an inverted mapping would send someone analysing the wrong half of the trace.

### Field mapping is unverified

No S88 is connected to this machine, so the tshark JSON field names `import-usb` looks for
(`frame.time_relative` for timestamps; `usb.endpoint_address.direction` and `usb.src`/`usb.dst` for
direction; `usb.capdata`, `usb.payload`, and `usb.data_fragment` for payload bytes) have not been
checked against a real export. The importer searches the whole JSON tree for each candidate name in
priority order rather than assuming one fixed nesting, and if none of the candidates for a value are
present it fails immediately with an error listing every name it tried and every field name it found
in the first packet, instead of silently emitting an empty or wrong fixture.

Treat the first real capture as a calibration run, not evidence: import it, then read the
`summarize` and `analyze-usb` output by eye — event counts, direction split, transfer-length
histogram, byte values — before drawing any conclusion from it. If the shape is wrong, `import-usb`
should have already refused to run rather than produced a plausible-looking but incorrect fixture;
if it did produce output, sanity-check that the reported direction split and byte values match what
you expect from the experiment you ran.

## Current evidence

| Area | Status | Evidence needed |
| --- | --- | --- |
| Standard-template absolute CC input | Implemented, hardware acceptance pending | Capture all eight knobs at min/mid/max and confirm channel/CC assignments |
| Device discovery and reconnect | Implemented, hardware acceptance pending | Unplug/replug trace plus runtime reconnect stats |
| Surface banks and pickup takeover | Deterministic software tests pass | Eight-knob manual no-jump acceptance sequence |
| DAW-port labels and values | Unverified; disabled | Paired baseline/one-label/one-value captures from the NI client |
| Pixel framebuffer | Unverified; post-v1 | USB transfer captures with controlled solid colors and single-pixel/rectangle changes |

No display byte sequence is currently approved for transmission. `UnverifiedDisplayTransport`
must continue returning `UnverifiedProtocol` until reviewed physical captures are checked in.

## Display research experiment matrix

Use separate captures for each row and reset the keyboard/host state between experiments.

| Experiment | Baseline | Single change | Expected analytical value |
| --- | --- | --- | --- |
| Idle | NI client connected, no input | none | Periodic/control traffic to exclude |
| Label length | label `A` | label `B`, then `AA` | Character encoding, length, and framing fields |
| Numeric value | value `0` | value `1`, then `127` | ASCII vs binary value encoding |
| Solid color | black page | white/red/green/blue page | Payload size, channel order, compression |
| Rectangle | empty page | one aligned rectangle | Stride, tile size, and origin |
| Bank navigation | bank 1 | bank 2 only | Page identifiers and activation handshake |

For every candidate field, record: byte offsets, endianness, values observed, competing
interpretations, confidence, and the experiment that could disprove it. Do not promote a hypothesis
to implementation until it predicts a capture that was not used to derive it.

## Hardware acceptance sequence

1. Confirm the exact firmware and selected standard MIDI template.
2. Record all eight knob CCs at 0, approximately 64, and 127.
3. Start Control Room and confirm discovery plus connected status.
4. Drive eight distinct graph bindings and verify the expected parameters move.
5. Switch banks/pages and verify deterministic ordering.
6. While an engine modulation is active, touch each assigned knob and verify pickup produces no
   output jump.
7. Unplug/replug the keyboard and verify reconnection without restarting Control Room.
8. Save the captures, effective configuration, runtime statistics, and observations together.

Only after all eight steps pass may the M4 hardware completion gate be marked complete.
