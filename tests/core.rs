use rust_water_structure::nbt::{self, Encoding, Nbt};
use rust_water_structure::{FormatId, Pos, Size};
use std::collections::BTreeMap;

#[test]
fn negative_coordinates_use_floor_chunks() {
    assert_eq!(Pos(-1, 0, -17).chunk(), (-1, -2));
    assert_eq!(Size(17, 1, 32).chunk_count(), 4);
}

#[test]
fn network_and_disk_nbt_preserve_types() {
    let value = BTreeMap::from([
        ("byte".into(), Nbt::Byte(-1)),
        ("int".into(), Nbt::Int(-300)),
        ("long".into(), Nbt::Long(i64::MIN + 1)),
        ("array".into(), Nbt::LongArray(vec![7, -1])),
        ("list".into(), Nbt::List(vec![Nbt::String("你好".into())])),
    ]);
    for encoding in [
        Encoding::BigEndian,
        Encoding::LittleEndian,
        Encoding::NetworkBigEndian,
        Encoding::NetworkLittleEndian,
    ] {
        let bytes = nbt::encode("", &Nbt::Compound(value.clone()), encoding).unwrap();
        assert_eq!(
            nbt::decode(&bytes, encoding).unwrap().1,
            Nbt::Compound(value.clone())
        );
    }
}

#[test]
fn malformed_nbt_returns_error() {
    assert!(
        nbt::decode(
            &[10, 0, 0, 7, 0, 1, b'x', 255, 255, 255, 255],
            Encoding::BigEndian
        )
        .is_err()
    );
}

#[test]
fn tag_reader_read_tag_byte() {
    let mut reader = nbt::Reader::new(&[0x42], Encoding::LittleEndian);
    assert_eq!(reader.byte().unwrap(), 0x42);
    assert_eq!(reader.pos, 1);
}

#[test]
fn tag_reader_read_tag_byte_eof_matches_go_reader() {
    let mut reader = nbt::Reader::new(&[], Encoding::LittleEndian);
    let error = reader.byte().unwrap_err();
    assert!(matches!(
        error,
        rust_water_structure::Error::Io(ref error)
            if error.kind() == std::io::ErrorKind::UnexpectedEof
    ));
    assert_eq!(reader.pos, 1);
}

#[test]
fn tag_reader_read_tag_type() {
    let mut reader = nbt::Reader::new(&[1, 0x42], Encoding::LittleEndian);
    assert_eq!(reader.tag_type().unwrap(), 1);
    assert_eq!(reader.pos, 1);
}

#[test]
fn tag_reader_read_tag_int16_uses_encoding_endianness() {
    let mut little = nbt::Reader::new(&[0x34, 0x12], Encoding::LittleEndian);
    assert_eq!(little.short().unwrap(), 0x1234);
    assert_eq!(little.pos, 2);

    let mut big = nbt::Reader::new(&[0x12, 0x34], Encoding::BigEndian);
    assert_eq!(big.short().unwrap(), 0x1234);
    assert_eq!(big.pos, 2);
}

#[test]
fn tag_reader_read_tag_int16_truncation_consumes_available_bytes() {
    let mut reader = nbt::Reader::new(&[0x34], Encoding::LittleEndian);
    let error = reader.short().unwrap_err();
    assert!(matches!(
        error,
        rust_water_structure::Error::Io(ref error)
            if error.kind() == std::io::ErrorKind::UnexpectedEof
    ));
    assert_eq!(reader.pos, 1);
}

#[test]
fn tag_reader_read_tag_int32_uses_all_encoding_layouts() {
    for (encoding, bytes) in [
        (Encoding::LittleEndian, vec![0xd4, 0xfe, 0xff, 0xff]),
        (Encoding::BigEndian, vec![0xff, 0xff, 0xfe, 0xd4]),
        (Encoding::NetworkLittleEndian, vec![0xd7, 0x04]),
        (Encoding::NetworkBigEndian, vec![0xff, 0xff, 0xfe, 0xd4]),
    ] {
        let mut reader = nbt::Reader::new(&bytes, encoding);
        assert_eq!(reader.int().unwrap(), -300, "{encoding:?}");
        assert_eq!(reader.pos, bytes.len());
    }
}

