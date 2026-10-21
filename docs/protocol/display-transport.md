# MK3 display transport + pixel format — recovered from NIHardwareAgent

**Date:** 2026-10-20
**Target:** `NIHardwareAgent.arm64`, Mach-O arm64, sha256 `48e2e563…`
**Provider:** Ghidra 12.1.4 via REA, plus `objdump --macho`/`nm` on the same file.
`__TEXT` vmaddr `0x100000000`, fileoff 0 → **VA == file offset**, so all addresses are reproducible.

`[OBS]` = literal code/bytes. `[INF]` = conclusion. `[UNK]` = undetermined.
Binary strings/symbols treated as data. Nothing in them resembled a directive.

---

## 1. Pixel format — SOLVED `[OBS]`

`NI::NHL2::Display::renderPixel(uint32 src, DataFormat, uint8_t** cursor)` @ `0x10019216c`.

Decisive first two instructions:

```
10019216c:  and  w8, w1, #0xffffffbf      ; fmt & ~0x40  — bit 6 masked OFF
100192170:  sub  w8, w8, #0x10            ; index = (fmt & ~0x40) - 0x10, valid 0..0x11
```

**Bit 6 does not affect colour at all.** It is purely the compressed flag. So `0x50 ≡ 0x10`,
`0x60 ≡ 0x20`, `0x61 ≡ 0x21`. This confirms the bitfield hypothesis derived independently from
`renderPixelData`.

Source pixel is `0xAARRGGBB`: `R = src>>16`, `G = src>>8`, `B = src & 0xff`, `A = src>>24`.

### The two compressed 16-bit formats

```
case 0x20 / 0x60:  byte0 = (R & 0xF8) | (G >> 5)            ; 0x100192580
                   byte1 = ((G << 3) & 0xE0) | (B >> 3)
case 0x21 / 0x61:  byte0 = (B & 0xF8) | (G >> 5)            ; 0x1001925d8
                   byte1 = ((G << 3) & 0xE0) | (R >> 3)
```

- Masks `0xF8`/`0xE0` with shifts 3 and 5 → **5-6-5**, not 555.
- **`0x60` = RGB565. `0x61` = BGR565.** The only difference is the R↔B swap, carried by bit 0.
- **Big-endian within the 16-bit word.** `byte0` is the high byte, `byte1` the low byte.
- `renderPixel` writes bytes straight to the cursor with **no `rev`**. The only `rev w8, w8` in the
  path (`0x100191820`) is on *command* words. Pixel bytes land on the wire exactly as written.

### Pair ordering — reconciled

`renderPixelData` establishes "two pixels per 32-bit word, low half-word = lower-x pixel".
`renderPixel` writes bytes sequentially through a shared cursor (two calls at `0x100191848` /
`0x100191868`). On little-endian ARM the low half-word occupies the first two bytes in memory, and
pixel payload is not byte-reversed. `[INF]` Therefore **the first `renderPixel` call is the lower-x
pixel, and occupies the first two bytes on the wire**: `pixLow.b0 pixLow.b1 pixHigh.b0 pixHigh.b1`.

### Full format families `[OBS]`

32 bpp (4 bytes out):

| Value | Wire bytes |
|---|---|
| `0x10` | `FF, R, G, B` |
| `0x11` | `R, G, B, FF` |
| `0x12` | `A, R, G, B` |
| `0x13` | `R, G, B, A` |
| `0x14` | `FF, B, G, R` |
| `0x15` | `B, G, R, FF` |
| `0x16` | `A, B, G, R` |
| `0x17` | `B, G, R, A` |

Sub-bits: bit0 = alpha last, bit1 = real alpha vs constant `0xFF`, bit2 = BGR.

16 bpp (`0x18`–`0x21`, all big-endian): `0x18` = XRGB1555 (X=0), `0x1A` = ARGB1555,
`0x19` = R5G5B6, `0x20` = RGB565, `0x21` = BGR565.
**`0x1B`–`0x1F` were read but not individually verified — do NOT transmit those without re-checking.**

### `getDataFormatProperties` @ `0x100192964` `[OBS]`

Returns `bytesPerUnit` / `pixelsPerUnit`:

- `0x00`/`0x01` → 1 byte / 8 px (**1 bpp mono**)
- `0x10`–`0x17`, `0x50` → 4 bytes/px
- `0x18`–`0x21`, `0x60`, `0x61` → 2 bytes/px
- everything else returns **false**

