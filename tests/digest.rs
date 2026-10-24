#[path = "support/world_digest.rs"]
mod digest;

use bedrock_leveldb::{Db, OpenOptions, WriteBatch, WriteOptions};
use rust_water_structure::{
    Block, BlockState, FormatId, Pos, Size, StructureData,
    nbt::{self, Compound, Encoding, Nbt},
};
use std::collections::BTreeMap;

fn state(name: &str) -> Nbt {
    Nbt::Compound(BTreeMap::from([
        ("name".into(), Nbt::String(format!("minecraft:{name}"))),
        ("states".into(), Nbt::Compound(Compound::new())),
        ("version".into(), Nbt::Int(18168865)),
    ]))
}

fn constant_storage(name: &str) -> Vec<u8> {
    let mut bytes = vec![0];
    bytes.extend(nbt::encode("", &state(name), Encoding::LittleEndian).unwrap());
    bytes
}

fn one_stone_storage(reverse_palette: bool) -> Vec<u8> {
    let mut bytes = vec![2];
    for word_index in 0..128 {
        let mut word = if reverse_palette { u32::MAX } else { 0 };
        if word_index == 0 {
            word ^= 1 << 17;
        }
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(2i32.to_le_bytes());
    for name in if reverse_palette {
        ["stone", "air"]
    } else {
        ["air", "stone"]
    } {
        bytes.extend(nbt::encode("", &state(name), Encoding::LittleEndian).unwrap());
    }
    bytes
}

fn key(x: i32, z: i32, tag: u8, y: Option<i8>) -> Vec<u8> {
    let mut key = x.to_le_bytes().to_vec();
    key.extend(z.to_le_bytes());
    key.push(tag);
    if let Some(y) = y {
        key.push(y as u8);
    }
    key
}

fn world(entries: Vec<(Vec<u8>, Vec<u8>)>) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let db = Db::open(directory.path().join("db"), OpenOptions::default()).unwrap();
    let mut batch = WriteBatch::default();
    for (key, value) in entries {
        batch.put(key, value);
    }
    db.write(batch, WriteOptions::default()).unwrap();
    db.flush().unwrap();
    drop(db);
    directory
}

#[test]
fn palette_order_explicit_air_layer_and_air_subchunks_do_not_change_digest() {
    let mut first = vec![9, 1, (-4i8) as u8];
    first.extend(one_stone_storage(false));
    let mut second = vec![9, 2, (-4i8) as u8];
    second.extend(one_stone_storage(true));
    second.extend(constant_storage("air"));
    let mut empty = vec![9, 1, 0];
    empty.extend(constant_storage("air"));
    let a = world(vec![(key(0, 0, b'/', Some(-4)), first)]);
    let b = world(vec![
        (key(0, 0, b'/', Some(-4)), second),
        (key(1, 0, b'/', Some(0)), empty),
    ]);
    let a = digest::digest_world(a.path()).unwrap();
    let b = digest::digest_world(b.path()).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.non_air, 1);
    assert_eq!(a.extra_non_air, 0);
    assert_eq!(a.subchunks, 1);
}

#[test]
fn missing_primary_layer_does_not_hide_extra_layer() {
    let mut bytes = vec![9, 2, (-4i8) as u8];
    bytes.extend(constant_storage("air"));
    bytes.extend(one_stone_storage(false));
    let directory = world(vec![(key(0, 0, b'/', Some(-4)), bytes)]);
    let result = digest::digest_world(directory.path()).unwrap();
    assert_eq!(result.non_air, 0);
    assert_eq!(result.extra_non_air, 1);
    assert_eq!(result.subchunks, 1);
}

