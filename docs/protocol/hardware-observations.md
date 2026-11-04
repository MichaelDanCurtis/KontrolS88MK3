# Kontrol S88 MK3 — FIRST HARDWARE-CONFIRMED OBSERVATIONS

**Date:** 2026-10-20
**Device present:** USB Product Name `KONTROL S88 MK3`, `idVendor` 6092 (`0x17CC`),
`idProduct` 8480 (`0x2120`)
**Serial (from feature report 0xD9):** `KS8FU6BEF4A29BU4AC584U6U3`
**Evidence class:** direct hardware observation (read-only)
**Tooling:** `mk3probe.c`, `mk3probe2.c`, `midiname.c` — all IOKit/CoreMIDI, reads only.
No `SetReport` was ever issued; the bulk display pipe was never touched.

This is the first entry in this project with `hardware_capture: true` standing.

---

## 1. Predictions from static analysis that the hardware CONFIRMED

| Prediction (source) | Hardware observation | Verdict |
|---|---|---|
| S88 MK3 PID = `0x2120` (NIHardwareAgent device DB) | `idProduct` 8480 = `0x2120` | **CONFIRMED** |
| VID = `0x17CC` | `idVendor` 6092 = `0x17CC` | **CONFIRMED** |
| DAW port named exactly `KONTROL S88 MK3 DAW` (NI's Cubase script) | device name `KONTROL S88 MK3` + entity `DAW` concatenates to exactly that | **CONFIRMED** (with a caveat, §3) |
| Report map `O-A1 BC8 W1 B1` is the largest output report → 203 + 1 ID = 204 | `MaxOutputReportSize` = **204** | **CONFIRMED** |
| Report map `F-D8 D2 W4 B10` is the largest feature report → 32 + 1 ID = 33 | `MaxFeatureReportSize` = **33** | **CONFIRMED** |

The last two matter beyond themselves: they independently validate that the
report-map DSL recovered from the binary (`I-`/`O-`/`F-` report ids with
`B`ytes/`W`ords/`D`words counts in hex) was **parsed correctly**. That raises
confidence in every other figure derived from that DSL — including the light-guide
finding that output report `0x82` is `0x58` = 88 bytes, one byte per key.

Additional observations:
- HID `PrimaryUsagePage` = 65281 = **`0xFF01`** (vendor-defined), `PrimaryUsage` = 0.
- `MaxInputReportSize` = 63.
- Three MIDI port pairs: **Main**, **DAW**, **Ext**.

---

## 2. NEW hardware fact: feature report `0xF8` IS NOT IMPLEMENTED BY THIS DEVICE

> **Correction.** An earlier version of this document concluded `0xF8` was
> "SET-only". That was wrong. The device's own HID report descriptor was
> subsequently dumped (§2a) and does not declare `0xF8` in any direction. The
> read failures below are explained by the report not existing, not by the
> direction of access.

The static analysis lead was that display geometry might be *readable* from the
device via `GET_REPORT` on feature report `0xF8` (declared `F-F8 W2 B6` = 10 bytes).

**It is not.** Tested read-only with a controlled experiment:

| Report ID | Declared? | GetReport result |
|---|---|---|
| `0xD0` (len 30) | yes | **OK** — 30 bytes, all zero |
| `0xD0` (len 33) | yes | **OK** |
| `0xD8` (len 33) | yes | **OK** — `D8 05 01 00 13 69 71 FE 99 5D 6A CF 65 00 00 00 00 FF…FF 73 74 33 32` |
| `0xD9` (len 33) | yes | **OK** — ASCII serial number |
| `0xF8` (len 10) | yes | **FAIL** `0xE0005000` |
| `0xF8` (len 11) | yes | **FAIL** `0xE0005000` |
| `0xF8` (len 33) | yes | **FAIL** `0xE0005000` |
| `0x55` (**negative control** — not in report map) | no | **FAIL** `0xE0005000` |

`IOHIDDeviceOpen` returned success, and three other declared feature reports read
fine — so `GetReport` is **not** blocked for this process, and the failure is not a
buffer-length problem (three lengths tried). `0xF8` fails with **the identical error
code as an undeclared report ID**, which is the signature of an unsupported
operation.

Seeding the buffer exactly as NI's `GenericHID::initialize` does — `buf[0]` = report
id, `buf[8] = 0x50`, `buf[9] = 0x32` — also failed, at lengths 10, 11 and 33, and
for ids `0xF9`/`0xFA` (display indices 1 and 2). Seven variants, all
`0xE0005000`.

### 2a. The decisive test: the device's own report descriptor

`kIOHIDReportDescriptorKey` (read-only property, 1001 bytes) was parsed for every
main item and its governing Report ID:

| Type | Report IDs the device actually declares |
|---|---|
| INPUT | `0x01`, `0x02`, `0x03`, `0x40`, `0xAA` |
| OUTPUT | `0x80`, `0x81`, `0x82`, `0xA0`, `0xA1`, `0xA2`, `0xA3`, `0xA4`, `0xAF`, `0xF3`, `0xF4` |
| FEATURE | `0xD0`, `0xD8`, `0xD9` |

**`0xF8` appears zero times.**

`[OBS]` **Conclusion: the S88 MK3 firmware does not implement report `0xF8` in any
direction.** The read failed because the report does not exist. The geometry is not
obtainable from the keyboard by this mechanism.

### 2b. This reconciles with the static analysis

Static analysis of `NIHostIntegrationAgent` established that `0xF8+i` *is* a GET —
`GenericHID::initialize` calls `getFeatureReport` and then reads width/height/format
out of it. Both facts are true, because **that code path never runs for an MK3
keyboard**: `HIDDisplayImplementation`'s only callers are `KontrolMK1`, `KontrolA`,
`KontrolM` and `MaschineMikroMK3`, and `BulkDisplayImplementation`'s are
`KontrolMK2`, `MaschineMK3` and `MaschinePlus`. No MK3 keyboard uses either.

So `0xF8` is the geometry mechanism **for other NI devices**, not for this one.

### 2c. NI's internal report-map table is NOT authoritative for the real device

Comparing NI's `kksm3-88` table string against the descriptor the hardware actually
presents:

- **Agrees** on `I-1`, `I-2`, `I-3`, `I-AA`; `O-80`, `O-81`, `O-82`, `O-A0`, `O-A1`,
  `O-A2`, `O-A3`, `O-A4`, `O-AF`, `O-F3`, `O-F4`; `F-D0`, `F-D8`, `F-D9`. That is a
  near-complete match and further validates the DSL parse.
- **NI's table lists `F-F8`, which the device does not implement.**
- **The device declares `INPUT 0x40`, which NI's table omits.**

`[INF]` NI's table is a generic or multi-firmware descriptor, not a faithful copy of
this unit's descriptor. **Treat it as a hint, and confirm every report against the
live descriptor before relying on it.**

`[OBS]` Separately, **`OUTPUT 0x82` is confirmed present on real hardware**, which
supports the light-guide finding (one byte per key) on something stronger than a
table string.

### Incidental finds worth keeping

- **`0xD9` = device serial**, 25 ASCII chars. Useful for distinguishing two
  identical keyboards.
- **`0xD8` = an identity/version block.** Starts `05 01 00 13`, has a 64-bit-looking
  field, `FF` padding, and ends with ASCII `st32`. **Not decoded — do not guess.**
- **`0xD0` reads as all zeros** (30 bytes). Possibly uninitialised, or a
  write-mostly config block.

---

## 3. Port-naming trap for the client implementation

CoreMIDI exposes the name at two levels:

- `MIDIDevice` `kMIDIPropertyName` = `KONTROL S88 MK3`
- `MIDIEntity`/endpoint `kMIDIPropertyName` = `Main` / `DAW` / `Ext`

So the string NI's script matches, `KONTROL S88 MK3 DAW`, exists only as the
**concatenation**. Code that enumerates endpoints and reads their own
`kMIDIPropertyName` sees the bare word `DAW`, which is ambiguous.

**Second trap: CoreMIDI retains offline devices.** `MIDIGetNumberOfDevices()`
returned 15 and listed **two** entries named `KONTROL S88 MK3` (indices 2 and 7),
while only four endpoints were actually online. One is a stale entry from a prior
connection.

`[INF]` Recommendation for Control Room: enumerate **online endpoints**
(`MIDIGetSource`/`MIDIGetDestination`), not devices; and match on the device-plus-
endpoint name, scoped to manufacturer `Native Instruments`, rather than on a bare
substring like `DAW`.

---

## 4. M1 hardware gate: STILL BLOCKED — no Novation Peak

Full MIDI device inventory observed:

`IAC Driver`, `Bluetooth`, **`KONTROL S88 MK3` ×2**, `mioXL`, `mioXL Loader`,
`LCXL3 1` (Focusrite–Novation), `RODECaster Duo` ×2, `Ableton Push 2`, `Network`,
`UMP Network`, `Osmose` (Expressive E), `Komplete Audio 6 MK2`, **`Moog One`**.

The only Novation device is an **LCXL3** (Launch Control XL 3), a control surface —
**not** a Peak synthesiser. So M1's audible acceptance gate remains open for the
same reason the ROADMAP recorded in August: no Peak port on this machine.

`docs/verification/` must stay absent until a Peak is actually driven.

Note for later: a **Moog One** and an **Osmose** are present and could be useful
semantic-pack targets, and the Moog One relates to separate existing notes.

---

## 5. What is still unknown

1. **Display resolution and `DataFormat` for the MK3** — not obtainable from the
   device (§2), absent from `NIHardwareAgent`. Analysis of
   `NIHostIntegrationAgent` is in progress.
2Decoding of feature report `0xD8` (identity/version block).
3. Why `0xD0` is all zeros.
4. Whether the second, stale `KONTROL S88 MK3` CoreMIDI device entry can cause a
   mismatch in practice.

## 6. Reproduction

Programs in the session scratchpad, all read-only:
- `mk3probe.c` — single feature-report read with layout self-check
- `mk3probe2.c` — multi-report-ID diagnostic with negative control, plus CoreMIDI listing
- `midiname.c` — device/entity/endpoint name resolution
- `wait_mk3.sh` — bounded wait-for-enumeration then probe

Build: `cc -o X X.c -framework IOKit -framework CoreFoundation -framework CoreMIDI`
