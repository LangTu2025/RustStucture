use rust_water_structure::{
    Block, BlockState, FormatId, Pos, Size, StructureData,
    formats::java,
    nbt::{self, Encoding, Nbt},
};
use sha2::{Digest, Sha256};
use std::io::Write;

#[test]
#[ignore = "需要通过 DENSE_PROBE_FILES 指定本地大型结构样例"]
fn compact_external_probe() {
    let files = std::env::var("DENSE_PROBE_FILES").expect("缺少 DENSE_PROBE_FILES");
    let output_path =
        std::env::var("DENSE_PROBE_OUTPUT").unwrap_or_else(|_| "work/dense-probe.jsonl".into());
    let mut output = std::fs::File::create(output_path).unwrap();
    for file in files.split(';').filter(|file| !file.is_empty()) {
        let started = std::time::Instant::now();
        let format = if file.ends_with(".schematic") {
            FormatId::Schematic
        } else if file.ends_with(".mcstructure") {
            FormatId::MCStructure
        } else {
            FormatId::SchemV1
        };
        let result = java::dense::from_file(file, format).or_else(|error| {
            if format == FormatId::SchemV1 {
                java::dense::from_file(file, FormatId::SchemV2)
            } else {
                Err(error)
            }
        });
        let summary = match result {
            Ok(data) => {
                let air: Vec<_> = data.palette.iter().map(BlockState::is_air).collect();
                let count = data
                    .indices
                    .iter()
                    .filter(|index| !air[**index as usize])
                    .count();
                serde_json::json!({"file":file,"size":data.size,"origin":data.origin,"volume":data.size.volume(),"non_air":count,"palette":data.palette.len(),"nbt":data.nbt.len(),"index_bytes":data.indices.len()*4,"seconds":started.elapsed().as_secs_f64()})
            }
            Err(error) => {
                serde_json::json!({"file":file,"error":error.to_string(),"seconds":started.elapsed().as_secs_f64()})
            }
        };
        let line = serde_json::to_string(&summary).unwrap();
        writeln!(output, "{line}").unwrap();
        output.flush().unwrap();
        println!("{line}");
    }
}

