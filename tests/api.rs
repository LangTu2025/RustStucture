use rust_water_structure::{Block, BlockState, FormatId, Pos, Size, StructureData, api};

#[test]
fn format_detection_resets_input_for_each_candidate() {
    let input = br#"[{"name":"minecraft:stone","aux":0,"x":-1,"y":64,"z":2}]"#;
    let reader = api::Reader::from_bytes("building.json", input, None).unwrap();
    assert_eq!(reader.data().format, FormatId::RunAway);
    assert_eq!(reader.data().non_air_count(), 1);
}

#[test]
fn strict_export_rejects_data_loss_and_compatible_export_reports_it() {
    let mut data = StructureData::new(FormatId::MCStructure);
    data.size = Size(1, 1, 1);
    let mut block = Block::new(BlockState::new("stone"));
    block.extra = Some(BlockState::new("water"));
    data.insert(Pos(0, 0, 0), block);
    assert!(api::encode(FormatId::MCFunction, &data, api::LossPolicy::Strict).is_err());
    let output = api::encode(FormatId::MCFunction, &data, api::LossPolicy::Compatible).unwrap();
    assert!(output.report.extra_layers > 0);
    let reader = api::Reader::from_bytes("output.mcfunction", &output.bytes, None).unwrap();
    assert_eq!(reader.data().non_air_count(), 1);
}

#[test]
fn sibi_is_explicitly_unsupported() {
    let error = api::Reader::from_bytes("x.sibi", b"H4AAAA", Some(FormatId::SIBI)).unwrap_err();
    assert!(error.to_string().contains("SIBI"));
}

#[test]
fn negative_offsets_move_blocks_into_negative_chunks() {
    let mut data = StructureData::new(FormatId::RunAway);
    data.size = Size(1, 1, 1);
    data.insert(Pos(0, 0, 0), Block::new(BlockState::new("stone")));
    let mut reader = api::Reader::from_data(data).unwrap();
    reader.set_offset(Pos(-17, 2, -1)).unwrap();
    let chunks = reader.get_chunks(&[(-2, -1), (0, 0)]).unwrap();
    assert_eq!(
        chunks[&(-2, -1)][&Pos(15, -62, 15)].state.name,
        "minecraft:stone"
    );
    assert!(chunks[&(0, 0)].is_empty());
}

fn one_block() -> StructureData {
    let mut data = StructureData::new(FormatId::MCStructure);
    data.size = Size(1, 1, 1);
    data.insert(
        Pos(0, 0, 0),
        Block::new(rust_water_structure::blocks::legacy("stone", 0)),
    );
    data
}

#[test]
fn capabilities_follow_the_actual_codec_exports() {
    assert!(api::capabilities(FormatId::Construction).extra_layers);
    for format in [
        FormatId::Schematic,
        FormatId::SchemV1,
        FormatId::SchemV2,
        FormatId::Litematic,
        FormatId::MCStructure,
        FormatId::AxiomBP,
        FormatId::Construction,
    ] {
        assert!(api::capabilities(format).entities, "{format:?}");
    }
    assert!(api::capabilities(FormatId::Schematic).origin);
    assert!(api::capabilities(FormatId::MCFunction).origin);
    assert!(api::capabilities(FormatId::BDX).declared_size);
    assert!(!api::capabilities(FormatId::BCF).declared_size);
}

#[test]
fn construction_keeps_extra_layers_in_strict_mode() {
    let mut data = one_block();
    data.blocks.get_mut(&Pos(0, 0, 0)).unwrap().extra =
        Some(rust_water_structure::blocks::legacy("water", 0));
    let encoded = api::encode(FormatId::Construction, &data, api::LossPolicy::Strict).unwrap();
    let restored = api::decode(FormatId::Construction, &encoded.bytes).unwrap();
    assert!(restored.blocks[&Pos(0, 0, 0)].extra.is_some());
}