So among bit-6-set values only `0x50`, `0x60`, `0x61` are supported — the only *compressed* formats
actually usable, even though `renderPixel` would handle `0x51`–`0x5F`.

`[INF, strong]` That function's 4th out-param (`bool&`) is true for exactly
`0x12,0x13,0x16,0x17,0x1A,0x1B,0x1E,0x1F` — precisely the real-alpha formats. It is an **alpha flag,
not a byte-swap flag**.

---

## 2. The wire format of a display draw — CONFIRMED `[OBS]`

Sole caller of `renderPixelData` is `NI::NHL2::Display::GenericHID::drawBulk` @ `0x100193a90`
(call at `0x100193c08`). It writes a 16-byte header before and a 4-byte footer after:

```
100193b90:  mov  w11, #0x84
100193b94:  strh w11, [x9]        ; +0x00  u16 LE 0x0084  → wire bytes 84 00
100193b98:  strb w8,  [x9, #0x2]  ; +0x02  display index
100193b9c:  strb w10, [x9, #0x3]  ; +0x03  DataFormat
100193ba0:  str  wzr, [x9, #0x4]  ; +0x04..07  ZEROED
            ; +0x08 x, +0x0A y, +0x0C w, +0x0E h — each u16 BIG-ENDIAN
100193bd4:  add  x8, x9, #0x10    ; payload cursor = +0x10
```

Footer, after `renderPixelData` returns:

```
100193c18:  mov  w10, #0x40
100193c1c:  strh w10, [x8]        ; 40 00
100193c20:  strb w9,  [x8, #0x2]  ; display index
100193c24:  strb wzr, [x8, #0x3]  ; 00
```

Then **one single transfer** of the whole buffer:
`writeToPipe(2, base=[this+0x38], len = cursor - base)` at `0x100193c30`–`0x100193c48`.
**No chunking anywhere.**

### Complete frame layout

```
84 00 | idx | fmt | 00 00 00 00 | x:BE16 | y:BE16 | w:BE16 | h:BE16
      | <renderPixelData command stream>
      | 40 00 idx 00
```

### CORRECTION to public research `[OBS]`

**There is no 4-byte little-endian payload-length header.** The community claim is refuted: the
transfer starts exactly at `84 00`, and bytes `+0x04..+0x07` are explicitly zeroed by `str wzr`.
There is no length field anywhere in the frame.

Output budget (`0x100193bdc`–`0x100193bf8`) is `width * 4 * height` bytes, read from the
display-config block — **not** from the dirty rect.

Thin wrappers that also hard-code pipe 2 (`mov w1,#0x2`): `sendBulkDisplayRequest` @ `0x1001932d4`,
`GenericHID::bulkRequest` @ `0x100193344`. `onDrawDisplayMessage` @ `0x100193368` selects the
display with `msg->[0x10] & 7`, payload at `msg+0x20`, length `msg->[0x1c]`.

---

## 3. USB transport `[OBS]`

From the device-DB static initializer `__GLOBAL__sub_I_unity_3_cxx.cxx`.
`USBDeviceDB::Entry` is 0x68 bytes: `+0x00 u32 eUSBDeviceType`, `+0x04 u16 VID`, `+0x06 u16 PID`,
`+0x08` SSO string (GUID), `+0x30` SSO string (IOKit class name). Layout independently confirmed by
`getDeviceFilter` @ `0x10018744c`.

**VID `0x17CC` for all.**

| Device | PID | Base entry | Bulk-display entry |
|---|---|---|---|
| Komplete Kontrol S49 MK3 | `0x2100` | `0x100985d30` type 2 | `0x100985d98` type **4** |
| Komplete Kontrol S61 MK3 | `0x2110` | `0x100985e68` type 2 | `0x100985ed0` type **4** |
| **Komplete Kontrol S88 MK3** | **`0x2120`** | `0x100985fa0` type 2 | `0x100986008` type **4** |
| Traktor Kontrol S4 MK3 | `0x1720` | `0x1009874c0` type 2 | `0x100987528` type **3** |
| Maschine+ | `0x1820` | `0x100986b00` type 2 | `0x100986b68` type **3** |

`[OBS]` **`eUSBDeviceType 3` = bulk display**, proven twice: `initializeWithBulkInterface` calls
`getDeviceFilter(vid, pid, 3)` (`0x100192b48: mov w2,#0x3`), and both known-good bulk-display
entries are type 3 taking their GUID from `USBDeviceDB::NIBulkDisplayGUID`.

