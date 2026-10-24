use crate::error::{LevelDbError, Result};

pub(crate) const VALUE_TYPE_DELETION: u8 = 0;
pub(crate) const VALUE_TYPE_VALUE: u8 = 1;

pub(crate) fn put_varint32(mut value: u32, out: &mut Vec<u8>) {
    while value >= 0x80 {
        let byte = u8::try_from(value & 0x7f).expect("masked varint32 byte fits in u8");
        out.push(byte | 0x80);
        value >>= 7;
    }
    out.push(u8::try_from(value).expect("final varint32 byte fits in u8"));
}

pub(crate) fn get_varint32(input: &mut &[u8]) -> Result<u32> {
    let mut result = 0_u32;
    for index in 0..5 {
        let Some((&byte, rest)) = input.split_first() else {
            return Err(LevelDbError::corruption("truncated varint32".to_string()));
        };
        *input = rest;
        if index == 4 && byte > 0x0f {
            return Err(LevelDbError::corruption(
                "varint32 overflows u32".to_string(),
            ));
        }
        result |= u32::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            return Ok(result);
        }
    }
    Err(LevelDbError::corruption("varint32 is too long".to_string()))
}

pub(crate) fn get_varint64(input: &mut &[u8]) -> Result<u64> {
    let mut result = 0_u64;
    for index in 0..10 {
        let Some((&byte, rest)) = input.split_first() else {
            return Err(LevelDbError::corruption("truncated varint64".to_string()));
        };
        *input = rest;
        if index == 9 && byte > 0x01 {
            return Err(LevelDbError::corruption(
                "varint64 overflows u64".to_string(),
            ));
        }
        result |= u64::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            return Ok(result);
        }
    }
    Err(LevelDbError::corruption("varint64 is too long".to_string()))
}

pub(crate) fn put_varint64(mut value: u64, out: &mut Vec<u8>) {
    while value >= 0x80 {
        let byte = u8::try_from(value & 0x7f).expect("masked varint64 byte fits in u8");
        out.push(byte | 0x80);
        value >>= 7;
    }
    out.push(u8::try_from(value).expect("final varint64 byte fits in u8"));
}

pub(crate) fn put_length_prefixed_slice(value: &[u8], out: &mut Vec<u8>) -> Result<()> {
    let len = u32::try_from(value.len())
        .map_err(|_| LevelDbError::invalid_argument("slice is too large".to_string()))?;
    put_varint32(len, out);
    out.extend_from_slice(value);
    Ok(())
}

pub(crate) fn get_length_prefixed_slice<'a>(input: &mut &'a [u8]) -> Result<&'a [u8]> {
    let len = usize::try_from(get_varint32(input)?)
        .map_err(|_| LevelDbError::corruption("length does not fit usize".to_string()))?;
    if input.len() < len {
        return Err(LevelDbError::corruption(
            "truncated length-prefixed slice".to_string(),
        ));
    }
    let (value, rest) = input.split_at(len);
    *input = rest;
    Ok(value)
}

pub(crate) fn crc32c(bytes: &[u8]) -> u32 {
    finalize_crc32c(update_crc32c(!0_u32, bytes))
}

pub(crate) fn masked_crc32c(chunks: &[&[u8]]) -> u32 {
    let mut crc = !0_u32;
    for chunk in chunks {
        crc = update_crc32c(crc, chunk);
    }
    mask_crc(finalize_crc32c(crc))
}

