#[path = "support/world_digest.rs"]
mod digest;

use rust_water_structure::{
    Block, BlockState, FormatId, Pos, Size, StructureData,
    formats::java::{
        self,
        dense::{self, DenseSource},
    },
    nbt::{Compound, Nbt},
};
use std::collections::BTreeMap;

fn fixture(format: FormatId) -> StructureData {
    let mut data = StructureData::new(format);
    data.size = Size(35, 19, 33);
    data.origin = Pos(-7, 3, 11);
    for p in [
        Pos(0, 0, 0),
        Pos(15, 15, 15),
        Pos(16, 16, 16),
        Pos(17, 0, 32),
        Pos(34, 18, 32),
    ] {
        data.blocks.insert(p, Block::new(BlockState::new("stone")));
    }
    let nbt: Compound = BTreeMap::from([
        ("id".into(), Nbt::String("Chest".into())),
        ("value".into(), Nbt::Short(513)),
        ("raw".into(), Nbt::RawString(vec![0xaa, 0x41])),
    ]);
    data.blocks.get_mut(&Pos(16, 16, 16)).unwrap().nbt = Some(nbt);
    if format == FormatId::MCStructure {
        let mut water = Block::new(BlockState::air());
        water.extra = Some(BlockState::new("water"));
        data.blocks.insert(Pos(33, 18, 32), water);
    }
    data
}

fn compare(a: &impl DenseSource, b: &impl DenseSource) {
    assert_eq!(a.size(), b.size());
    assert_eq!(a.origin(), b.origin());
    assert_eq!(a.palette(), b.palette());
    assert_eq!(a.block_nbt(), b.block_nbt());
    assert_eq!(a.loss().entities, b.loss().entities);
    assert_eq!(a.loss().metadata, b.loss().metadata);
    assert_eq!(a.has_extra(), b.has_extra());
    let mut expected = [0; 4096];
    let mut actual = [0; 4096];
    for cy in 0..a.size().1.div_ceil(16) {
        for cz in 0..a.size().2.div_ceil(16) {
            for cx in 0..a.size().0.div_ceil(16) {
                for layer in 0..2 {
                    let p = Pos(cx as i32 * 16, cy as i32 * 16, cz as i32 * 16);
                    a.fill_subchunk(p, layer, &mut expected).unwrap();
                    b.fill_subchunk(p, layer, &mut actual).unwrap();
                    assert_eq!(expected, actual, "tile {p:?} layer {layer}");
                }
            }
        }
    }
}

#[test]
fn forced_paging_matches_memory_coordinates_layers_and_nbt_for_all_dense_formats() {
    for format in [
        FormatId::Schematic,
        FormatId::SchemV1,
        FormatId::SchemV2,
        FormatId::MCStructure,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.nbt");
        std::fs::write(&path, java::encode(format, &fixture(format)).unwrap()).unwrap();
        let memory = dense::from_file(&path, format).unwrap();
        let paged = dense::from_file_bounded(&path, format, 256).unwrap();
        assert!(paged.is_paged(), "{format:?}");
        assert!(paged.temporary_bytes() > 0);
        compare(&memory, &paged);
        let striped = dense::from_file_bounded(&path, format, 32768).unwrap();
        assert!(striped.is_paged());
        compare(&memory, &striped);
        let automatic_memory = dense::from_file_bounded(&path, format, usize::MAX).unwrap();
        assert!(!automatic_memory.is_paged());
        compare(&memory, &automatic_memory);
    }
}

#[test]
fn paged_world_writer_has_the_same_semantic_digest_as_memory_writer() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.mcstructure");
    std::fs::write(
        &source,
        java::encode(FormatId::MCStructure, &fixture(FormatId::MCStructure)).unwrap(),
    )
    .unwrap();
    let memory = dense::from_file(&source, FormatId::MCStructure).unwrap();
    let paged = dense::from_file_bounded(&source, FormatId::MCStructure, 256).unwrap();
    let memory_world = directory.path().join("memory");
    let paged_world = directory.path().join("paged");
    rust_water_structure::world::write_dense_directory(
        &memory_world,
        &memory,
        Pos(0, -64, 0),
        None,
    )
    .unwrap();
    rust_water_structure::world::write_dense_source_directory(
        &paged_world,
        &paged,
        Pos(0, -64, 0),
        None,
    )
    .unwrap();
    let expected = digest::digest_world(&memory_world).unwrap();
    assert_eq!(expected, digest::digest_world(&paged_world).unwrap());
    let decoded = java::decode(FormatId::MCStructure, &std::fs::read(&source).unwrap()).unwrap();
    assert_eq!(
        expected,
        digest::digest_structure(&decoded, Pos(0, -64, 0)).unwrap()
    );
}

#[test]
fn absent_extra_layer_does_not_create_a_secondary_temporary_index() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.mcstructure");
    let mut data = fixture(FormatId::MCStructure);
    for block in data.blocks.values_mut() {
        block.extra = None;
    }
    std::fs::write(&path, java::encode(FormatId::MCStructure, &data).unwrap()).unwrap();
    let paged = dense::from_file_bounded(&path, FormatId::MCStructure, 1).unwrap();
    assert!(!paged.has_extra());
    let tiles = u64::from(data.size.0.div_ceil(16))
        * u64::from(data.size.1.div_ceil(16))
        * u64::from(data.size.2.div_ceil(16));
    assert_eq!(paged.temporary_bytes(), tiles * 4096 * 4);
}