#[test]
fn tag_reader_read_tag_int32_fixed_width_truncation_consumes_available_bytes() {
    for encoding in [Encoding::LittleEndian, Encoding::BigEndian] {
        let mut reader = nbt::Reader::new(&[1, 2, 3], encoding);
        let error = reader.int().unwrap_err();
        assert!(matches!(
            error,
            rust_water_structure::Error::Io(ref error)
                if error.kind() == std::io::ErrorKind::UnexpectedEof
        ));
        assert_eq!(reader.pos, 3);
    }
}

#[test]
fn tag_reader_read_tag_int32_network_fifth_byte_matches_go_uint32_truncation() {
    let bytes = [0x80, 0x80, 0x80, 0x80, 0x7f];
    let mut reader = nbt::Reader::new(&bytes, Encoding::NetworkLittleEndian);
    assert_eq!(reader.int().unwrap(), 0x7800_0000);
    assert_eq!(reader.pos, 5);
}

#[test]
fn tag_reader_read_tag_int64_uses_all_encoding_layouts() {
    for (encoding, bytes) in [
        (
            Encoding::LittleEndian,
            vec![0xd4, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        ),
        (
            Encoding::BigEndian,
            vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe, 0xd4],
        ),
        (Encoding::NetworkLittleEndian, vec![0xd7, 0x04]),
        (
            Encoding::NetworkBigEndian,
            vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe, 0xd4],
        ),
    ] {
        let mut reader = nbt::Reader::new(&bytes, encoding);
        assert_eq!(reader.long().unwrap(), -300, "{encoding:?}");
        assert_eq!(reader.pos, bytes.len());
    }
}

#[test]
fn tag_reader_read_tag_int64_fixed_width_truncation_consumes_available_bytes() {
    for encoding in [Encoding::LittleEndian, Encoding::BigEndian] {
        let mut reader = nbt::Reader::new(&[1, 2, 3, 4, 5, 6, 7], encoding);
        let error = reader.long().unwrap_err();
        assert!(matches!(
            error,
            rust_water_structure::Error::Io(ref error)
                if error.kind() == std::io::ErrorKind::UnexpectedEof
        ));
        assert_eq!(reader.pos, 7);
    }
}

#[test]
fn tag_reader_read_tag_int64_network_tenth_byte_matches_go_uint64_truncation() {
    let bytes = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f];
    let mut reader = nbt::Reader::new(&bytes, Encoding::NetworkLittleEndian);
    assert_eq!(reader.long().unwrap(), i64::MIN);
    assert_eq!(reader.pos, 10);
}

#[test]
fn tag_reader_read_tag_float32_preserves_encoding_and_bit_patterns() {
    for (encoding, bytes) in [
        (Encoding::LittleEndian, vec![0x00, 0x00, 0xc0, 0x7f]),
        (Encoding::BigEndian, vec![0x7f, 0xc0, 0x00, 0x00]),
    ] {
        let mut reader = nbt::Reader::new(&bytes, encoding);
        let value = reader.float().unwrap();
        assert!(value.is_nan(), "{encoding:?}");
        assert_eq!(value.to_bits(), 0x7fc0_0000, "{encoding:?}");
        assert_eq!(reader.pos, 4);
    }
}

#[test]
fn tag_reader_read_tag_float32_truncation_consumes_available_bytes() {
    let mut reader = nbt::Reader::new(&[1, 2, 3], Encoding::LittleEndian);
    let error = reader.float().unwrap_err();
    assert!(matches!(
        error,
        rust_water_structure::Error::Io(ref error)
            if error.kind() == std::io::ErrorKind::UnexpectedEof
    ));
    assert_eq!(reader.pos, 3);
}

#[test]
fn tag_reader_read_tag_float64_preserves_encoding_and_bit_patterns() {
    for (encoding, bytes) in [
        (
            Encoding::LittleEndian,
            vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf8, 0x7f],
        ),
        (
            Encoding::BigEndian,
            vec![0x7f, 0xf8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ),
    ] {
        let mut reader = nbt::Reader::new(&bytes, encoding);
        let value = reader.double().unwrap();
        assert!(value.is_nan(), "{encoding:?}");
        assert_eq!(value.to_bits(), 0x7ff8_0000_0000_0000, "{encoding:?}");
        assert_eq!(reader.pos, 8);
    }
}