#[test]
fn strict_nbt_checks_fields_and_reconstructed_values() {
    use rust_water_structure::nbt::{Compound, Nbt};
    let mut data = one_block();
    let block = data.blocks.get_mut(&Pos(0, 0, 0)).unwrap();
    block.state = rust_water_structure::blocks::legacy("command_block", 0);
    block.nbt = Some(Compound::from([
        ("id".into(), Nbt::String("CommandBlock".into())),
        ("Command".into(), Nbt::String("say hello".into())),
        ("CustomField".into(), Nbt::Int(42)),
    ]));
    assert!(api::encode(FormatId::IBImport, &data, api::LossPolicy::Strict).is_err());
    let output = api::encode(FormatId::IBImport, &data, api::LossPolicy::Compatible).unwrap();
    assert_eq!(output.report.block_nbt, 1);
    block_nbt_remove_custom(&mut data);
    data.blocks
        .get_mut(&Pos(0, 0, 0))
        .unwrap()
        .nbt
        .as_mut()
        .unwrap()
        .insert("Version".into(), Nbt::Int(19));
    assert!(api::encode(FormatId::IBImport, &data, api::LossPolicy::Strict).is_err());
}

fn block_nbt_remove_custom(data: &mut StructureData) {
    data.blocks
        .get_mut(&Pos(0, 0, 0))
        .unwrap()
        .nbt
        .as_mut()
        .unwrap()
        .remove("CustomField");
}

#[test]
fn legacy_state_downgrades_are_reported_outside_schematic() {
    use rust_water_structure::nbt::Nbt;
    let mut data = one_block();
    data.blocks
        .get_mut(&Pos(0, 0, 0))
        .unwrap()
        .state
        .properties
        .insert("unsupported_state".into(), Nbt::Int(123));
    let output = api::encode(FormatId::RunAway, &data, api::LossPolicy::Compatible).unwrap();
    assert_eq!(output.report.block_states, 1);
    assert!(api::encode(FormatId::RunAway, &data, api::LossPolicy::Strict).is_err());
}

#[test]
fn quantized_axiom_bounds_and_archive_relocation_are_reported() {
    let data = one_block();
    for format in [FormatId::AxiomBP, FormatId::MCWorld] {
        assert!(
            api::encode(format, &data, api::LossPolicy::Strict).is_err(),
            "{format:?}"
        );
        let output = api::encode(format, &data, api::LossPolicy::Compatible).unwrap();
        assert!(output.report.bounds_or_origin, "{format:?}");
    }
}

#[test]
fn signed_offsets_are_materialized_with_air_padding_and_origin() {
    for (offset, origin, local) in [(2, 10, 2), (-2, 8, 0)] {
        let mut data = one_block();
        data.origin = Pos(10, 20, 30);
        data.offset = Pos(offset, 0, 0);
        let output = api::encode(FormatId::Schematic, &data, api::LossPolicy::Strict).unwrap();
        let restored = api::decode(FormatId::Schematic, &output.bytes).unwrap();
        assert_eq!(restored.size, Size(3, 1, 1));
        assert_eq!(restored.origin, Pos(origin, 20, 30));
        assert_eq!(
            restored.blocks[&Pos(local, 0, 0)].state.name,
            "minecraft:stone"
        );
    }
}

#[test]
fn export_rejects_coordinate_overflows_and_out_of_bounds_blocks() {
    let mut data = one_block();
    data.origin = Pos(i32::MIN, 0, 0);
    data.offset = Pos(-1, 0, 0);
    assert!(matches!(
        api::encode(FormatId::Schematic, &data, api::LossPolicy::Compatible),
        Err(rust_water_structure::Error::Invalid(_))
    ));
    let mut data = one_block();
    data.blocks
        .insert(Pos(1, 0, 0), Block::new(BlockState::new("glass")));
    assert!(matches!(
        api::encode(FormatId::Schematic, &data, api::LossPolicy::Compatible),
        Err(rust_water_structure::Error::Invalid(_))
    ));
}

