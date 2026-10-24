use rust_water_structure::nbt::Nbt;
use rust_water_structure::{Block, BlockState, FormatId, Pos, Size, StructureData, world};

#[test]
fn world_database_preserves_negative_positions_and_extra_layer() {
    let temp = tempfile::tempdir().unwrap();
    let mut data = StructureData::new(FormatId::MCStructure);
    data.size = Size(18, 3, 1);
    let mut block = Block::new(BlockState::new("stone"));
    block.extra = Some(BlockState::new("water"));
    block.nbt = Some(std::collections::BTreeMap::from([
        ("id".into(), Nbt::String("Chest".into())),
        ("custom".into(), Nbt::Long(9007199254740993)),
    ]));
    data.insert(Pos(17, 2, 0), block.clone());
    world::write_directory(temp.path(), &data, Pos(-16, -64, -16), None).unwrap();
    let actual =
        world::read_directory(temp.path(), Some((Pos(-16, -64, -16), Pos(1, -62, -16)))).unwrap();
    let stored = &actual.blocks[&Pos(17, 2, 0)];
    assert_eq!(stored.state.name, "minecraft:stone");
    assert_eq!(stored.extra.as_ref().unwrap().name, "minecraft:water");
    assert_eq!(
        stored.nbt.as_ref().unwrap()["custom"],
        Nbt::Long(9007199254740993)
    );
}

#[test]
fn compact_dense_import_uses_correct_yzx_order() {
    use rust_water_structure::formats::java::dense::DenseStructure;
    let temp = tempfile::tempdir().unwrap();
    let mut indices = vec![0; 18 * 3];
    indices[2 * 18 + 17] = 1;
    let dense = DenseStructure {
        size: Size(18, 3, 1),
        origin: Pos::default(),
        palette: vec![BlockState::air(), BlockState::new("stone")],
        indices,
        extra_indices: None,
        nbt: Default::default(),
        loss: Default::default(),
    };
    world::write_dense_directory(temp.path(), &dense, Pos(-16, -64, 0), None).unwrap();
    let actual = world::read_directory(temp.path(), None).unwrap();
    assert_eq!(actual.non_air_count(), 1);
    assert_eq!(actual.blocks[&Pos(17, 2, 0)].state.name, "minecraft:stone");
}

#[test]
fn complete_world_export_rejects_nonempty_targets() {
    let directory = tempfile::tempdir().unwrap();
    let sentinel = directory.path().join("keep.txt");
    std::fs::write(&sentinel, b"existing user data").unwrap();
    let mut data = StructureData::new(FormatId::MCWorld);
    data.size = Size(1, 1, 1);
    data.insert(Pos::default(), Block::new(BlockState::new("stone")));
    assert!(world::write_directory(directory.path(), &data, Pos(0, -64, 0), None).is_err());
    assert_eq!(std::fs::read(sentinel).unwrap(), b"existing user data");
    assert!(!directory.path().join("db").exists());
}

#[test]
fn empty_world_exports_without_an_empty_sst_flush() {
    let directory = tempfile::tempdir().unwrap();
    let mut data = StructureData::new(FormatId::MCWorld);
    data.size = Size(1, 1, 1);
    world::write_directory(directory.path(), &data, Pos(0, -64, 0), None).unwrap();
    assert!(directory.path().join("level.dat").is_file());
}
#[test]
fn world_origin_overflow_returns_an_error() {
    let directory = tempfile::tempdir().unwrap();
    let mut data = StructureData::new(FormatId::MCWorld);
    data.size = Size(2, 1, 1);
    data.insert(Pos(1, 0, 0), Block::new(BlockState::new("stone")));
    assert!(world::write_directory(directory.path(), &data, Pos(i32::MAX, -64, 0), None).is_err());
}
#[test]
fn legacy_world_palettes_are_upgraded_before_native_lookup() {
    use rust_water_structure::nbt::{self, Encoding};
    let directory = tempfile::tempdir().unwrap();
    let db = bedrock_leveldb::Db::open(directory.path().join("db"), Default::default()).unwrap();
    let palette = Nbt::Compound(std::collections::BTreeMap::from([
        ("name".into(), Nbt::String("minecraft:grass".into())),
        ("states".into(), Nbt::Compound(Default::default())),
        ("version".into(), Nbt::Int(17959425)),
    ]));
    let mut value = vec![9, 1, (-4i8) as u8, 0];
    value.extend(nbt::encode("", &palette, Encoding::LittleEndian).unwrap());
    let mut key = vec![0; 8];
    key.extend([b'/', (-4i8) as u8]);
    db.put(key, value, Default::default()).unwrap();
    db.flush().unwrap();
    drop(db);
    world::write_metadata(
        directory.path(),
        Size(1, 1, 1),
        Pos(0, -64, 0),
        &Default::default(),
    )
    .unwrap();
    let actual = world::read_directory(directory.path(), None).unwrap();
    assert_eq!(
        actual.blocks[&Pos::default()].state.name,
        "minecraft:grass_block"
    );
}