fn compound(values: &[(&str, Nbt)]) -> Nbt {
    Nbt::Compound(
        values
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}

fn gzip(root: Nbt) -> Vec<u8> {
    let raw = nbt::encode("Schematic", &root, Encoding::BigEndian).unwrap();
    let mut zip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    zip.write_all(&raw).unwrap();
    zip.finish().unwrap()
}

fn primary_blocks_hash(data: &StructureData) -> String {
    let mut blocks: Vec<_> = data
        .blocks
        .iter()
        .filter(|(p, b)| p.1 < 384 && !b.state.is_air())
        .collect();
    blocks.sort_by_key(|(p, _)| (p.0.div_euclid(16), p.2.div_euclid(16), p.0, p.1, p.2));
    let mut hash = Sha256::new();
    for (p, block) in blocks {
        hash.update(format!(
            "{},{},{},0,{}\n",
            p.0,
            p.1,
            p.2,
            rust_water_structure::blocks::runtime_id(&block.state)
        ));
    }
    hex::encode(hash.finalize())
}

fn compact_fixture(
    format: FormatId,
    bytes: &[u8],
) -> rust_water_structure::formats::java::dense::DenseStructure {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fixture.nbt");
    std::fs::write(&path, bytes).unwrap();
    java::dense::from_file(path, format).unwrap()
}

#[test]
fn independent_schem_varints_and_entity_position() {
    let root = compound(&[
        ("Width", Nbt::Short(2)),
        ("Height", Nbt::Short(1)),
        ("Length", Nbt::Short(1)),
        ("Offset", Nbt::IntArray(vec![-8, 4, 9])),
        (
            "Palette",
            compound(&[
                ("minecraft:air", Nbt::Int(0)),
                ("minecraft:stone", Nbt::Int(128)),
            ]),
        ),
        ("BlockData", Nbt::ByteArray(vec![-128, 1, 0])),
        (
            "BlockEntities",
            Nbt::List(vec![compound(&[
                ("Pos", Nbt::IntArray(vec![0, 0, 0])),
                ("Id", Nbt::String("minecraft:chest".into())),
            ])]),
        ),
    ]);
    let bytes = gzip(root);
    let compact = compact_fixture(FormatId::SchemV1, &bytes);
    assert_eq!(compact.state(0, 0, 0, 0).unwrap().name, "minecraft:stone");
    assert!(compact.state(1, 0, 0, 0).unwrap().is_air());
    assert!(compact.nbt.contains_key(&Pos(0, 0, 0)));
    let decoded = java::decode(FormatId::SchemV1, &bytes).unwrap();
    assert_eq!(decoded.size, Size(2, 1, 1));
    assert_eq!(decoded.origin, Pos(-8, 4, 9));
    assert_eq!(decoded.blocks[&Pos(0, 0, 0)].state.name, "minecraft:stone");
    assert!(decoded.blocks[&Pos(0, 0, 0)].nbt.is_some());
}

#[test]
fn schematic_byte_arrays_start_after_the_four_byte_length() {
    let fixture = gzip(compound(&[
        ("Width", Nbt::Short(4)),
        ("Height", Nbt::Short(1)),
        ("Length", Nbt::Short(1)),
        ("Blocks", Nbt::ByteArray(vec![0, 1, 0, 0])),
        ("Data", Nbt::ByteArray(vec![0, 0, 0, 0])),
    ]));
    let data = java::decode(FormatId::Schematic, &fixture).unwrap();
    let compact = compact_fixture(FormatId::Schematic, &fixture);
    assert_eq!(compact.state(1, 0, 0, 0).unwrap().name, "minecraft:stone");
    assert!(compact.state(3, 0, 0, 0).unwrap().is_air());
    assert_eq!(data.blocks.len(), 1);
    assert_eq!(data.blocks[&Pos(1, 0, 0)].state.name, "minecraft:stone");
    let encoded = java::encode(FormatId::Schematic, &data).unwrap();
    let mut raw = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&encoded[..]), &mut raw).unwrap();
    let raw_compact = compact_fixture(FormatId::Schematic, &raw);
    assert_eq!(
        raw_compact.state(1, 0, 0, 0).unwrap().name,
        "minecraft:stone"
    );
    let (_, Nbt::Compound(root)) = nbt::decode(&raw, Encoding::BigEndian).unwrap() else {
        panic!("缺少根复合标签");
    };
    // 原 Go 的 Schematic 读取器会把数组长度 00 00 00 04 当成前四个方块，
    // 已确认属于原读取器缺陷；写入器保留标准 ByteArray，不能为此生成畸形 NBT。
    assert_eq!(root["Blocks"], Nbt::ByteArray(vec![0, 1, 0, 0]));
    assert_eq!(root["Data"], Nbt::ByteArray(vec![0, 0, 0, 0]));
}

#[test]
fn independent_mcstructure_extra_layer_and_nbt() {
    let root = compound(&[
        ("format_version", Nbt::Int(1)),
        (
            "size",
            Nbt::List(vec![Nbt::Int(1), Nbt::Int(1), Nbt::Int(1)]),
        ),
        (
            "structure_world_origin",
            Nbt::List(vec![Nbt::Int(-2), Nbt::Int(3), Nbt::Int(4)]),
        ),
        (
            "structure",
            compound(&[
                (
                    "block_indices",
                    Nbt::List(vec![
                        Nbt::List(vec![Nbt::Int(-1)]),
                        Nbt::List(vec![Nbt::Int(0)]),
                    ]),
                ),
                ("entities", Nbt::List(vec![])),
                (
                    "palette",
                    compound(&[(
                        "default",
                        compound(&[
                            (
                                "block_palette",
                                Nbt::List(vec![compound(&[
                                    ("name", Nbt::String("minecraft:water".into())),
                                    ("states", compound(&[])),
                                    ("version", Nbt::Int(18168865)),
                                ])]),
                            ),
                            (
                                "block_position_data",
                                compound(&[(
                                    "0",
                                    compound(&[(
                                        "block_entity_data",
                                        compound(&[("id", Nbt::String("Chest".into()))]),
                                    )]),
                                )]),
                            ),
                        ]),
                    )]),
                ),
            ]),
        ),
    ]);
    let bytes = nbt::encode("", &root, Encoding::LittleEndian).unwrap();
    let decoded = java::decode(FormatId::MCStructure, &bytes).unwrap();
    let compact = compact_fixture(FormatId::MCStructure, &bytes);
    assert!(compact.state(0, 0, 0, 0).unwrap().is_air());
    assert_eq!(compact.state(0, 0, 0, 1).unwrap().name, "minecraft:water");
    assert_eq!(compact.origin, Pos(-2, 3, 4));
    assert!(compact.nbt.contains_key(&Pos(0, 0, 0)));
    let block = &decoded.blocks[&Pos(0, 0, 0)];
    assert!(block.state.is_air());
    assert_eq!(block.extra.as_ref().unwrap().name, "minecraft:water");
    assert_eq!(decoded.origin, Pos(-2, 3, 4));
    assert!(block.nbt.is_some());
}

