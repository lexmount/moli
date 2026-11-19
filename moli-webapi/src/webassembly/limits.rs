const WASM_HEADER: &[u8; 8] = b"\0asm\x01\0\0\0";
const TABLE_SECTION_ID: u8 = 4;
const MEMORY_SECTION_ID: u8 = 5;

const V8_MAX_TABLE_ELEMENTS: u64 = 10_000_000;
const V8_MAX_MEMORY32_PAGES: u64 = 65_536;
const V8_MAX_MEMORY64_PAGES: u64 = 262_144;
const SPEC_MAX_MEMORY32_PAGES: u64 = 65_536;
const SPEC_MAX_MEMORY64_PAGES: u64 = 1 << 48;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NormalizedWasmModule {
    pub(super) bytes: Vec<u8>,
    pub(super) instantiation_exceeds_v8_limit: bool,
}

#[derive(Default)]
struct SectionRewrite {
    bytes: Vec<u8>,
    changed: bool,
    instantiation_exceeds_v8_limit: bool,
}

/// Lowers declared memory/table bounds only where V8's decoder applies a
/// stricter implementation limit than the WebAssembly specification.
///
/// The lowered maximum is not observable on V8 because no Memory or Table can
/// grow beyond the same process-wide implementation limit. An initial bound
/// above that limit is lowered only so compilation can represent the valid
/// module; callers must reject instantiation when the returned marker is set.
/// Unsupported proposal encodings are left entirely to V8.
pub(super) fn normalize_v8_implementation_limits(bytes: &[u8]) -> Option<NormalizedWasmModule> {
    if bytes.len() < WASM_HEADER.len() || &bytes[..WASM_HEADER.len()] != WASM_HEADER {
        return None;
    }

    let mut input = WASM_HEADER.len();
    let mut output = Vec::with_capacity(bytes.len());
    output.extend_from_slice(WASM_HEADER);
    let mut changed = false;
    let mut instantiation_exceeds_v8_limit = false;

    while input < bytes.len() {
        let section_id = *bytes.get(input)?;
        input += 1;
        let section_len = read_unsigned(bytes, &mut input, 32)? as usize;
        let section_end = input.checked_add(section_len)?;
        let payload = bytes.get(input..section_end)?;
        input = section_end;

        let rewrite = match section_id {
            TABLE_SECTION_ID => rewrite_table_section(payload)?,
            MEMORY_SECTION_ID => rewrite_memory_section(payload)?,
            _ => SectionRewrite {
                bytes: payload.to_vec(),
                ..SectionRewrite::default()
            },
        };
        changed |= rewrite.changed;
        instantiation_exceeds_v8_limit |= rewrite.instantiation_exceeds_v8_limit;
        output.push(section_id);
        write_unsigned(rewrite.bytes.len() as u64, &mut output);
        output.extend_from_slice(&rewrite.bytes);
    }

    changed.then_some(NormalizedWasmModule {
        bytes: output,
        instantiation_exceeds_v8_limit,
    })
}

fn rewrite_memory_section(payload: &[u8]) -> Option<SectionRewrite> {
    rewrite_limits_section(payload, LimitsKind::Memory)
}

fn rewrite_table_section(payload: &[u8]) -> Option<SectionRewrite> {
    rewrite_limits_section(payload, LimitsKind::Table)
}

#[derive(Clone, Copy)]
enum LimitsKind {
    Memory,
    Table,
}

fn rewrite_limits_section(payload: &[u8], kind: LimitsKind) -> Option<SectionRewrite> {
    let mut input = 0;
    let count = read_unsigned(payload, &mut input, 32)?;
    let mut output = Vec::with_capacity(payload.len());
    write_unsigned(count, &mut output);
    let mut changed = false;
    let mut instantiation_exceeds_v8_limit = false;

    for _ in 0..count {
        if matches!(kind, LimitsKind::Table) {
            copy_reference_type(payload, &mut input, &mut output)?;
        }
        let flags = read_unsigned(payload, &mut input, 32)?;
        // max, shared, and i64-address flags are the encodings understood by
        // this compatibility layer. Custom-page and future proposal shapes
        // remain V8-owned rather than being guessed here.
        if flags & !0x07 != 0 {
            return None;
        }
        let address64 = flags & 0x04 != 0;
        let has_maximum = flags & 0x01 != 0;
        let minimum = read_unsigned(payload, &mut input, if address64 { 64 } else { 32 })?;
        let maximum = if has_maximum {
            Some(read_unsigned(
                payload,
                &mut input,
                if address64 { 64 } else { 32 },
            )?)
        } else {
            None
        };

        let (spec_maximum, implementation_maximum) = match (kind, address64) {
            (LimitsKind::Memory, false) => (SPEC_MAX_MEMORY32_PAGES, V8_MAX_MEMORY32_PAGES),
            (LimitsKind::Memory, true) => (SPEC_MAX_MEMORY64_PAGES, V8_MAX_MEMORY64_PAGES),
            (LimitsKind::Table, false) => (u64::from(u32::MAX), V8_MAX_TABLE_ELEMENTS),
            (LimitsKind::Table, true) => (u64::MAX, V8_MAX_TABLE_ELEMENTS),
        };
        if minimum > spec_maximum
            || maximum.is_some_and(|maximum| maximum > spec_maximum || minimum > maximum)
        {
            return None;
        }

        let normalized_minimum = if minimum > implementation_maximum {
            changed = true;
            instantiation_exceeds_v8_limit = true;
            0
        } else {
            minimum
        };
        let normalized_maximum = maximum.map(|maximum| {
            if maximum > implementation_maximum {
                changed = true;
                implementation_maximum
            } else {
                maximum
            }
        });

        write_unsigned(flags, &mut output);
        write_unsigned(normalized_minimum, &mut output);
        if let Some(maximum) = normalized_maximum {
            write_unsigned(maximum, &mut output);
        }
    }

    (input == payload.len()).then_some(SectionRewrite {
        bytes: output,
        changed,
        instantiation_exceeds_v8_limit,
    })
}