#[test]
fn nbt_field_order_is_ignored_but_numeric_type_is_preserved() {
    let payload = |reverse: bool, short: bool| {
        let mut writer = nbt::Writer::new(Encoding::LittleEndian);
        writer.data.push(10);
        writer.string("").unwrap();
        let mut fields = vec![
            ("x", Nbt::Int(0)),
            ("y", Nbt::Int(-63)),
            ("z", Nbt::Int(1)),
            ("id", Nbt::String("Chest".into())),
            ("value", if short { Nbt::Short(7) } else { Nbt::Int(7) }),
        ];
        if reverse {
            fields.reverse();
        }
        for (name, value) in fields {
            writer.named(name, &value).unwrap();
        }
        writer.data.push(0);
        writer.data
    };
    let a = world(vec![(key(0, 0, b'1', None), payload(false, false))]);
    let b = world(vec![(key(0, 0, b'1', None), payload(true, false))]);
    let c = world(vec![(key(0, 0, b'1', None), payload(true, true))]);
    let a = digest::digest_world(a.path()).unwrap();
    let b = digest::digest_world(b.path()).unwrap();
    let c = digest::digest_world(c.path()).unwrap();
    assert_eq!(a.nbt_sha256, b.nbt_sha256);
    assert_ne!(a.nbt_sha256, c.nbt_sha256);
    assert_eq!(a.nbt_count, 1);
}

#[test]
fn world_and_normalized_structure_use_the_same_subchunk_layout() {
    let mut bytes = vec![9, 1, (-4i8) as u8];
    bytes.extend(one_stone_storage(false));
    let directory = world(vec![(key(0, 0, b'/', Some(-4)), bytes)]);
    let mut data = StructureData::new(FormatId::BDX);
    data.size = Size(16, 16, 16);
    data.blocks
        .insert(Pos(0, 1, 1), Block::new(BlockState::new("stone")));
    assert_eq!(
        digest::digest_world(directory.path()).unwrap(),
        digest::digest_structure(&data, Pos(0, -64, 0)).unwrap()
    );
}

#[test]
fn zipped_world_and_bdx_reader_match_directory_with_typed_nbt() {
    let mut data = StructureData::new(FormatId::BDX);
    data.size = Size(16, 16, 16);
    data.blocks
        .insert(Pos(0, 0, 0), Block::new(BlockState::new("stone")));
    let mut chest = Block::new(rust_water_structure::blocks::legacy("chest", 2));
    chest.nbt = Some(BTreeMap::from([
        ("id".into(), Nbt::String("Chest".into())),
        ("ByteValue".into(), Nbt::Byte(-3)),
        ("ShortValue".into(), Nbt::Short(513)),
        ("Raw".into(), Nbt::RawString(vec![0xaa, 0x41])),
        ("Items".into(), Nbt::List(vec![])),
    ]));
    data.blocks.insert(Pos(1, 1, 1), chest);
    let directory = tempfile::tempdir().unwrap();
    rust_water_structure::world::write_directory(directory.path(), &data, Pos(0, -64, 0), None)
        .unwrap();
    let expected = digest::digest_world(directory.path()).unwrap();
    assert_eq!(
        expected,
        digest::digest_structure(&data, Pos(0, -64, 0)).unwrap()
    );
    let zip_path = directory.path().join("world.mcworld");
    let file = std::fs::File::create(&zip_path).unwrap();
    rust_water_structure::world::write_directory_archive(directory.path(), file).unwrap();
    assert_eq!(expected, digest::digest_world(&zip_path).unwrap());

    let file = std::fs::File::create(&zip_path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    use std::io::Write;
    for entry in std::fs::read_dir(directory.path().join("db")).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            zip.start_file(
                format!("nested/world/db/{}", entry.file_name().to_string_lossy()),
                options,
            )
            .unwrap();
            let mut file = std::fs::File::open(entry.path()).unwrap();
            std::io::copy(&mut file, &mut zip).unwrap();
        }
    }
    zip.flush().unwrap();
    zip.finish().unwrap();
    assert_eq!(expected, digest::digest_world(&zip_path).unwrap());
    let bytes = rust_water_structure::formats::binary::encode(FormatId::BDX, &data).unwrap();
    if let Some(output) = std::env::var_os("DIGEST_FIXTURE_OUTPUT") {
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(std::path::Path::new(&output).join("fixture.bdx"), &bytes).unwrap();
        std::fs::write(
            std::path::Path::new(&output).join("fixture@[0,-64,0]~[15,-49,15].mcworld"),
            rust_water_structure::world::encode_directory(directory.path()).unwrap(),
        )
        .unwrap();
    }
    let reader =
        rust_water_structure::api::Reader::from_bytes("fixture.bdx", &bytes, Some(FormatId::BDX))
            .unwrap();
    assert_eq!(
        expected,
        digest::digest_structure(reader.data(), Pos(0, -64, 0)).unwrap()
    );
}