#[test]
fn tag_reader_read_tag_float64_truncation_consumes_available_bytes() {
    let mut reader = nbt::Reader::new(&[1, 2, 3, 4, 5, 6, 7], Encoding::LittleEndian);
    let error = reader.double().unwrap_err();
    assert!(matches!(
        error,
        rust_water_structure::Error::Io(ref error)
            if error.kind() == std::io::ErrorKind::UnexpectedEof
    ));
    assert_eq!(reader.pos, 7);
}

#[test]
fn tag_reader_read_tag_int32_array_matches_all_encodings_and_empty_count() {
    let fixed_le = [2, 0, 0, 0, 1, 0, 0, 0, 0xfe, 0xff, 0xff, 0xff];
    let fixed_be = [0, 0, 0, 2, 0, 0, 0, 1, 0xff, 0xff, 0xff, 0xfe];
    for (encoding, bytes) in [(Encoding::LittleEndian, &fixed_le[..]), (Encoding::BigEndian, &fixed_be[..])] {
        let mut reader = nbt::Reader::new(bytes, encoding);
        assert_eq!(reader.int_array().unwrap(), [1, -2]);
        assert_eq!(reader.pos, bytes.len());
    }

    let mut network = nbt::Reader::new(&[8, 4, 2, 1, 2], Encoding::NetworkLittleEndian);
    assert_eq!(network.int_array().unwrap(), [2, 1, -1, 1]);

    let mut empty = nbt::Reader::new(&[0, 0, 0, 0], Encoding::LittleEndian);
    let values = empty.int_array().unwrap();
    assert!(values.is_empty());
}

#[test]
fn tag_reader_read_tag_int32_array_truncation_consumes_available_bytes() {
    let mut reader = nbt::Reader::new(&[2, 0, 0, 0, 1, 0, 0], Encoding::LittleEndian);
    assert!(reader.int_array().is_err());
    assert_eq!(reader.pos, 7);
}

#[test]
fn tag_reader_read_tag_int32_array_negative_length_panics_like_go() {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut reader = nbt::Reader::new(&[0xff, 0xff, 0xff, 0xff], Encoding::LittleEndian);
        let _ = reader.int_array();
    }));
    let payload = result.expect_err("negative array length should panic");
    let text = payload.downcast_ref::<&str>().copied().unwrap_or_default();
    assert_eq!(text, "runtime error: makeslice: len out of range");
}

#[test]
fn tag_reader_read_tag_int64_array_matches_all_encodings_and_empty_count() {
    let fixed_le = [2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
    let fixed_be = [0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 1, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe];
    for (encoding, bytes) in [(Encoding::LittleEndian, &fixed_le[..]), (Encoding::BigEndian, &fixed_be[..])] {
        let mut reader = nbt::Reader::new(bytes, encoding);
        assert_eq!(reader.long_array().unwrap(), [1, -2]);
        assert_eq!(reader.pos, bytes.len());
    }

    let mut network = nbt::Reader::new(&[8, 4, 2, 1, 2], Encoding::NetworkLittleEndian);
    assert_eq!(network.long_array().unwrap(), [2, 1, -1, 1]);

    let mut empty = nbt::Reader::new(&[0, 0, 0, 0], Encoding::LittleEndian);
    assert!(empty.long_array().unwrap().is_empty());
}

#[test]
fn tag_reader_read_tag_int64_array_truncation_consumes_available_bytes() {
    let mut reader = nbt::Reader::new(&[2, 0, 0, 0, 1, 0, 0, 0], Encoding::LittleEndian);
    assert!(reader.long_array().is_err());
    assert_eq!(reader.pos, 8);
}

#[test]
fn tag_reader_read_tag_int64_array_negative_length_panics_like_go() {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut reader = nbt::Reader::new(&[0xff, 0xff, 0xff, 0xff], Encoding::LittleEndian);
        let _ = reader.long_array();
    }));
    let payload = result.expect_err("negative array length should panic");
    let text = payload.downcast_ref::<&str>().copied().unwrap_or_default();
    assert_eq!(text, "runtime error: makeslice: len out of range");
}