#[test]
fn compact_reader_uses_yzx_layout_and_preserves_nbt() {
    for format in [
        FormatId::Schematic,
        FormatId::SchemV1,
        FormatId::SchemV2,
        FormatId::MCStructure,
    ] {
        let mut data = StructureData::new(format);
        data.size = Size(3, 2, 4);
        data.origin = Pos(-3, 5, 7);
        let mut block = Block::new(BlockState::new("stone"));
        block.nbt = Some(
            [("raw".into(), Nbt::RawString(vec![0xaa, 0x41]))]
                .into_iter()
                .collect(),
        );
        data.blocks.insert(Pos(2, 1, 3), block);
        let bytes = java::encode(format, &data).unwrap();
        let compact = compact_fixture(format, &bytes);
        assert_eq!(compact.origin, data.origin, "{format:?}");
        assert_eq!(compact.size, data.size, "{format:?}");
        assert_eq!(
            compact.state(2, 1, 3, 0).unwrap().name,
            "minecraft:stone",
            "{format:?}"
        );
        assert!(compact.state(0, 0, 0, 0).unwrap().is_air());
        assert_eq!(
            compact.nbt[&Pos(2, 1, 3)]["raw"],
            Nbt::RawString(vec![0xaa, 0x41])
        );
    }
}

#[test]
fn construction_preserves_an_extra_layer_over_air() {
    let mut data = StructureData::new(FormatId::Construction);
    data.size = Size(2, 1, 2);
    data.origin = Pos(-5, 7, 15);
    let mut water = Block::new(BlockState::air());
    water.extra = Some(BlockState::new("water"));
    data.blocks.insert(Pos(1, 0, 1), water);
    let encoded = java::encode(FormatId::Construction, &data).unwrap();
    let decoded = java::decode(FormatId::Construction, &encoded).unwrap();
    let block = &decoded.blocks[&Pos(1, 0, 1)];
    assert!(block.state.is_air());
    assert_eq!(block.extra.as_ref().unwrap().name, "minecraft:water");
    let metadata_offset = u32::from_be_bytes(
        encoded[encoded.len() - 12..encoded.len() - 8]
            .try_into()
            .unwrap(),
    ) as usize;
    assert_eq!(
        &encoded[metadata_offset..metadata_offset + 2],
        &[0x1f, 0x8b]
    );
}

#[test]
fn every_java_writer_has_real_signature_and_preserves_stone() {
    for format in [
        FormatId::Schematic,
        FormatId::SchemV1,
        FormatId::SchemV2,
        FormatId::Litematic,
        FormatId::MCStructure,
        FormatId::AxiomBP,
        FormatId::Construction,
    ] {
        let mut data = StructureData::new(format);
        data.size = Size(3, 2, 4);
        data.blocks
            .insert(Pos(1, 1, 2), Block::new(BlockState::new("stone")));
        let bytes = java::encode(format, &data).unwrap();
        if let Some(directory) = std::env::var_os("JAVA_ORACLE_DIR") {
            std::fs::create_dir_all(&directory).unwrap();
            let extension = match format {
                FormatId::Schematic => "schematic",
                FormatId::SchemV1 => "v1",
                FormatId::SchemV2 => "v2",
                FormatId::Litematic => "litematic",
                FormatId::MCStructure => "mcstructure",
                FormatId::AxiomBP => "bp",
                FormatId::Construction => "construction",
                _ => unreachable!(),
            };
            std::fs::write(
                std::path::Path::new(&directory).join(format!("stone.{extension}")),
                &bytes,
            )
            .unwrap();
        }
        match format {
            FormatId::Construction => assert!(bytes.starts_with(b"constrct\0")),
            FormatId::AxiomBP => assert!(bytes.starts_with(&0x0ae5bb36u32.to_be_bytes())),
            FormatId::MCStructure => assert_eq!(bytes[0], 10),
            _ => assert!(bytes.starts_with(&[0x1f, 0x8b])),
        }
        let decoded = java::decode(format, &bytes).unwrap();
        assert_eq!(
            decoded.blocks[&Pos(1, 1, 2)].state.name,
            "minecraft:stone",
            "{format:?}"
        );
    }
}

