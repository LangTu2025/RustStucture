use rust_water_structure::{Block, BlockState, FormatId, Pos, Size, StructureData, world};

fn varint(input: &[u8], position: &mut usize) -> u64 {
    let mut value = 0;
    for shift in (0..64).step_by(7) {
        let byte = input[*position];
        *position += 1;
        value |= u64::from(byte & 127) << shift;
        if byte < 128 {
            return value;
        }
    }
    panic!("无效 SST footer varint");
}

#[test]
fn native_tables_contain_a_real_metaindex_block() {
    let directory = tempfile::tempdir().unwrap();
    let mut data = StructureData::new(FormatId::MCWorld);
    data.size = Size(1, 1, 1);
    data.insert(Pos::default(), Block::new(BlockState::new("stone")));
    world::write_directory(directory.path(), &data, Pos(0, -64, 0), None).unwrap();
    let table = std::fs::read_dir(directory.path().join("db"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|extension| extension == "ldb"))
        .unwrap();
    let bytes = std::fs::read(table).unwrap();
    let mut position = bytes.len() - 48;
    let offset = varint(&bytes, &mut position) as usize;
    let length = varint(&bytes, &mut position) as usize;
    assert!(
        length >= 8,
        "Go LevelDB 必须读取真实的空 metaindex，不能写 (0,0) 句柄"
    );
    assert_eq!(&bytes[offset..offset + length], &[0, 0, 0, 0, 1, 0, 0, 0]);
}