`[OBS]` `NIBulkDisplayGUID` @ `0x1009851a8` is the 15-char string **`"nibd-guid-magic"`** — a
placeholder, not a real GUID. Siblings: `"usb-guid-magic"`, `"hid-guid-magic"`,
`"nidfu-guid-magic"`, `"niiap-guid-magic"`.

`[OBS]` **The three MK3 keyboards' bulk-display entries are type `4`, not `3`, and carry a real
Windows GUID `{9CB5BE68-52CA-4F04-A394-090391A15E51}`.** **No code in this binary ever requests
type 4** — the single `getDeviceFilter` callsite passes 3.

### Interface and endpoint `[OBS]`

`connectToBulkDevice` @ `0x1001936cc` builds an `NIUCDevice` and calls vtable `+0x28`:

```
1001938e4:  mov  w1, #0xbd
1001938e8:  mov  w2, #0x2
1001938ec:  blr  x8            ; openVendorInterface(0xBD, 2)
```

→ `OSImpl::openVendorSpecificPipes` @ `0x1001a09fc` fills an `IOUSBFindInterfaceRequest`:

```
1001a0a34:  mov  w8, #0xff
1001a0a38:  strh w8, [sp, #0x38]   ; bInterfaceClass    = 0xFF (vendor specific)
1001a0a3c:  strh w1, [sp, #0x3a]   ; bInterfaceSubClass = 0xBD
1001a0a40:  mov  w8, #-0x1
1001a0a44:  str  w8, [sp, #0x3c]   ; bInterfaceProtocol / bAlternateSetting = DontCare
```

Second arg is a **pipe bitmask**: bit0 = bulk IN, bit1 = bulk OUT, bit2 = interrupt IN,
bit3 = interrupt OUT. The display opens **bulk OUT only** (mask 2).

`[OBS]` **The endpoint address is NOT hardcoded.** The code enumerates pipes and keeps the IOKit
*pipeRef*, ignoring `bEndpointAddress`. `OSImpl::writeToPipe` @ `0x1001a1000`: `fPipe==2` →
interface `[this+0xe0]`, pipeRef `[this+0x6c]`, vtable `+0x100` = `WritePipe(pipeRef, buf, size)`.

**Portable matching recipe:** VID `0x17CC`, PID `0x21x0`, interface class `0xFF` subclass `0xBD`,
first bulk-OUT endpoint. `initializeWithBulkInterface` falls back to a plain VID/PID +
`"IOUSBDevice"` filter when `getDeviceFilter` misses (`0x100192bb4`), so a type-4 miss is non-fatal.

`[OBS]` No transfer-size or chunking limit in this layer. `maxPacketSize` is recorded
(`[this+0x74]`) but never used to split; one `WritePipe` per draw.

---

## 4. MK3 display geometry — NOT IN THIS BINARY `[OBS, negative result]`

Well-supported negative. The MK3 keyboard display is driven by a **different component**.

- **No MK3-keyboard device class exists here.** The only `NHL2::*::Device` implementations are
  `MaschineJam`, `Traktor::{KontrolS2MK3, KontrolS4MK3, KontrolX1MK3, KontrolS3}`, and
  `Komplete::KontrolMK1`. There is no `Komplete::KontrolMK3`.
- Every display-constructor callsite, with literal geometry:

| Callsite | Device | w × h | fmt | count |
|---|---|---|---|---|
| `0x1001c25e0` | `Traktor::KontrolS4MK3` | `0x140`×`0xF0` = **320×240** | **`0x21`** | 2 |
| `0x100038320` | `SERVER::MaschineStudio` | 480×272 | `0x60` | 2 |
| `0x10003b960` | `SERVER::TraktorKontrolD2` | 480×272 | `0x60` | 1 |
| `0x10004b5b0` | `SERVER::TraktorKontrolS5` | 480×272 | `0x60` | 2 |
| `0x10004dbc4` | `SERVER::TraktorKontrolS8` | 480×272 | `0x60` | 2 |
| `0x1001a7ff4` | `Traktor::KontrolX1MK3` | 128×64 | `0x00` | 5 |
| `0x1001b7918` | `Komplete::KontrolMK1` | 72×3 | `0x09` | 1 |

  **None is an MK3 keyboard.**