#[test]
fn tag_reader_read_tag_list_decodes_values_end_items_and_nested_lists() {
    let mut ints = nbt::Reader::new(&[3, 2, 0, 0, 0, 1, 0, 0, 0, 0xfe, 0xff, 0xff, 0xff], Encoding::LittleEndian);
    assert_eq!(ints.list().unwrap(), [Nbt::Int(1), Nbt::Int(-2)]);
    assert_eq!(ints.pos, 13);

    let mut ends = nbt::Reader::new(&[0, 2, 0, 0, 0, 0xaa], Encoding::LittleEndian);
    assert_eq!(ends.list().unwrap(), [Nbt::End, Nbt::End]);
    assert_eq!(ends.pos, 5);
    assert_eq!(ends.byte().unwrap(), 0xaa);

    let mut nested = nbt::Reader::new(&[9, 1, 0, 0, 0, 1, 1, 0, 0, 0, 9], Encoding::LittleEndian);
    assert_eq!(nested.list().unwrap(), [Nbt::List(vec![Nbt::Byte(9)])]);
}

#[test]
fn tag_reader_read_tag_list_checks_type_before_count_and_handles_empty() {
    let mut invalid = nbt::Reader::new(&[13, 0, 0, 0, 0], Encoding::LittleEndian);
    assert!(invalid.list().is_err());
    assert_eq!(invalid.pos, 1);

    let mut empty = nbt::Reader::new(&[3, 0, 0, 0, 0], Encoding::LittleEndian);
    assert!(empty.list().unwrap().is_empty());
    assert_eq!(empty.pos, 5);
}

#[test]
fn tag_reader_read_tag_list_negative_count_errors_and_truncation_consumes() {
    let mut negative = nbt::Reader::new(&[1, 0xff, 0xff, 0xff, 0xff], Encoding::LittleEndian);
    assert!(negative.list().is_err());
    assert_eq!(negative.pos, 5);

    let mut truncated = nbt::Reader::new(&[1, 2, 0, 0, 0, 0x42], Encoding::LittleEndian);
    assert!(truncated.list().is_err());
    assert_eq!(truncated.pos, 7);
}

#[test]
fn tag_reader_read_tag_list_network_encoding() {
    let mut reader = nbt::Reader::new(&[3, 4, 2, 3], Encoding::NetworkLittleEndian);
    assert_eq!(reader.list().unwrap(), [Nbt::Int(1), Nbt::Int(-2)]);
    assert_eq!(reader.pos, 4);
}

#[test]
fn tag_reader_read_tag_list_low_level_has_no_decoder_depth_limit() {
    let mut bytes = Vec::new();
    for _ in 0..520 {
        bytes.extend([9, 1, 0, 0, 0]);
    }
    bytes.extend([0, 0, 0, 0, 0]);
    let mut reader = nbt::Reader::new(&bytes, Encoding::LittleEndian);
    assert!(reader.list().is_ok());

    let mut named = vec![9, 0, 0];
    named.extend_from_slice(&bytes);
    assert!(nbt::decode(&named, Encoding::LittleEndian).is_err());
}

#[test]
fn tag_reader_read_tag_compound_handles_end_nested_and_duplicate_names() {
    let bytes = [
        1, 1, 0, b'x', 1,
        1, 1, 0, b'x', 2,
        10, 1, 0, b'n', 1, 1, 0, b'y', 3, 0,
        0,
    ];
    let mut reader = nbt::Reader::new(&bytes, Encoding::LittleEndian);
    let Nbt::Compound(compound) = reader.compound().unwrap() else {
        panic!("expected UTF-8-keyed compound");
    };
    assert_eq!(compound["x"], Nbt::Byte(2));
    assert_eq!(
        compound["n"],
        Nbt::Compound(BTreeMap::from([("y".into(), Nbt::Byte(3))]))
    );
    assert_eq!(reader.pos, bytes.len());
}

#[test]
fn tag_reader_read_tag_compound_preserves_non_utf8_keys() {
    let bytes = [1, 1, 0, 0xff, 9, 0];
    let mut reader = nbt::Reader::new(&bytes, Encoding::LittleEndian);
    let compound = reader.compound().unwrap();
    assert_eq!(
        compound,
        Nbt::RawCompound(BTreeMap::from([(vec![0xff], Nbt::Byte(9))]))
    );
    for encoding in [
        Encoding::BigEndian,
        Encoding::LittleEndian,
        Encoding::NetworkBigEndian,
        Encoding::NetworkLittleEndian,
    ] {
        let encoded = nbt::encode("", &compound, encoding).unwrap();
        assert_eq!(nbt::decode(&encoded, encoding).unwrap().1, compound);
    }
}