#[test]
fn java_formats_preserve_typed_block_nbt_entities_and_origin() {
    for format in [
        FormatId::Schematic,
        FormatId::SchemV1,
        FormatId::SchemV2,
        FormatId::Litematic,
        FormatId::MCStructure,
        FormatId::AxiomBP,
        FormatId::Construction,
    ] {
        let mut data = StructureData::new(format);
        data.size = Size(3, 2, 4);
        data.origin = Pos(-32, 48, 16);
        let mut block = Block::new(BlockState::new("stone"));
        let nbt = compound(&[
            ("id", Nbt::String("minecraft:chest".into())),
            ("ByteValue", Nbt::Byte(-5)),
            ("ShortValue", Nbt::Short(513)),
            ("LongValue", Nbt::Long(1234567890123)),
            ("FloatValue", Nbt::Float(1.25)),
            ("DoubleValue", Nbt::Double(2.5)),
            ("ByteArrayValue", Nbt::ByteArray(vec![-1, 0, 1])),
            ("IntArrayValue", Nbt::IntArray(vec![-1, 2])),
            ("LongArrayValue", Nbt::LongArray(vec![i64::MAX, i64::MIN])),
            ("RawStringValue", Nbt::RawString(vec![0xaa, 0x41])),
        ]);
        block.nbt = nbt.as_compound().cloned();
        data.blocks.insert(Pos(1, 1, 2), block);
        let entity = compound(&[
            ("id", Nbt::String("minecraft:pig".into())),
            (
                "Pos",
                Nbt::List(vec![Nbt::Double(1.5), Nbt::Double(1.0), Nbt::Double(2.5)]),
            ),
        ]);
        data.entities.push(entity.as_compound().unwrap().clone());
        let encoded = java::encode(format, &data).unwrap();
        let decoded = java::decode(format, &encoded).unwrap();
        assert_eq!(decoded.origin, data.origin, "{format:?}");
        assert_eq!(decoded.entities, data.entities, "{format:?}");
        let actual = decoded.blocks[&Pos(1, 1, 2)].nbt.as_ref().unwrap();
        for (key, value) in nbt.as_compound().unwrap() {
            assert_eq!(actual.get(key), Some(value), "{format:?} {key}");
        }
    }
}

#[test]
fn axiom_non_aligned_origin_preserves_absolute_block_position() {
    let mut data = StructureData::new(FormatId::AxiomBP);
    data.origin = Pos(-5, 7, 15);
    data.size = Size(20, 3, 20);
    data.blocks
        .insert(Pos(19, 2, 19), Block::new(BlockState::new("stone")));
    let decoded = java::decode(
        FormatId::AxiomBP,
        &java::encode(FormatId::AxiomBP, &data).unwrap(),
    )
    .unwrap();
    assert_eq!(decoded.blocks.len(), 1);
    let p = decoded.blocks.keys().next().unwrap();
    assert_eq!(
        Pos(
            p.0 + decoded.origin.0,
            p.1 + decoded.origin.1,
            p.2 + decoded.origin.2
        ),
        Pos(14, 9, 34)
    );
}