fn copy_reference_type(payload: &[u8], input: &mut usize, output: &mut Vec<u8>) -> Option<()> {
    let first = *payload.get(*input)?;
    *input += 1;
    // 0x40 introduces the table-with-initializer encoding, whose constant
    // expression is deliberately outside this narrow rewriter.
    if first == 0x40 || first & 0x80 != 0 {
        return None;
    }
    output.push(first);
    if matches!(first, 0x63 | 0x64) {
        copy_signed_leb(payload, input, output, 5)?;
    }
    Some(())
}

fn copy_signed_leb(
    bytes: &[u8],
    input: &mut usize,
    output: &mut Vec<u8>,
    max_bytes: usize,
) -> Option<()> {
    for _ in 0..max_bytes {
        let byte = *bytes.get(*input)?;
        *input += 1;
        output.push(byte);
        if byte & 0x80 == 0 {
            return Some(());
        }
    }
    None
}

fn read_unsigned(bytes: &[u8], input: &mut usize, bits: u32) -> Option<u64> {
    let max_bytes = bits.div_ceil(7) as usize;
    let mut value = 0_u64;
    for index in 0..max_bytes {
        let byte = *bytes.get(*input)?;
        *input += 1;
        let payload = u64::from(byte & 0x7f);
        let shift = (index * 7) as u32;
        if shift >= bits && payload != 0 {
            return None;
        }
        let remaining_bits = bits.saturating_sub(shift);
        if remaining_bits < 7 && payload >= (1_u64 << remaining_bits) {
            return None;
        }
        value |= payload.checked_shl(shift).unwrap_or(0);
        if byte & 0x80 == 0 {
            if bits < 64 && value >= (1_u64 << bits) {
                return None;
            }
            return Some(value);
        }
    }
    None
}

fn write_unsigned(mut value: u64, output: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module_with_section(section_id: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = WASM_HEADER.to_vec();
        bytes.push(section_id);
        write_unsigned(payload.len() as u64, &mut bytes);
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn lowers_memory64_maximum_to_v8_limit_without_instantiation_guard() {
        let input = module_with_section(
            MEMORY_SECTION_ID,
            &[1, 5, 0, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x40],
        );
        let normalized = normalize_v8_implementation_limits(&input).unwrap();

        assert!(!normalized.instantiation_exceeds_v8_limit);
        assert_eq!(
            normalized.bytes,
            module_with_section(MEMORY_SECTION_ID, &[1, 5, 0, 0x80, 0x80, 0x10])
        );
    }

    #[test]
    fn lowers_spec_valid_oversized_initial_bounds_and_requires_guard() {
        let memory64 = module_with_section(
            MEMORY_SECTION_ID,
            &[1, 4, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x40],
        );
        let table32 = module_with_section(
            TABLE_SECTION_ID,
            &[1, 0x70, 0, 0xff, 0xff, 0xff, 0xff, 0x0f],
        );

        for input in [memory64, table32] {
            let normalized = normalize_v8_implementation_limits(&input).unwrap();
            assert!(normalized.instantiation_exceeds_v8_limit);
        }
    }

    #[test]
    fn leaves_spec_invalid_or_supported_declarations_to_v8() {
        let memory64_over_spec_max = module_with_section(
            MEMORY_SECTION_ID,
            &[1, 4, 0x81, 0x80, 0x80, 0x80, 0x80, 0x80, 0x40],
        );
        let ordinary_memory = module_with_section(MEMORY_SECTION_ID, &[1, 1, 1, 2]);
        let reversed_table_limits = module_with_section(
            TABLE_SECTION_ID,
            &[1, 0x70, 1, 0xff, 0xff, 0xff, 0x7f, 0xfe, 0xff, 0xff, 0x7f],
        );

        assert!(normalize_v8_implementation_limits(&memory64_over_spec_max).is_none());
        assert!(normalize_v8_implementation_limits(&ordinary_memory).is_none());
        assert!(normalize_v8_implementation_limits(&reversed_table_limits).is_none());
    }
}