fn update_crc32c(crc: u32, bytes: &[u8]) -> u32 {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("sse4.2") {
        // SAFETY: 运行时检查保证当前 CPU 支持此函数使用的 CRC32C 指令。
        #[allow(unsafe_code)]
        return unsafe { update_crc32c_hardware(crc, bytes) };
    }
    update_crc32c_portable(crc, bytes)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "table index is always below 256"
)]
const fn crc32c_table() -> [u32; 256] {
    let mut table = [0; 256];
    let mut index = 0;
    while index < table.len() {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            let mask = 0_u32.wrapping_sub(value & 1);
            value = (value >> 1) ^ (0x82f6_3b78 & mask);
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

fn update_crc32c_portable(mut crc: u32, bytes: &[u8]) -> u32 {
    const TABLE: [u32; 256] = crc32c_table();
    for &byte in bytes {
        crc = (crc >> 8) ^ TABLE[((crc ^ u32::from(byte)) & 255) as usize];
    }
    crc
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.2")]
#[allow(unsafe_code)]
unsafe fn update_crc32c_hardware(mut crc: u32, bytes: &[u8]) -> u32 {
    use std::arch::x86_64::{_mm_crc32_u8, _mm_crc32_u64};
    let mut words = bytes.chunks_exact(8);
    for word in words.by_ref() {
        let word = u64::from_le_bytes(word.try_into().expect("eight-byte CRC word"));
        crc = u32::try_from(_mm_crc32_u64(u64::from(crc), word))
            .expect("CRC32 instruction returns a 32-bit checksum");
    }
    for byte in words.remainder() {
        crc = _mm_crc32_u8(crc, *byte);
    }
    crc
}

const fn finalize_crc32c(crc: u32) -> u32 {
    !crc
}

pub(crate) const fn mask_crc(crc: u32) -> u32 {
    crc.rotate_right(15).wrapping_add(0xa282_ead8)
}

pub(crate) const fn unmask_crc(masked: u32) -> u32 {
    masked.wrapping_sub(0xa282_ead8).rotate_left(15)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrips() {
        for value in [0, 1, 127, 128, 16_384, u32::MAX] {
            let mut encoded = Vec::new();
            put_varint32(value, &mut encoded);
            let mut input = encoded.as_slice();
            assert_eq!(get_varint32(&mut input).expect("decode"), value);
            assert!(input.is_empty());
        }
    }

    #[test]
    fn varint32_rejects_overflow_and_truncation() {
        let mut overflow = [0xff, 0xff, 0xff, 0xff, 0x10].as_slice();
        assert!(get_varint32(&mut overflow).is_err());

        let mut truncated = [0x80].as_slice();
        assert!(get_varint32(&mut truncated).is_err());
    }

    #[test]
    fn varint64_roundtrips() {
        for value in [0, 1, 127, 128, 16_384, u64::MAX] {
            let mut encoded = Vec::new();
            put_varint64(value, &mut encoded);
            let mut input = encoded.as_slice();
            assert_eq!(get_varint64(&mut input).expect("decode"), value);
            assert!(input.is_empty());
        }
    }

    #[test]
    fn varint64_rejects_overflow_and_truncation() {
        let mut overflow = [0xff; 10].as_slice();
        assert!(get_varint64(&mut overflow).is_err());

        let mut truncated = [0x80].as_slice();
        assert!(get_varint64(&mut truncated).is_err());
    }

    #[test]
    fn crc_mask_roundtrips() {
        let crc = crc32c(b"abc");
        assert_eq!(unmask_crc(mask_crc(crc)), crc);
    }

    #[test]
    fn crc32c_matches_go_castagnoli_vectors_and_segmented_input() {
        for (length, expected) in [
            (0, 0x0000_0000),
            (1, 0x86b7_37ba),
            (7, 0x5110_a112),
            (8, 0x4079_5c72),
            (9, 0x6fe3_5da6),
            (63, 0xd701_3350),
            (4096, 0xe1c2_f7e8),
            (65536, 0x55dd_3dcd),
        ] {
            let data: Vec<u8> = (0..length).map(|index| (index * 31 + 7) as u8).collect();
            assert_eq!(crc32c(&data), expected);
            assert_eq!(finalize_crc32c(update_crc32c_portable(!0, &data)), expected);
            for split in [0, length / 3, length / 2, length] {
                assert_eq!(
                    masked_crc32c(&[&data[..split], &data[split..]]),
                    mask_crc(expected)
                );
            }
        }
        assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    }
}