#[test]
fn independent_litematic_crosses_long_boundary_and_handles_negative_size() {
    let air = compound(&[("Name", Nbt::String("minecraft:air".into()))]);
    let stone = compound(&[("Name", Nbt::String("minecraft:stone".into()))]);
    let mut palette = vec![air; 17];
    palette.push(stone);
    // 第 12 个方块的五位索引 17 跨越 long 边界，低位在 bit60，高位在下一 long bit0。
    let region = compound(&[
        (
            "Position",
            compound(&[("x", Nbt::Int(4)), ("y", Nbt::Int(0)), ("z", Nbt::Int(0))]),
        ),
        (
            "Size",
            compound(&[("x", Nbt::Int(-20)), ("y", Nbt::Int(1)), ("z", Nbt::Int(1))]),
        ),
        ("BlockStatePalette", Nbt::List(palette)),
        ("BlockStates", Nbt::LongArray(vec![1i64 << 60, 1])),
    ]);
    let root = compound(&[
        ("Version", Nbt::Int(6)),
        ("Regions", compound(&[("negative", region)])),
    ]);
    let data = java::decode(FormatId::Litematic, &gzip(root)).unwrap();
    assert_eq!(data.origin, Pos(-15, 0, 0));
    assert_eq!(data.size, Size(20, 1, 1));
    assert_eq!(data.blocks.len(), 1);
    assert_eq!(data.blocks[&Pos(12, 0, 0)].state.name, "minecraft:stone");
}

#[test]
fn independent_axiom_padding_keeps_values_inside_each_long() {
    let mut palette = vec![compound(&[("Name", Nbt::String("minecraft:air".into()))]); 17];
    palette.push(compound(&[("Name", Nbt::String("minecraft:stone".into()))]));
    let mut packed = vec![0; 4096usize.div_ceil(12)];
    packed[1] = 17;
    let region = compound(&[
        ("X", Nbt::Int(-2)),
        ("Y", Nbt::Int(3)),
        ("Z", Nbt::Int(1)),
        (
            "BlockStates",
            compound(&[
                ("palette", Nbt::List(palette)),
                ("data", Nbt::LongArray(packed)),
            ]),
        ),
    ]);
    let payload = gzip(compound(&[("BlockRegion", Nbt::List(vec![region]))]));
    let mut bytes = 0x0ae5bb36u32.to_be_bytes().to_vec();
    bytes.extend_from_slice(&0i32.to_be_bytes());
    bytes.extend_from_slice(&0i32.to_be_bytes());
    bytes.extend_from_slice(&(payload.len() as i32).to_be_bytes());
    bytes.extend_from_slice(&payload);
    let data = java::decode(FormatId::AxiomBP, &bytes).unwrap();
    assert_eq!(data.origin, Pos(-32, 48, 16));
    assert_eq!(data.blocks.len(), 1);
    assert_eq!(data.blocks[&Pos(12, 0, 0)].state.name, "minecraft:stone");
}