- `procedure_callers` on the `BulkDisplayImplementation` ctor (`0x100192630`) returns exactly one
  caller — the S4 MK3. The second ctor symbol `0x10030d920` has **zero** callers.
- Constructor arg mapping confirmed: `(uint w, uint h, DataFormat, uint8 count, ControllerDevice&)`
  — `stp w1, w2, [x0, #0x20]` at `0x100192768`, `strb w3, [x0,#0x34]`, and arg4 drives the loop
  allocating that many `GenericHID` objects → **arg4 is the display count** (D2=1, S4MK3=2).
- The MK3 keyboards appear here **only** as device-DB rows and HID report-map strings — no geometry.
- The unused type-4 entries independently corroborate this.

### Lead instead of a guess `[INF]`

`GenericHID::GenericHID(unsigned char index)` @ `0x100193460` allocates a **10-byte HID report**
whose id is `index - 8` (→ `0xF8` for index 0), and `drawBulk` reads its payload as
`[1..2] = width LE16, [3..4] = height LE16, [6] = DataFormat, [9] = 0x32`.

All three MK3 keyboards' report maps contain **`F-F8 W2 B6`** = feature report `0xF8`,
2 words + 6 bytes = **exactly 10 bytes**.

`[INF]` **The geometry is very likely readable from the hardware itself via HID feature report
`0xF8` (GET_REPORT)** rather than needing to be guessed. Direction is unproven — the agent writes
this report to configure; whether it reads back was not established.

**Next binary to analyse:** `NIHostIntegrationAgent` (29 MB), or the Komplete Kontrol host app
(not installed on this machine), or a per-device plugin.

---

## 5. Light guide — CORRECTION to public research `[OBS]`

The HWS1 device table (`___cxx_global_var_init.173` @ `0x1001899cc`) holds a report-map DSL string
per device. For `kksm3-88` (`0x10018a8f8`, literal at `0x100335eb8`):

```
I-1 b28 n2 I-2 b8 WC b10 I-3 b8 W3 W2 B1 I-AA W19 O-80 B2A O-81 B1E O-82 B58
O-A0 BE O-A1 BC8 W1 B1 O-A2 B2C O-A3 B90 O-A4 B80 O-AF B2 O-F3 B1 O-F4 B20
F-D0 D1 B1C F-D8 D2 W4 B10 F-D9 B20 F-F8 W2 B6
```

DSL: `I-`/`O-`/`F-` = input/output/feature report id; `B` = bytes, `W` = 16-bit words,
`D` = dwords, `b` = bits (hex counts).

**Output report `0x82` size tracks the key count exactly:**

| Model | Size | Keys |
|---|---|---|
| S49 | `B31` | 49 |
| S61 | `B3D` | 61 |
| **S88** | **`B58`** | **88** |

`[INF]` So `0x82` is the per-key light guide: **one byte per key, sized to the keybed — NOT 128
note-indexed entries** as community research reports.

Also `O-80` = 42 bytes and `O-81` = 30 bytes (button/pad LEDs), `O-AF` = 2 bytes.

`[OBS]` **No evidence for an `A0 00 00` / `A0 01 00` "legacy LED mode" command.** `O-A0` here is 14
bytes. Unverified either way — absence here is not proof it does not exist elsewhere.

---

## 6. Explicit unknowns

1. **MK3 display resolution and DataFormat value — still unknown.** Not in this binary. Do not
   guess; `0x60`/`0x61` are *plausible* given the family but **unproven for the MK3**.
2. Semantics of header bytes `+0x04..+0x07` (always 0 here) and the footer tag `0x0040`.
3. `eUSBDeviceType 4` semantics; nothing in this binary consumes it.
4. Which IOKit class name (`"IOUSBDevice"` vs `"IOHIDDevice"`) each MK3 entry's `+0x30` holds —
   both are 11 chars with identical tails; the register was not traced.
5. `DataFormat 0x1B`–`0x1F` exact layouts (read but unverified). `0x09` (KontrolMK1, 72×3) is
   rejected by `getDataFormatProperties` and handled on some other path.
6. Whether feature report `0xF8` is readable from the device (the geometry lead).

## Scratchpad artifacts

- `dis_devdb.txt` — full disassembly
- `findrefs.py` — reference resolver script
- `pixel_raw.txt` — extracted `renderPixelData` pseudocode
