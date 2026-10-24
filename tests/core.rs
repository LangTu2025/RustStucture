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