#[test]
fn directory_export_uses_offset_and_reports_loss_without_archive_origin_warning() {
    let mut data = one_block();
    data.origin = Pos(10, 20, 30);
    data.offset = Pos(-2, 1, 0);
    let directory = tempfile::tempdir().unwrap();
    let report =
        api::encode_world_directory(directory.path(), &data, api::LossPolicy::Strict).unwrap();
    assert!(report.is_empty());
    let restored = rust_water_structure::world::read_directory(directory.path(), None).unwrap();
    assert_eq!(restored.origin, Pos(8, 20, 30));
    assert_eq!(restored.size, Size(3, 2, 1));
    assert_eq!(restored.blocks[&Pos(0, 1, 0)].state.name, "minecraft:stone");
    data.entities.push(Default::default());
    let directory = tempfile::tempdir().unwrap();
    assert!(api::encode_world_directory(directory.path(), &data, api::LossPolicy::Strict).is_err());
    assert!(!directory.path().join("db").exists());
    let report =
        api::encode_world_directory(directory.path(), &data, api::LossPolicy::Compatible).unwrap();
    assert_eq!(report.entities, 1);
    assert!(!report.bounds_or_origin);
}

#[test]
fn sparse_air_records_do_not_hide_air_boundary_loss() {
    let mut data = one_block();
    data.size = Size(3, 1, 1);
    data.blocks
        .insert(Pos(2, 0, 0), Block::new(BlockState::air()));
    let (_, report) =
        api::prepare_export(FormatId::NexusNP, &data, api::LossPolicy::Compatible).unwrap();
    assert!(report.bounds_or_origin);
}

#[test]
fn detection_preserves_binary_nbt_tag_ten_and_accepts_raw_schematic() {
    use std::io::Read;
    let data = one_block();
    for format in [FormatId::MCStructure, FormatId::AxiomBP] {
        let bytes = rust_water_structure::formats::java::encode(format, &data).unwrap();
        assert_eq!(bytes[0], 10);
        let restored = api::detect("unknown.bin", &bytes).unwrap();
        assert_eq!(restored.format, format);
    }
    let compressed =
        rust_water_structure::formats::java::encode(FormatId::Schematic, &data).unwrap();
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(compressed.as_slice())
        .read_to_end(&mut raw)
        .unwrap();
    assert_eq!(raw[0], 10);
    let restored = api::detect("unknown.bin", &raw).unwrap();
    assert_eq!(restored.format, FormatId::Schematic);
}

#[test]
fn textual_detection_still_accepts_leading_bom_and_whitespace() {
    let bytes = b"\xef\xbb\xbf \r\n\tsetblock ~ ~ ~ stone []\n";
    let restored = api::detect("unknown.txt", bytes).unwrap();
    assert_eq!(restored.format, FormatId::MCFunction);
    let json = b"\xef\xbb\xbf \r\n [{\"name\":\"stone\",\"aux\":0,\"x\":0,\"y\":0,\"z\":0}]";
    assert_eq!(
        api::detect("unknown.json", json).unwrap().format,
        FormatId::RunAway
    );
}
#[test]
fn strict_export_reports_unrepresentable_metadata() {
    let mut data =
        rust_water_structure::StructureData::new(rust_water_structure::FormatId::MCStructure);
    data.size = rust_water_structure::Size(1, 1, 1);
    data.insert(
        rust_water_structure::Pos::default(),
        rust_water_structure::Block::new(rust_water_structure::BlockState::new("stone")),
    );
    data.metadata
        .insert("custom".into(), rust_water_structure::nbt::Nbt::Long(7));
    assert!(
        rust_water_structure::api::encode(
            rust_water_structure::FormatId::MCFunction,
            &data,
            rust_water_structure::api::LossPolicy::Strict
        )
        .is_err()
    );
}
#[test]
fn text_nbt_export_reports_invalid_utf8_string_loss() {
    let mut data = one_block();
    data.blocks.get_mut(&Pos::default()).unwrap().nbt = Some(std::collections::BTreeMap::from([(
        "raw".into(),
        rust_water_structure::nbt::Nbt::RawString(vec![0xaa]),
    )]));
    assert!(api::encode(FormatId::MianYangV1, &data, api::LossPolicy::Strict).is_err());
}
