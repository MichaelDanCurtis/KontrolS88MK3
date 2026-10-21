# MK3 display: RLE/delta command stream — recovered from NIHardwareAgent

**Date:** 2026-10-20
**Target:** `NIHardwareAgent.arm64`, Mach-O arm64, sha256 `48e2e563ffc695e6db6947b7c4a06dc14d146ee4055bd97dc1607d47290a93b5`
**Procedure:** `NI::NHL2::Display::Bulk::renderPixelData` @ `0x100191228`
**Provider:** Ghidra 12.1.4, authority `shipped-artifact`, confidence `observed`
**Extracted pseudocode:** `scratchpad/pixel_raw.txt` (757 lines)

Ghidra pseudocode is **inference, not source**. `[OBS]` = literal code, `[INF]` = conclusion, `[UNK]` = undetermined.

## Headline

`renderPixelData` is the **RLE/delta framer**, NOT the pixel-format converter. Per-pixel colour
conversion happens in a callee, `renderPixel(uint32 srcPixel, DataFormat, uint8_t** cursor)`,
called from 9 sites. **No RGB565 arithmetic exists in this function** — no `0xF800`/`0x07E0`/`0x001F`,
no shifts by 11/5/3/2. So the 16-bit pixel layout is still open.

## Command stream opcodes `[OBS, numerically verified]`

The encoder builds words with ARM `rev` (rendered by Ghidra as shift/mask idioms). Evaluated
numerically for `n ∈ {1, 2, 3, 0x7f, 0x100, 0x1234, 0xABCDEF, 0x010203}`, memory layout ascending is
**1 opcode byte + 24-bit big-endian count**:

| byte 0 | bytes 1–3 | payload | cursor advance | meaning |
|---|---|---|---|---|
| `0x00` | N (BE24) | N words of literal pixel data | `slot + N + 1` words | literal run |
| `0x01` | N (BE24) | 1 word at `slot+1` | 2 words | repeat that word N times |
| `0x02` | N (BE24) | none | 1 word | skip N words (delta only) |
| `0x03` | `00 00 00` | none | 1 word | end of stream |

Example: `n = 0x1234`, tag 1 → word `0x34120001` → bytes `01 00 12 34`.

**N counts 32-bit WORDS, not pixels** — 1 word = 2 pixels at 16 bpp, 1 pixel at 32 bpp.

`[INF]` The command stream is MSB-first on the wire, produced by byte-reversing host little-endian
words.

## DataFormat `[OBS]`

Only three literal values appear:

| Value | Encoding |
|---|---|
| `0x50` | **32 bpp**, written inline, 1 pixel per word |
| `0x60` / `0x61` | **16 bpp**, two pixels per 32-bit word, low half-word = lower-x pixel |
| bit `0x40` clear | **raw/uncompressed**, bpp `[UNK]`, no delta support |

Anything else with bit 6 set, or `value <= 1`, returns 0.

`[INF]` `DataFormat` is a **packed bitfield**: bit 6 = compressed flag, a 3-bit subfield at bits 5:3
(`& 0x38`), bit 0 = variant. This is proven by the code re-testing those bits, though those
particular tests are tautological for `0x60`/`0x61` (inlined-predicate residue — **not** a second
dispatch).

`[UNK]` Enumerator names; what `0x61` changes vs `0x60` (same body here — the difference is in
`renderPixel`).

### `0x50` wire bytes `[OBS]`

Ascending: `0xFF`, `src>>16 & 0xFF`, `src>>8 & 0xFF`, `src & 0xFF`. Source bits 31:24 are
**discarded** and replaced by constant `0xFF`.

`[UNK]` **Channel identity.** Whether `src>>16` is R or B depends on `NI::UIA::Picture`'s internal
layout, which this function never reveals. **Do not assume RGB.**

### `0x60`/`0x61` → 16 bpp `[INF, five independent facts]`

1. Pixels are read in adjacent pairs and the pair is the comparison unit.
2. x advances by 2, width decrements by 2.
3. x origin and width must **both be even**, else the path is bypassed and 0 returned.
4. After a repeat patch: two `renderPixel` calls, slot advances exactly 2 words.
5. **Decisive** — the literal-run counter increments once per *pair* and is consumed as a `uint*`
   word count.

`[UNK — the critical gap]` RGB565 vs RGB555/ARGB1555 vs BGR565, and whether the 16-bit value is
byte-swapped within its half-word. Only `renderPixel` answers this.

## Source surface: `NI::UIA::Picture` offsets `[OBS]`

| Offset | Meaning |
|---|---|
| `+0x30` | pointer to pixel buffer, indexed `* 4` → **32 bits per source pixel always** |
| `+0x38` | row stride in pixels |
| `+0x3c` | `[INF]` height / second extent (only ever compared, never used in addressing) |
| `+0x60` | x sub-image offset |
| `+0x64` | y sub-image offset |