#[test]
fn tag_reader_read_tag_compound_rejects_unknown_type_before_name() {
    let mut reader = nbt::Reader::new(&[13, 1, 0, b'x'], Encoding::LittleEndian);
    assert!(reader.compound().is_err());
    assert_eq!(reader.pos, 1);
}

#[test]
fn tag_reader_read_tag_compound_propagates_truncated_name_and_value() {
    let mut name = nbt::Reader::new(&[1, 2, 0, b'x'], Encoding::LittleEndian);
    assert!(name.compound().is_err());
    assert_eq!(name.pos, 4);

    let mut value = nbt::Reader::new(&[1, 1, 0, b'x'], Encoding::LittleEndian);
    assert!(value.compound().is_err());
    assert_eq!(value.pos, 5);
}

#[test]
fn tag_reader_read_tag_compound_stops_at_end_and_last_duplicate_wins() {
    let bytes = [
        1, 1, 0, b'x', 7,
        1, 1, 0, b'x', 9,
        0, 99,
    ];
    let mut reader = nbt::Reader::new(&bytes, Encoding::LittleEndian);
    assert_eq!(
        reader.compound().unwrap(),
        Nbt::Compound(BTreeMap::from([("x".into(), Nbt::Byte(9))]))
    );
    assert_eq!(reader.pos, 11);
    assert_eq!(reader.byte().unwrap(), 99);
}

#[test]
fn tag_reader_read_tag_compound_rejects_unknown_tag_before_name() {
    let mut reader = nbt::Reader::new(&[13, 1, 0, b'x'], Encoding::LittleEndian);
    assert!(reader.compound().is_err());
    assert_eq!(reader.pos, 1);
}

#[test]
fn tag_reader_read_tag_compound_preserves_non_utf8_name_bytes() {
    let raw = Nbt::RawCompound(BTreeMap::from([(vec![0xff, b'k'], Nbt::Byte(4))]));
    let encoded = nbt::encode("root", &raw, Encoding::LittleEndian).unwrap();
    let decoded = nbt::decode(&encoded, Encoding::LittleEndian).unwrap();
    assert_eq!(decoded.1, raw);
    assert_eq!(nbt::encode(&decoded.0, &decoded.1, Encoding::LittleEndian).unwrap(), encoded);
}

#[test]
fn tag_reader_skip_tag_byte_consumes_one_byte() {
    let mut reader = nbt::Reader::new(&[0x42, 0x43], Encoding::LittleEndian);
    reader.skip_byte().unwrap();
    assert_eq!(reader.byte().unwrap(), 0x43);
    assert_eq!(reader.pos, 2);
}

#[test]
fn tag_reader_skip_tag_byte_eof_advances_offset_like_go() {
    let mut reader = nbt::Reader::new(&[], Encoding::LittleEndian);
    assert!(reader.skip_byte().is_err());
    assert_eq!(reader.pos, 1);
}

#[test]
fn tag_reader_skip_tag_int16_consumes_two_bytes() {
    let mut reader = nbt::Reader::new(&[0x34, 0x12, 0x43], Encoding::LittleEndian);
    reader.skip_short().unwrap();
    assert_eq!(reader.byte().unwrap(), 0x43);
    assert_eq!(reader.pos, 3);
}

#[test]
fn tag_reader_skip_tag_int16_partial_and_eof_advance_offset() {
    let mut partial = nbt::Reader::new(&[0x34], Encoding::LittleEndian);
    assert!(partial.skip_short().is_err());
    assert_eq!(partial.pos, 1);

    let mut empty = nbt::Reader::new(&[], Encoding::LittleEndian);
    assert!(empty.skip_short().is_err());
    assert_eq!(empty.pos, 0);
}

#[test]
fn tag_reader_skip_tag_int32_always_consumes_four_fixed_bytes() {
    for encoding in [Encoding::LittleEndian, Encoding::BigEndian, Encoding::NetworkLittleEndian] {
        let mut reader = nbt::Reader::new(&[1, 2, 3, 4, 0x43], encoding);
        reader.skip_int().unwrap();
        assert_eq!(reader.byte().unwrap(), 0x43, "{encoding:?}");
        assert_eq!(reader.pos, 5);
    }
}