#[test]
fn independent_construction_section_index_and_xyz_order() {
    let section = compound(&[
        ("blocks_array_type", Nbt::Byte(7)),
        ("blocks", Nbt::ByteArray(vec![0, 1, 1, 0])),
        (
            "shape",
            Nbt::List(vec![Nbt::Int(2), Nbt::Int(1), Nbt::Int(2)]),
        ),
        (
            "block_entities",
            Nbt::List(vec![compound(&[
                ("x", Nbt::Int(-4)),
                ("y", Nbt::Int(5)),
                ("z", Nbt::Int(8)),
                ("namespace", Nbt::String("minecraft".into())),
                ("base_name", Nbt::String("chest".into())),
                (
                    "nbt",
                    compound(&[("CustomName", Nbt::String("fixture".into()))]),
                ),
            ])]),
        ),
    ]);
    let section = nbt::encode("", &section, Encoding::BigEndian).unwrap();
    let mut index = Vec::new();
    for coordinate in [-4i32, 5, 7] {
        index.extend_from_slice(&coordinate.to_le_bytes());
    }
    index.extend_from_slice(&[2, 1, 2]);
    index.extend_from_slice(&9i32.to_le_bytes());
    index.extend_from_slice(&(section.len() as i32).to_le_bytes());
    let entry = |name: &str| {
        compound(&[
            ("namespace", Nbt::String("minecraft".into())),
            ("blockname", Nbt::String(name.into())),
            ("properties", compound(&[])),
        ])
    };
    let mut air = entry("air");
    if let Nbt::Compound(entry) = &mut air {
        entry.insert("extra_blocks".into(), Nbt::List(vec![Nbt::Int(2)]));
    }
    let metadata = compound(&[
        ("selection_boxes", Nbt::IntArray(vec![-4, 5, 7, -2, 6, 9])),
        (
            "block_palette",
            Nbt::List(vec![air, entry("stone"), entry("water")]),
        ),
        (
            "section_index_table",
            Nbt::ByteArray(index.into_iter().map(|b| b as i8).collect()),
        ),
        ("section_version", Nbt::Byte(0)),
    ]);
    let mut bytes = b"constrct\0".to_vec();
    bytes.extend_from_slice(&section);
    let metadata_offset = bytes.len() as u32;
    bytes.extend_from_slice(&nbt::encode("", &metadata, Encoding::BigEndian).unwrap());
    bytes.extend_from_slice(&metadata_offset.to_be_bytes());
    bytes.extend_from_slice(b"constrct");
    let data = java::decode(FormatId::Construction, &bytes).unwrap();
    assert_eq!(data.origin, Pos(-4, 5, 7));
    assert_eq!(data.size, Size(2, 1, 2));
    assert!(data.blocks[&Pos(0, 0, 0)].state.is_air());
    assert_eq!(
        data.blocks[&Pos(0, 0, 0)].extra.as_ref().unwrap().name,
        "minecraft:water"
    );
    assert_eq!(data.blocks[&Pos(0, 0, 1)].state.name, "minecraft:stone");
    assert_eq!(data.blocks[&Pos(1, 0, 0)].state.name, "minecraft:stone");
    assert_eq!(
        data.blocks[&Pos(0, 0, 1)].nbt.as_ref().unwrap()["id"],
        Nbt::String("minecraft:chest".into())
    );
}