Addressing: `buf[(x + xoff) + stride * (y + yoff)]`. `[INF]` The source is **always 32 bpp**;
`DataFormat` describes the **wire** format only.

## Geometry and layout `[OBS]`

- Parameters are **origin + extent** `(x, y, w, h)`, not two corners.
- **No clipping** against the Picture's own dimensions — the caller owns clipping.
- **Strict row-major** in every path. No tiling, no block/swizzle addressing, **no output stride or
  row padding**. Rows are back to back.
- Raw path zero-pads to a 4-byte boundary once at the very end.
- Output budget is truncated to a 4-byte multiple; compressed paths require ≥ 8 bytes.

## Partial and delta updates — both supported `[OBS]`

This is the most consequential finding for building a client: **a dirty-rect protocol is not merely
possible, it is what the stock driver already does.**

- Partial rectangles are honoured directly as loop bounds.
- Delta updates work via an optional reference ("previous frame") Picture. Unchanged regions are
  coded with opcode `0x02`. Preconditions: reference non-NULL **and** matching `+0x38` stride
  **and** matching `+0x3c`.
- Encoder heuristics worth copying: a repeat run opens only at **3 or more** identical units (a
  2-unit repeat is cheaper as a literal), and before opening a skip run the encoder looks **2 units
  ahead** in both surfaces.

`[INF]` A no-change delta emits **nothing at all — not even the `0x03` terminator** — and the cursor
is rewound. An empty delta is therefore indistinguishable from an error at the call boundary (both
return 0 with the cursor restored).

## No header is written here `[OBS]`

The first write is always a command word. There are **no** length/display-index/command constants
beyond the four opcodes.

`[INF]` The USB bulk header — display index, geometry, command id — is written by the **caller**,
which also chooses the output budget. This fits the community observation that captured MK2/MK3
display payloads begin `0x84 0x00`: `0x84` is very likely a **caller-level bulk command header**,
not a pixel opcode. Decompiling the caller should confirm it.

## Return value `[OBS]`

Returns 0 for: format `<= 1`; bit-6-set format other than the three; odd x or width on `0x60`/`0x61`;
budget < 8; reference stride/extent mismatch; any overflow. Otherwise returns "at least one literal
or repeat emitted AND within limit". The raw path returns 1 unconditionally.

`[INF]` A 0 return means **discard this buffer** — on overflow `*cursor` is left mid-command.

## Open questions and what settles each

| Unknown | What settles it |
|---|---|
| **16-bit packing** (565? 555? channel order? intra-pixel swap?) | **`renderPixel` — highest priority.** Signature `(uint32 srcPixel, DataFormat, uint8_t** cursor)` |
| Channel identity for `0x50` | `NI::UIA::Picture` accessors / the producer of its `+0x30` buffer |
| bpp for raw (bit-6-clear) formats | `renderPixel` |
| DataFormat enumerator names, full value set, `0x60` vs `0x61` | `renderPixel`'s dispatch; the device capability table that chooses the format |
| **Display resolution** | Not here — this function is purely rectangle-driven |
| **USB endpoint, interface, transfer size, bulk header** | The **caller** of `0x100191228` |
| Whether `0x02` skip is legal on a first frame | Behavioural. The driver only emits it with a reference picture, implying the device retains the previous frame. Treat as **unverified**. |

`[INF, SPECULATIVE — explicitly flagged]` A consistent bitfield reading predicts uncompressed twins
at `0x10`, `0x20`/`0x21`, `0x18`. **Hypothesis only. Must be checked against `renderPixel` before
any byte reaches hardware.**

## Decompiler artifacts — flagged, not smoothed over

1. One `renderPixel` call site shows 2 args where the other 8 pass 3 — Ghidra lost the cursor arg.
   Not a second overload.
2. Tautological bit tests inside the `0x60`/`0x61` arm — inlined-predicate residue.
3. `*param_7 = (long)puVar20 + 0xd` mixes a byte offset with `uint*` arithmetic. Net effect is 4
   bytes at word `slot+3`; `0xd` is **not** a 13-byte stride.
4. Several `if ((a <= b && c != 0) && (b = a, c != 0))` forms — a conditional-move plus branch
   rendered badly. Intent is legible but **verify against disassembly for byte-exact run
   boundaries.**
5. Heavy variable reuse: the DataFormat variable is later reused as a run counter and byte-swap
   temporary. The dispatch meaning holds only at the four dispatch sites.

## Untrusted-content check

The decompiled body contains no strings, URLs, or anything resembling a directive (grepped for
`http`, `ignore`, `instruct`, `you must`, `please`, `system`, `prompt`, `exec`, `curl`, `download` —
zero hits). Nothing in the target was treated as an instruction.