#[test]
fn tag_reader_skip_tag_int32_partial_and_eof_advance_offset() {
    let mut partial = nbt::Reader::new(&[1, 2, 3], Encoding::LittleEndian);
    assert!(partial.skip_int().is_err());
    assert_eq!(partial.pos, 3);

    let mut empty = nbt::Reader::new(&[], Encoding::LittleEndian);
    assert!(empty.skip_int().is_err());
    assert_eq!(empty.pos, 0);
}

#[test]
fn tag_reader_skip_tag_int64_always_consumes_eight_fixed_bytes() {
    for encoding in [Encoding::LittleEndian, Encoding::BigEndian, Encoding::NetworkLittleEndian] {
        let mut reader = nbt::Reader::new(&[1, 2, 3, 4, 5, 6, 7, 8, 0x43], encoding);
        reader.skip_long().unwrap();
        assert_eq!(reader.byte().unwrap(), 0x43, "{encoding:?}");
        assert_eq!(reader.pos, 9);
    }
}

#[test]
fn tag_reader_skip_tag_int64_partial_and_eof_advance_offset() {
    let mut partial = nbt::Reader::new(&[1, 2, 3, 4, 5, 6, 7], Encoding::LittleEndian);
    assert!(partial.skip_long().is_err());
    assert_eq!(partial.pos, 7);

    let mut empty = nbt::Reader::new(&[], Encoding::LittleEndian);
    assert!(empty.skip_long().is_err());
    assert_eq!(empty.pos, 0);
}

#[test]
fn tag_reader_skip_tag_float32_always_consumes_four_fixed_bytes() {
    for encoding in [Encoding::LittleEndian, Encoding::BigEndian, Encoding::NetworkLittleEndian] {
        let mut reader = nbt::Reader::new(&[1, 2, 3, 4, 0x43], encoding);
        reader.skip_float().unwrap();
        assert_eq!(reader.byte().unwrap(), 0x43, "{encoding:?}");
        assert_eq!(reader.pos, 5);
    }
}

#[test]
fn tag_reader_skip_tag_float32_partial_and_eof_advance_offset() {
    let mut partial = nbt::Reader::new(&[1, 2, 3], Encoding::LittleEndian);
    assert!(partial.skip_float().is_err());
    assert_eq!(partial.pos, 3);

    let mut empty = nbt::Reader::new(&[], Encoding::LittleEndian);
    assert!(empty.skip_float().is_err());
    assert_eq!(empty.pos, 0);
}

#[test]
fn tag_reader_skip_tag_float64_always_consumes_eight_fixed_bytes() {
    for encoding in [Encoding::LittleEndian, Encoding::BigEndian, Encoding::NetworkLittleEndian] {
        let mut reader = nbt::Reader::new(&[1, 2, 3, 4, 5, 6, 7, 8, 0x43], encoding);
        reader.skip_double().unwrap();
        assert_eq!(reader.byte().unwrap(), 0x43, "{encoding:?}");
        assert_eq!(reader.pos, 9);
    }
}

#[test]
fn tag_reader_skip_tag_float64_partial_and_eof_advance_offset() {
    let mut partial = nbt::Reader::new(&[1, 2, 3, 4, 5, 6, 7], Encoding::LittleEndian);
    assert!(partial.skip_double().is_err());
    assert_eq!(partial.pos, 7);

    let mut empty = nbt::Reader::new(&[], Encoding::LittleEndian);
    assert!(empty.skip_double().is_err());
    assert_eq!(empty.pos, 0);
}

#[test]
fn tag_reader_read_tag_and_end() {
    let mut reader = nbt::Reader::new(&[1, 4, 0, b't', b'e', b's', b't'], Encoding::LittleEndian);
    assert_eq!(reader.read_tag().unwrap(), (1, "test".into()));

    let mut end = nbt::Reader::new(&[0, 4, 0, b'x'], Encoding::LittleEndian);
    assert_eq!(end.read_tag().unwrap(), (0, String::new()));
    assert_eq!(end.pos, 1);
}

#[test]
fn tag_reader_read_tag_rejects_unknown_type_before_name() {
    let mut reader = nbt::Reader::new(&[13, 4, 0, b'x'], Encoding::LittleEndian);
    assert!(reader.read_tag().is_err());
    assert_eq!(reader.pos, 1);
}

