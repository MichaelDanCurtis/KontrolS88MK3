//! Wire encoding. Every function is total and appends to a caller-owned
//! buffer so the hot path never allocates.

use cr_packs::Encoding;

/// Number of complete three-byte MIDI messages emitted by [`encode`].
///
/// This is intentionally separate from the byte count: aggregate bus budgets
/// are expressed in MIDI messages, while a single semantic parameter update
/// can require one (CC7), two (CC14), or four (NRPN) messages.
pub const fn message_count(enc: &Encoding) -> u8 {
    match enc {
        Encoding::Cc7 { .. } => 1,
        Encoding::Cc14 { .. } => 2,
        Encoding::Nrpn { .. } => 4,
    }
}

/// Bit depth a given encoding transmits.
pub fn resolution_bits(enc: &Encoding) -> u8 {
    match enc {
        Encoding::Cc7 { .. } => 7,
        Encoding::Cc14 { .. } | Encoding::Nrpn { .. } => 14,
    }
}

/// Map a real value in `min..=max` onto an unsigned integer of `bits` width.
pub fn quantize(value: f32, min: f32, max: f32, bits: u8) -> u16 {
    let max_raw: u16 = if bits >= 14 { 16383 } else { 127 };
    if !value.is_finite() || !min.is_finite() || !max.is_finite() || min >= max {
        return 0;
    }
    let t = ((value - min) / (max - min)).clamp(0.0, 1.0);
    // `round` then clamp keeps the result inside the representable range even
    // for float edge cases.
    let scaled = (t * max_raw as f32).round();
    scaled.clamp(0.0, max_raw as f32) as u16
}

/// Append the MIDI bytes for `raw` on `channel` (0-15) to `out`.
///
/// # Precondition
///
/// For `Encoding::Cc14 { msb_cc }`, the caller must ensure `msb_cc <= 95`.
/// The LSB byte always lands on `msb_cc + 32`, and this function performs no
/// range check of its own — that guard lives upstream, in
/// `cr_packs::DevicePack::validate`. If called directly with `msb_cc > 95`,
/// `msb_cc.wrapping_add(32) & 0x7F` wraps around instead of overflowing, so
/// the LSB silently lands on an unrelated low-numbered CC. For example,
/// `msb_cc: 96` sends the LSB to CC 0 (Bank Select MSB) — a different and
/// potentially destructive control change, not a benign no-op.
pub fn encode(enc: &Encoding, channel: u8, raw: u16, out: &mut Vec<u8>) {
    let ch = channel & 0x0F;
    let status = 0xB0 | ch;
    match enc {
        Encoding::Cc7 { cc } => {
            out.extend_from_slice(&[status, cc & 0x7F, (raw & 0x7F) as u8]);
        }
        Encoding::Cc14 { msb_cc } => {
            let msb = ((raw >> 7) & 0x7F) as u8;
            let lsb = (raw & 0x7F) as u8;
            out.extend_from_slice(&[status, msb_cc & 0x7F, msb]);
            out.extend_from_slice(&[status, (msb_cc.wrapping_add(32)) & 0x7F, lsb]);
        }
        Encoding::Nrpn { msb, lsb } => {
            let data_msb = ((raw >> 7) & 0x7F) as u8;
            let data_lsb = (raw & 0x7F) as u8;
            out.extend_from_slice(&[status, 99, msb & 0x7F]);
            out.extend_from_slice(&[status, 98, lsb & 0x7F]);
            out.extend_from_slice(&[status, 6, data_msb]);
            out.extend_from_slice(&[status, 38, data_lsb]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cc7_is_three_bytes_on_the_right_channel() {
        let mut out = Vec::new();
        encode(&Encoding::Cc7 { cc: 29 }, 0, 64, &mut out);
        assert_eq!(out, vec![0xB0, 29, 64]);

        out.clear();
        encode(&Encoding::Cc7 { cc: 29 }, 15, 127, &mut out);
        assert_eq!(out, vec![0xBF, 29, 127]);
    }

    #[test]
    fn message_count_matches_each_encoding() {
        assert_eq!(message_count(&Encoding::Cc7 { cc: 29 }), 1);
        assert_eq!(message_count(&Encoding::Cc14 { msb_cc: 1 }), 2);
        assert_eq!(message_count(&Encoding::Nrpn { msb: 2, lsb: 5 }), 4);
    }

    #[test]
    fn cc14_emits_msb_then_lsb_at_offset_32() {
        let mut out = Vec::new();
        // 16383 == all 14 bits set => msb 127, lsb 127
        encode(&Encoding::Cc14 { msb_cc: 1 }, 0, 16383, &mut out);
        assert_eq!(out, vec![0xB0, 1, 127, 0xB0, 33, 127]);

        out.clear();
        // 128 == msb 1, lsb 0
        encode(&Encoding::Cc14 { msb_cc: 1 }, 0, 128, &mut out);
        assert_eq!(out, vec![0xB0, 1, 1, 0xB0, 33, 0]);
    }

    #[test]
    fn nrpn_emits_param_then_data_in_standard_order() {
        let mut out = Vec::new();
        encode(&Encoding::Nrpn { msb: 2, lsb: 5 }, 0, 300, &mut out);
        // 300 = 0b100101100 -> data msb 2, data lsb 44
        assert_eq!(
            out,
            vec![
                0xB0, 99, 2, // NRPN param MSB
                0xB0, 98, 5, // NRPN param LSB
                0xB0, 6, 2, // data entry MSB
                0xB0, 38, 44, // data entry LSB
            ]
        );
    }

    #[test]
    fn quantize_covers_full_7bit_range() {
        assert_eq!(quantize(0.0, 0.0, 127.0, 7), 0);
        assert_eq!(quantize(127.0, 0.0, 127.0, 7), 127);
        assert_eq!(quantize(63.5, 0.0, 127.0, 7), 64);
    }

    #[test]
    fn quantize_covers_full_14bit_range() {
        assert_eq!(quantize(0.0, 0.0, 1.0, 14), 0);
        assert_eq!(quantize(1.0, 0.0, 1.0, 14), 16383);
        assert_eq!(quantize(0.5, 0.0, 1.0, 14), 8192);
    }

    #[test]
    fn quantize_clamps_out_of_range_input() {
        assert_eq!(quantize(-50.0, 0.0, 127.0, 7), 0);
        assert_eq!(quantize(500.0, 0.0, 127.0, 7), 127);
    }

    #[test]
    fn quantize_is_total_for_degenerate_input() {
        assert_eq!(quantize(f32::NAN, 0.0, 1.0, 7), 0);
        assert_eq!(quantize(0.5, 1.0, 1.0, 7), 0);
        assert_eq!(quantize(0.5, f32::INFINITY, 1.0, 7), 0);
    }

    #[test]
    fn no_encoded_data_byte_ever_sets_the_status_bit() {
        // A data byte >= 0x80 would be interpreted as a new status byte and
        // would corrupt the stream.
        let encodings = [
            Encoding::Cc7 { cc: 29 },
            Encoding::Cc14 { msb_cc: 1 },
            Encoding::Nrpn { msb: 2, lsb: 5 },
        ];
        for enc in &encodings {
            for raw in [0u16, 1, 127, 128, 8192, 16383] {
                let mut out = Vec::new();
                encode(enc, 0, raw, &mut out);
                for (i, b) in out.iter().enumerate() {
                    if i % 3 == 0 {
                        assert!(*b >= 0x80, "byte {i} should be a status byte");
                    } else {
                        assert!(*b < 0x80, "data byte {i} has status bit set: {b:#04x}");
                    }
                }
            }
        }
    }
}