#[test]
fn decode_existing_go_samples_matches_oracle_counts() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../WaterStructure/test/测试文件");
    let mut errors = Vec::new();
    let mut summaries = Vec::new();
    for (name, format, expected_size, expected_count) in [
        (
            "123.construction",
            FormatId::Construction,
            Size(17, 16, 17),
            333,
        ),
        (
            "结构方块测试.mcstructure",
            FormatId::MCStructure,
            Size(12, 1, 12),
            61,
        ),
        (
            "影子门户.mcstructure",
            FormatId::MCStructure,
            Size(51, 38, 23),
            6150,
        ),
        (
            "建筑37.schem",
            FormatId::SchemV1,
            Size(136, 70, 135),
            185345,
        ),
        (
            "ziyounvshenxiang.schem",
            FormatId::SchemV2,
            Size(123, 207, 115),
            261653,
        ),
        (
            "破碎之梦·落叶就此消逝于尘埃.litematic",
            FormatId::Litematic,
            Size(256, 116, 197),
            621933,
        ),
        (
            "末至审判者.bp",
            FormatId::AxiomBP,
            Size(147, 119, 351),
            200100,
        ),
    ] {
        let path = directory.join(name);
        if !path.exists() {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let data = match java::decode(format, &bytes) {
            Ok(data) => data,
            Err(error) => {
                errors.push(format!("{name}: {error}"));
                continue;
            }
        };
        if matches!(
            format,
            FormatId::Schematic | FormatId::SchemV1 | FormatId::SchemV2 | FormatId::MCStructure
        ) {
            let dense = java::dense::from_file(&path, format).unwrap();
            assert_eq!(dense.size, data.size, "dense {name}");
            assert_eq!(dense.origin, data.origin, "dense {name}");
            let count = dense
                .indices
                .iter()
                .filter(|id| !dense.palette[**id as usize].is_air())
                .count() as u64;
            assert_eq!(count, expected_count, "dense {name}");
            for (p, block) in &data.blocks {
                assert_eq!(
                    dense.state(p.0 as u32, p.1 as u32, p.2 as u32, 0),
                    Some(&block.state),
                    "dense {name} {p:?}"
                );
                if let Some(extra) = &block.extra {
                    assert_eq!(
                        dense.state(p.0 as u32, p.1 as u32, p.2 as u32, 1),
                        Some(extra)
                    );
                }
                if let Some(nbt) = &block.nbt {
                    assert_eq!(dense.nbt.get(p), Some(nbt), "dense NBT {name} {p:?}");
                }
            }
        }
        if data.size != expected_size {
            errors.push(format!(
                "{name}: size {:?} expected {:?}",
                data.size, expected_size
            ));
        }
        if data.non_air_count() != expected_count {
            errors.push(format!(
                "{name}: count {} expected {}",
                data.non_air_count(),
                expected_count
            ));
        }
        let hash = primary_blocks_hash(&data);
        let expected_hash = match name {
            "123.construction" => {
                Some("61bb467081bf8ad4066a5e312a290debc1a565b7813c3a5a64a2146cf15fef1f")
            }
            "结构方块测试.mcstructure" => {
                Some("535d02a62f8e3801195e3a50d633c0989affcbc8fd428fb0c8d6c0b11a5455dc")
            }
            "影子门户.mcstructure" => {
                Some("fec9a838faf9acfda2bd62353e16b6558563830fa0f79145ef0de83efc22d785")
            }
            "建筑37.schem" => {
                Some("55bb4d3690fdc3827052a50cb05a2a610c273b1808b34af4145909358aea6033")
            }
            "ziyounvshenxiang.schem" => {
                Some("a79d19b0c2b40d523bcabc3fbaa98c51617c4e91b79c8c29b4defe9ea0808dc0")
            }
            "破碎之梦·落叶就此消逝于尘埃.litematic" => {
                Some("f1616d43bd8db957ebc47920b2240fb82d0a3cb981cae2c4762a4e567ef80668")
            }
            "末至审判者.bp" => {
                Some("9254669c2abe136ec77ff1c25e5674e7f645277f61d1da32ab40e68716d29278")
            }
            _ => None,
        };
        if let Some(expected_hash) = expected_hash
            && hash != expected_hash
        {
            errors.push(format!(
                "{name}: block hash {hash} expected {expected_hash}"
            ));
        }
        summaries.push(serde_json::json!({
            "file": name, "format": format.name(), "size": data.size, "origin": data.origin,
            "non_air": data.non_air_count(), "primary_blocks_sha256": hash,
            "block_nbt": data.blocks.values().filter(|b| b.nbt.is_some()).count(),
            "extra_layer": data.blocks.values().filter(|b| b.extra.is_some()).count(),
            "entities": data.entities.len(),
            "unknown_blocks": data.blocks.values().filter(|b| b.state.name == "minecraft:unknown").count(),
        }));
        if let Some(directory) = std::env::var_os("JAVA_ORACLE_DIR") {
            std::fs::create_dir_all(&directory).unwrap();
            if matches!(format, FormatId::MCStructure | FormatId::Construction) {
                let encoded = java::encode(format, &data).unwrap();
                std::fs::write(
                    std::path::Path::new(&directory).join(format!("rewritten-{name}")),
                    encoded,
                )
                .unwrap();
            }
            let air = rust_water_structure::blocks::runtime_id(&BlockState::air());
            let mut runtime_ids = vec![air; data.size.volume() as usize];
            for (p, block) in &data.blocks {
                if p.0 < 0
                    || p.1 < 0
                    || p.2 < 0
                    || p.0 >= data.size.0 as i32
                    || p.1 >= data.size.1 as i32
                    || p.2 >= data.size.2 as i32
                {
                    continue;
                }
                let index = (p.0 as usize * data.size.1 as usize + p.1 as usize)
                    * data.size.2 as usize
                    + p.2 as usize;
                runtime_ids[index] = rust_water_structure::blocks::runtime_id(&block.state);
            }
            let bytes: Vec<_> = runtime_ids.into_iter().flat_map(u32::to_le_bytes).collect();
            std::fs::write(
                std::path::Path::new(&directory).join(format!("{name}.primary.u32")),
                bytes,
            )
            .unwrap();
        }
    }
    if let Some(directory) = std::env::var_os("JAVA_ORACLE_DIR") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            std::path::Path::new(&directory).join("real-samples-summary.json"),
            serde_json::to_vec_pretty(&summaries).unwrap(),
        )
        .unwrap();
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