#[test]
fn tag_reader_read_tag_value_byte_end_and_unknown() {
    let mut byte = nbt::Reader::new(&[0x42], Encoding::LittleEndian);
    assert_eq!(byte.read_tag_value(1).unwrap(), Some(Nbt::Byte(0x42)));

    let mut end = nbt::Reader::new(&[], Encoding::LittleEndian);
    assert_eq!(end.read_tag_value(0).unwrap(), None);
    assert_eq!(end.pos, 0);

    let mut unknown = nbt::Reader::new(&[], Encoding::LittleEndian);
    assert!(unknown.read_tag_value(13).is_err());
}

#[test]
fn tag_reader_read_tag_string() {
    let mut reader = nbt::Reader::new(&[2, 0, b'H', b'i'], Encoding::LittleEndian);
    assert_eq!(reader.string().unwrap(), "Hi");
    assert_eq!(reader.pos, 4);
}

#[test]
fn tag_reader_read_tag_string_preserves_bytes_for_all_encodings() {
    for (encoding, prefix) in [
        (Encoding::BigEndian, vec![0, 2]),
        (Encoding::LittleEndian, vec![2, 0]),
        (Encoding::NetworkBigEndian, vec![0, 2]),
        (Encoding::NetworkLittleEndian, vec![2]),
    ] {
        let mut bytes = prefix;
        bytes.extend([0xff, b'A']);
        let mut reader = nbt::Reader::new(&bytes, encoding);
        assert_eq!(reader.tag_string_bytes().unwrap(), [0xff, b'A']);
        assert_eq!(reader.pos, bytes.len());
    }
}

#[test]
fn tag_reader_read_tag_string_rejects_truncation_and_long_network_strings() {
    let mut truncated = nbt::Reader::new(&[2, 0, b'A'], Encoding::LittleEndian);
    assert!(truncated.tag_string_bytes().is_err());
    assert_eq!(truncated.pos, 3);

    let mut max_network_string = vec![0xff, 0xff, 0x01];
    max_network_string.resize(3 + i16::MAX as usize, b'x');
    let mut reader = nbt::Reader::new(&max_network_string, Encoding::NetworkLittleEndian);
    assert_eq!(reader.tag_string_bytes().unwrap().len(), i16::MAX as usize);

    let mut too_long = vec![0x80, 0x80, 0x02];
    too_long.resize(3 + i16::MAX as usize + 1, b'x');
    let mut reader = nbt::Reader::new(&too_long, Encoding::NetworkLittleEndian);
    assert!(reader.tag_string_bytes().is_err());
}

#[test]
fn tag_reader_read_tag_byte_array() {
    let mut reader = nbt::Reader::new(&[3, 0, 0, 0, 1, 2, 3], Encoding::LittleEndian);
    assert_eq!(reader.byte_array().unwrap(), [1, 2, 3]);
    assert_eq!(reader.pos, 7);
}

#[test]
fn tag_reader_read_tag_byte_array_negative_length() {
    let mut reader = nbt::Reader::new(&[255, 255, 255, 255], Encoding::LittleEndian);
    assert!(reader.byte_array().is_err());
    assert_eq!(reader.pos, 4);
}

#[test]
fn tag_reader_read_tag_byte_array_truncated_payload_consumes_available_bytes() {
    let mut reader = nbt::Reader::new(&[3, 0, 0, 0, 1, 2], Encoding::LittleEndian);
    assert!(reader.byte_array().is_err());
    assert_eq!(reader.pos, 6);
}

#[test]
fn tag_reader_read_named_tag_and_compound_end() {
    let input = [10, 0, 0, 1, 1, 0, b'x', 0x42, 0];
    let (name, value) = nbt::decode(&input, Encoding::LittleEndian).unwrap();
    assert_eq!(name, "");
    assert_eq!(value, Nbt::Compound(BTreeMap::from([("x".into(), Nbt::Byte(0x42))])));
}

#[test]
fn tag_reader_read_tag_value_byte() {
    let mut reader = nbt::Reader::new(&[0x42], Encoding::LittleEndian);
    assert_eq!(reader.value(1, 0).unwrap(), Nbt::Byte(0x42));
}

#[test]
fn tag_writer_round_trip() {
    let value = Nbt::Compound(BTreeMap::from([
        ("ByteVal".into(), Nbt::Byte(0x7f)),
        ("ShortVal".into(), Nbt::Short(12345)),
        ("IntVal".into(), Nbt::Int(123456789)),
        ("LongVal".into(), Nbt::Long(-9876543210)),
        ("FloatVal".into(), Nbt::Float(1.5)),
        ("DoubleVal".into(), Nbt::Double(2.25)),
        ("StringVal".into(), Nbt::String("hello".into())),
        ("ByteArrayVal".into(), Nbt::ByteArray(vec![1, 2, 3])),
        ("IntArrayVal".into(), Nbt::IntArray(vec![4, 5])),
        ("LongArrayVal".into(), Nbt::LongArray(vec![6, 7])),
        ("ListVal".into(), Nbt::List(vec![Nbt::Int(1), Nbt::Int(2), Nbt::Int(3)])),
        ("EmptyList".into(), Nbt::List(vec![])),
        (
            "CompoundVal".into(),
            Nbt::Compound(BTreeMap::from([
                ("NestedString".into(), Nbt::String("nested".into())),
                ("NestedList".into(), Nbt::List(vec![Nbt::Float(1.0)])),
            ])),
        ),
    ]));
    let bytes = nbt::encode("Root", &value, Encoding::NetworkLittleEndian).unwrap();
    assert_eq!(nbt::decode(&bytes, Encoding::NetworkLittleEndian).unwrap(), ("Root".into(), value));
}

#[test]
fn tag_writer_rejects_mixed_type_list() {
    let value = Nbt::List(vec![Nbt::Byte(1), Nbt::Int(2)]);
    assert!(nbt::encode("", &value, Encoding::LittleEndian).is_err());
}

#[test]
fn tag_reader_rejects_unknown_tag_type() {
    let mut reader = nbt::Reader::new(&[], Encoding::LittleEndian);
    assert!(reader.value(13, 0).is_err());
}

#[test]
fn non_utf8_nbt_strings_keep_original_bytes() {
    let bytes = [8, 0, 0, 1, 0, 0xaa];
    let (name, value) = nbt::decode(&bytes, Encoding::LittleEndian).unwrap();
    assert_eq!(value, Nbt::RawString(vec![0xaa]));
    assert_eq!(
        nbt::encode(&name, &value, Encoding::LittleEndian).unwrap(),
        bytes
    );
}

#[test]
fn original_format_ids_are_stable() {
    assert_eq!(FormatId::SIBI as u8, 32);
    assert_eq!(FormatId::NexusNP as u8, 36);
    assert_eq!(FormatId::ALL.len(), 37);
}

#[test]
fn disk_nbt_string_lengths_are_unsigned_16_bit() {
    for encoding in [
        Encoding::BigEndian,
        Encoding::LittleEndian,
        Encoding::NetworkBigEndian,
    ] {
        let mut bytes = vec![8];
        bytes.extend([0, 0]);
        bytes.extend([255, 255]);
        bytes.extend(std::iter::repeat_n(b'x', 65535));
        let expected = Nbt::String("x".repeat(65535));
        assert_eq!(nbt::decode(&bytes, encoding).unwrap().1, expected);
        assert_eq!(nbt::encode("", &expected, encoding).unwrap(), bytes);
    }
}
#[test]
fn oversized_dimensions_do_not_wrap_to_a_small_volume() {
    let size = Size(1 << 30, 16, 1 << 30);
    assert_eq!(size.volume(), u64::MAX);
}
#[test]
fn network_big_endian_omits_only_the_compound_root_name() {
    let string = [8, 0, 1, b'n', 0, 1, b'v'];
    assert_eq!(
        nbt::decode(&string, Encoding::NetworkBigEndian).unwrap(),
        ("n".into(), Nbt::String("v".into()))
    );
    assert_eq!(
        nbt::encode("n", &Nbt::String("v".into()), Encoding::NetworkBigEndian).unwrap(),
        string
    );
    assert_eq!(
        nbt::encode(
            "n",
            &Nbt::Compound(BTreeMap::new()),
            Encoding::NetworkBigEndian
        )
        .unwrap(),
        [10, 0]
    );
}
