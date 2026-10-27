use rust_water_structure::formats::binary;
use rust_water_structure::{FormatId, Pos, Size};

#[test]
fn mcfunction_coordinates_fill_and_overwrite() {
    let source = b"# fixture\nfill ~-2 ~3 ~4 ~0 ~3 ~4 stone\nsetblock ~-1 ~3 ~4 air\n";
    let data = binary::decode(FormatId::MCFunction, source).unwrap();
    assert_eq!(data.origin, Pos(-2, 3, 4));
    assert_eq!(data.size, Size(3, 1, 1));
    assert_eq!(data.blocks[&Pos(0, 0, 0)].state.name, "minecraft:stone");
    assert!(data.blocks[&Pos(1, 0, 0)].state.is_air());
}

#[test]
fn command_states_support_colons_and_legacy_block_aliases() {
    let bytes = b"setblock ~ ~ ~ stone [\"stone_type\":\"granite\"]\nsetblock ~1 ~ ~ oak_stairs [\"upside_down_bit\"=false,\"weirdo_direction\"=2]\n";
    let data = binary::decode(FormatId::MCFunction, bytes).unwrap();
    assert_eq!(data.blocks[&Pos(0, 0, 0)].state.name, "minecraft:granite");
    assert_eq!(
        data.blocks[&Pos(1, 0, 0)].state.properties["weirdo_direction"].as_i64(),
        Some(2)
    );
}

#[test]
fn kbdx_reads_independent_little_endian_fixture() {
    let mut bytes = 1u32.to_le_bytes().to_vec();
    for value in [-10i32, 64, 9, 1, 0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(br#"{"minecraft:stone":1,"BlockEntityData":[]}"#);
    let data = binary::decode(FormatId::KBDX, &bytes).unwrap();
    assert_eq!(data.origin, Pos(-10, 64, 9));
    assert_eq!(data.non_air_count(), 1);
}

#[test]
fn kbdx_container_items_accept_numeric_strings_and_json_floats() {
    let mut bytes = 1u32.to_le_bytes().to_vec();
    for value in [5i32, 64, -3, 1, 0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(br#"{"chest":1.0,"BlockEntityData":[{"id":"chest","x":"5","y":64.0,"z":-3,"Items":[{"name":"stone","count":3.0,"damage":"2","slot":"7"}]}]}"#);
    let data = binary::decode(FormatId::KBDX, &bytes).unwrap();
    let nbt = data.blocks[&Pos(0, 0, 0)].nbt.as_ref().unwrap();
    let item = nbt["Items"].as_list().unwrap()[0].as_compound().unwrap();
    assert_eq!(nbt["id"].as_str(), Some("Chest"));
    assert_eq!(item["Name"].as_str(), Some("minecraft:stone"));
    assert_eq!(item["Count"].as_i64(), Some(3));
    assert_eq!(item["Damage"].as_i64(), Some(2));
    assert_eq!(item["Slot"].as_i64(), Some(7));
}

#[test]
fn msgpack_formats_accept_independent_wire_fixture() {
    // fixarray、fixstr 和负 fixint，直接编写 MessagePack 字节。
    let np = b"\x92\x91\x95\xa5stone\xfe\x40\x09\x00\x90";
    let data = binary::decode(FormatId::NexusNP, np).unwrap();
    assert_eq!(data.origin, Pos(-2, 64, 9));
    let bds = b"\x91\x91\x96\xa5stone\xfe\x40\x09\x00\xc2";
    assert_eq!(
        binary::decode(FormatId::BDS, bds).unwrap().non_air_count(),
        1
    );
}

#[test]
fn rejects_truncated_binary_inputs() {
    for format in [
        FormatId::BDX,
        FormatId::KBDX,
        FormatId::IBImport,
        FormatId::BDS,
        FormatId::BCF,
        FormatId::TIBI,
        FormatId::NexusNP,
    ] {
        assert!(binary::decode(format, &[0, 1]).is_err(), "{format:?}");
    }
}

fn bdx_container(body: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut bytes = b"BD@".to_vec();
    {
        let mut writer = brotli::CompressorWriter::new(&mut bytes, 4096, 4, 22);
        writer.write_all(body).unwrap();
    }
    bytes
}

#[test]
fn bdx_reads_constant_legacy_and_signed_movements() {
    let mut wire = b"BDX\0fixture\0\x01stone\0".to_vec();
    wire.extend_from_slice(&[28, 254, 29, 3, 30, 4, 7, 0, 0, 0, 0]);
    wire.extend_from_slice(b"XE");
    let data = binary::decode(FormatId::BDX, &bdx_container(&wire)).unwrap();
    assert_eq!(data.origin, Pos(-2, 0, 0));
    assert_eq!(data.size, Size(3, 4, 5));
    assert_eq!(data.blocks[&Pos(0, 3, 4)].state.name, "minecraft:stone");
    assert_eq!(data.metadata["author"].as_str(), Some("fixture"));
}

#[test]
fn bdx_encoder_uses_go_shortest_movement_opcodes() {
    use rust_water_structure::bdump::{Command, CommandReader};
    use rust_water_structure::{Block, BlockState, StructureData};
    use std::io::Read;

    let mut data = StructureData::new(FormatId::BDX);
    let boundaries = [0, 1, 3, 130, 258, 33_025, 65_793];
    for y in boundaries {
        data.insert(Pos(0, y, 0), Block::new(BlockState::new("stone")));
    }
    for z in boundaries {
        data.insert(Pos(0, 0, z), Block::new(BlockState::new("stone")));
    }
    for (y, z_values) in [
        (65_787, &[0, 1][..]),
        (65_788, &[0, 2][..]),
        (65_789, &[0, 128][..]),
        (65_790, &[0, 129][..]),
        (65_791, &[0, 32_768][..]),
        (65_792, &[0, 32_769][..]),
        (65_793, &[0][..]),
    ] {
        for z in z_values {
            data.insert(Pos(1, y, *z), Block::new(BlockState::new("stone")));
        }
    }
    for (x, y) in [
        (3, 65_792),
        (130, 65_790),
        (258, 65_663),
        (33_025, 65_535),
        (65_793, 65_406),
    ] {
        data.insert(Pos(x, y, 0), Block::new(BlockState::new("stone")));
    }
    data.size = Size(65_797, 65_794, 65_794);

    let encoded = binary::encode(FormatId::BDX, &data).unwrap();
    let mut body = Vec::new();
    brotli::Decompressor::new(&encoded[3..], 4096)
        .read_to_end(&mut body)
        .unwrap();
    assert!(body.starts_with(b"BDX\0\0"));

    let mut reader = CommandReader::new(&body[5..]);
    let mut movements = Vec::new();
    loop {
        let offset = reader.position;
        let Some(command) = reader.read().unwrap() else {
            break;
        };
        match command {
            Command::Move { axis, delta } => {
                let opcode = body[5 + offset];
                movements.push((axis, delta, opcode));
            }
            Command::Terminate => break,
            _ => {}
        }
    }

    for expected in [
        (0, 1, 14),
        (0, 2, 28),
        (0, 127, 28),
        (0, 128, 20),
        (0, 32_767, 20),
        (0, 32_768, 21),
        (1, 1, 16),
        (1, 2, 29),
        (1, 127, 29),
        (1, 128, 22),
        (1, 32_767, 22),
        (1, 32_768, 23),
        (1, -1, 17),
        (1, -2, 29),
        (1, -127, 29),
        (1, -128, 29),
        (1, -129, 22),
        (2, 1, 18),
        (2, 2, 30),
        (2, 127, 30),
        (2, 128, 24),
        (2, 32_767, 24),
        (2, 32_768, 25),
        (2, -1, 19),
        (2, -2, 30),
        (2, -128, 30),
        (2, -129, 24),
        (2, -32_768, 24),
        (2, -32_769, 25),
        (0, 3, 28),
        (1, 387, 22),
        (2, 65_793, 25),
    ] {
        assert!(
            movements.contains(&expected),
            "missing movement {expected:?}"
        );
    }
}

#[test]
fn bdx_brotli_output_matches_go_default_compressor() {
    use std::io::Write;

    let body = b"BDX\0\0\x01minecraft:stone\0\x01[]\0\x05\0\0\0\x01XE";
    let mut compressed = Vec::new();
    {
        let mut writer = brotli::CompressorWriter::new(&mut compressed, 4096, 6, 22);
        writer.write_all(body).unwrap();
    }

    assert_eq!(
        compressed,
        hex::decode("0b10804244580000016d696e6563726166743a73746f6e6500015b5d000500000001584503")
            .unwrap()
    );
}

#[test]
fn bdx_preserves_command_mode_from_raw_nbt_in_decode_and_direct_import() {
    use rust_water_structure::world::cache::CachedWorld;
    let mut wire = b"BDX\0fixture\0\x01chain_command_block\0\x01[]\0".to_vec();
    wire.extend_from_slice(&[41, 0, 0, 0, 1, 0, 1]);
    // 独立小端NBT夹具，模式字段属于原始负载，不能当作临时解析字段丢弃。
    wire.extend_from_slice(
        b"\x0a\0\0\x08\x02\0id\x0c\0CommandBlock\x03\x0d\0LPCommandMode\x02\0\0\0\0",
    );
    wire.extend_from_slice(b"XE");
    let bytes = bdx_container(&wire);
    let data = binary::decode(FormatId::BDX, &bytes).unwrap();
    assert_eq!(
        data.blocks[&Pos(0, 0, 0)].nbt.as_ref().unwrap()["LPCommandMode"],
        rust_water_structure::nbt::Nbt::Int(2)
    );

    let directory = tempfile::tempdir().unwrap();
    let world = CachedWorld::open(directory.path()).unwrap();
    binary::import_bdx_to_world(&bytes, &world, Pos(0, -64, 0), None).unwrap();
    world.close().unwrap();
    let reopened = CachedWorld::open(directory.path()).unwrap();
    assert_eq!(
        reopened.load_block_nbt(Pos(0, -64, 0)).unwrap().unwrap()["LPCommandMode"],
        rust_water_structure::nbt::Nbt::Int(2)
    );
    reopened.close().unwrap();
}

#[test]
fn bdx_all_declared_movement_opcodes_have_correct_payload_width() {
    use rust_water_structure::bdump::{Command, CommandReader};
    let cases: &[(u8, &[u8], usize, i32)] = &[
        (6, &[255, 254], 2, -2),
        (8, &[], 2, 1),
        (12, &[255, 255, 255, 254], 2, -2),
        (14, &[], 0, 1),
        (15, &[], 0, -1),
        (16, &[], 1, 1),
        (17, &[], 1, -1),
        (18, &[], 2, 1),
        (19, &[], 2, -1),
        (20, &[255, 254], 0, -2),
        (21, &[255, 255, 255, 254], 0, -2),
        (22, &[255, 254], 1, -2),
        (23, &[255, 255, 255, 254], 1, -2),
        (24, &[255, 254], 2, -2),
        (25, &[255, 255, 255, 254], 2, -2),
        (28, &[254], 0, -2),
        (29, &[254], 1, -2),
        (30, &[254], 2, -2),
    ];
    for (opcode, payload, axis, delta) in cases {
        let mut wire = vec![*opcode];
        wire.extend_from_slice(payload);
        wire.push(88);
        let mut reader = CommandReader::new(&wire);
        assert_eq!(
            reader.read().unwrap(),
            Some(Command::Move {
                axis: *axis,
                delta: *delta
            })
        );
        assert_eq!(reader.read().unwrap(), Some(Command::Terminate));
    }
}

#[test]
fn bdx_all_placement_opcodes_decode_their_nbt_payloads() {
    use rust_water_structure::bdump::{BlockReference, Command, CommandReader};
    let command_nbt = [
        0, 0, 0, 1, b's', b'a', b'y', b' ', b'h', b'i', 0, b'n', 0, b'o', 0, 0, 0, 0, 9, 1, 0, 1, 0,
    ];
    let chest_nbt = [1, b'd', b'i', b'r', b't', 0, 4, 0, 2, 3];
    let raw_nbt = [
        10, 0, 0, 8, 2, 0, b'i', b'd', 5, 0, b'C', b'h', b'e', b's', b't', 0,
    ];
    for opcode in [5, 7, 13, 27, 32, 33, 34, 35, 36, 37, 38, 40, 41] {
        let mut wire = vec![opcode];
        match opcode {
            5 | 7 | 27 | 40 | 41 => wire.extend_from_slice(&[0, 2, 0, 3]),
            13 => wire.extend_from_slice(b"\0\x02[]\0"),
            32 | 34 | 37 => wire.extend_from_slice(&[0, 2]),
            33 | 35 | 38 => wire.extend_from_slice(&[0, 0, 0, 2]),
            36 => wire.extend_from_slice(&[0, 3]),
            _ => unreachable!(),
        }
        match opcode {
            27 | 34 | 35 | 36 => wire.extend_from_slice(&command_nbt),
            37 | 38 | 40 => wire.extend_from_slice(&chest_nbt),
            41 => {
                wire.extend_from_slice(&[0, 3]);
                wire.extend_from_slice(&raw_nbt);
            }
            _ => {}
        }
        wire.push(88);
        let mut reader = CommandReader::new(&wire);
        let Some(Command::Place { block, nbt }) = reader.read().unwrap() else {
            panic!("opcode {opcode}");
        };
        match opcode {
            5 | 41 => assert_eq!(block, BlockReference::States { name: 2, states: 3 }),
            7 | 27 | 40 => assert_eq!(block, BlockReference::Legacy { name: 2, data: 3 }),
            13 => assert_eq!(
                block,
                BlockReference::InlineStates {
                    name: 2,
                    states: "[]".into()
                }
            ),
            36 => assert_eq!(block, BlockReference::CommandBlock { data: 3 }),
            _ => assert_eq!(block, BlockReference::Runtime(2)),
        }
        match opcode {
            27 | 34 | 35 | 36 => {
                let nbt = nbt.unwrap();
                assert_eq!(nbt["Command"].as_str(), Some("say hi"));
                assert_eq!(nbt["TickDelay"].as_i64(), Some(9));
                assert_eq!(nbt["auto"].as_i64(), Some(1));
                assert_eq!(nbt["conditionalMode"].as_i64(), Some(1));
            }
            37 | 38 | 40 => {
                let nbt = nbt.unwrap();
                let item = nbt["Items"].as_list().unwrap()[0].as_compound().unwrap();
                assert_eq!(item["Name"].as_str(), Some("dirt"));
                assert_eq!(item["Damage"].as_i64(), Some(2));
                assert_eq!(item["Count"].as_i64(), Some(4));
                assert_eq!(item["Slot"].as_i64(), Some(3));
            }
            41 => assert_eq!(nbt.unwrap()["id"].as_str(), Some("Chest")),
            _ => assert!(nbt.is_none()),
        }
        assert_eq!(reader.read().unwrap(), Some(Command::Terminate));
    }
}

#[test]
fn bdx_auxiliary_commands_and_signature_bounds() {
    use rust_water_structure::bdump::{Command, CommandReader, verify_stream};
    let bytes = [31, 117, 39, 0, 0, 0, 2, 65, 66, 9, 88];
    let mut reader = CommandReader::new(&bytes);
    assert_eq!(reader.read().unwrap(), Some(Command::RuntimePool(117)));
    assert_eq!(reader.read().unwrap(), Some(Command::Debug(b"AB".to_vec())));
    assert_eq!(reader.read().unwrap(), Some(Command::NoOperation));
    assert_eq!(reader.read().unwrap(), Some(Command::Terminate));
    assert!(!verify_stream(b"BDX\0\0XE").unwrap().signed);
    assert!(verify_stream(&[90]).is_err());
    assert!(verify_stream(&[88, 1, 255, 90]).is_err());
    let legacy = verify_stream(&[65, 88, 1, 2, 2, 90]).unwrap();
    assert!(legacy.signed && legacy.legacy && !legacy.corrupted);
}

#[test]
fn bdx_reads_117_pool_and_rejects_unknown_pool() {
    // 117池中 134 是 air，0 是 acacia_button:0，独立于当前 runtime ID 池。
    let wire = b"BDX\0\0\x1f\x75\x20\x00\x00XE";
    let data = binary::decode(FormatId::BDX, &bdx_container(wire)).unwrap();
    assert!(data.blocks[&Pos(0, 0, 0)].state.name.contains("button"));
    let invalid = b"BDX\0\0\x1f\x01\x20\x00\x00XE";
    assert!(binary::decode(FormatId::BDX, &bdx_container(invalid)).is_err());
}

#[test]
fn ibimport_xor_segments_and_base64_command() {
    let mut bytes = b"IBImport ".to_vec();
    let segments: &[&[u8]] = &[
        b"setblock ~-2 ~64 ~9 command_block []\n",
        br#"[{"posX":"~-2","posY":"~64","posZ":"~9","CommandMessage":"c2F5IGhlbGxv","Commandtitle":"fixture","mode":0,"isTime":7,"Conditional":"1","isRedstone":true}]"#,
    ];
    for segment in segments {
        let mut length = segment.len();
        while length >= 128 {
            bytes.push(length as u8 | 128);
            length >>= 7;
        }
        bytes.push(length as u8);
        bytes.push(0x35);
        bytes.extend(segment.iter().map(|byte| byte ^ 0x35));
    }
    let data = binary::decode(FormatId::IBImport, &bytes).unwrap();
    let nbt = data.blocks[&Pos(0, 0, 0)].nbt.as_ref().unwrap();
    assert_eq!(nbt["Command"].as_str(), Some("say hello"));
    assert_eq!(nbt["TickDelay"].as_i64(), Some(7));
    assert_eq!(nbt["auto"].as_i64(), Some(0));
    assert_eq!(nbt["conditionalMode"].as_i64(), Some(1));
}

#[test]
fn ibimport_accepts_go_boolish_values_and_plain_command_fallback() {
    let mut bytes = b"IBImport ".to_vec();
    let script = (0..8)
        .map(|x| format!("setblock {x} 0 0 command_block []\n"))
        .collect::<String>();
    let commands = serde_json::json!([
        {"posX":"~0","posY":"~0","posZ":"~0","CommandMessage":"say hello","Conditional":0},
        {"posX":"~1","posY":"~0","posZ":"~0","CommandMessage":"c2F5IGhp","Conditional":1},
        {"posX":"~2","posY":"~0","posZ":"~0","CommandMessage":"say false","Conditional":false},
        {"posX":"~3","posY":"~0","posZ":"~0","CommandMessage":"say true","Conditional":true},
        {"posX":"~4","posY":"~0","posZ":"~0","CommandMessage":"say zero","Conditional":"0"},
        {"posX":"~5","posY":"~0","posZ":"~0","CommandMessage":"say one","Conditional":"1"},
        {"posX":"~6","posY":"~0","posZ":"~0","CommandMessage":"say string false","Conditional":"false"},
        {"posX":"~7","posY":"~0","posZ":"~0","CommandMessage":"say string true","Conditional":"true"}
    ]);
    for segment in [script.into_bytes(), serde_json::to_vec(&commands).unwrap()] {
        let mut length = segment.len();
        while length >= 128 {
            bytes.push(length as u8 | 128);
            length >>= 7;
        }
        bytes.push(length as u8);
        bytes.push(0x35);
        bytes.extend(segment.iter().map(|byte| byte ^ 0x35));
    }

    let data = binary::decode(FormatId::IBImport, &bytes).unwrap();
    for (x, expected) in [false, true, false, true, false, true, false, true]
        .into_iter()
        .enumerate()
    {
        let block = &data.blocks[&Pos(x as i32, 0, 0)];
        let nbt = block.nbt.as_ref().unwrap();
        assert_eq!(nbt["conditionalMode"].as_i64(), Some(i64::from(expected)));
    }
    assert_eq!(
        data.blocks[&Pos(0, 0, 0)].nbt.as_ref().unwrap()["Command"].as_str(),
        Some("say hello")
    );
    assert_eq!(
        data.blocks[&Pos(1, 0, 0)].nbt.as_ref().unwrap()["Command"].as_str(),
        Some("say hi")
    );
}

#[test]
fn tibi_reads_external_raw_deflate_md5_xor_fixture() {
    // 通过 Python 标准库 hashlib/zlib 按 Go 测试定义独立生成。
    let bytes = hex::decode("6360646266616563e7e0e4e2e6e1e5db7b582b2e29bd6cb6d7f1856e6a553c9b2f2d5331e6b214fa61b1ecb8910c00").unwrap();
    let data = binary::decode(FormatId::TIBI, &bytes).unwrap();
    assert_eq!(data.size, Size(1, 1, 1));
    assert_eq!(data.blocks[&Pos(0, 0, 0)].state.name, "minecraft:stone");
}

fn independent_bcf(version: u8) -> Vec<u8> {
    // 对照 Go writeMinimalBCF；各段偏移为手算的固定布局。
    let mut bytes = b"BCF".to_vec();
    bytes.push(version);
    for _ in 0..3 {
        bytes.extend_from_slice(&1u16.to_le_bytes());
    }
    bytes.push(144);
    bytes.extend_from_slice(&1u64.to_le_bytes());
    for offset in [93u64, 109, 121, 144, 148] {
        bytes.extend_from_slice(&offset.to_le_bytes());
    }
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(&[0; 6]);
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&[0; 16]);
    bytes.extend_from_slice(&1u64.to_le_bytes());
    bytes.extend_from_slice(&59u64.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&[0; 8]);
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&15u16.to_le_bytes());
    bytes.extend_from_slice(b"minecraft:stone");
    bytes.extend_from_slice(&[0; 8]);
    bytes
}

#[test]
fn bcf_multiple_version_headers_share_the_original_go_layout() {
    for version in [1, 2, 3] {
        let data = binary::decode(FormatId::BCF, &independent_bcf(version)).unwrap();
        assert_eq!(data.size, Size(1, 1, 1));
        assert_eq!(data.non_air_count(), 1);
        assert_eq!(data.metadata["version"].as_i64(), Some(i64::from(version)));
    }
}

#[test]
fn bcf_encoder_uses_subchunk_origins_for_large_world_coordinates() {
    use rust_water_structure::{Block, BlockState, StructureData};
    let mut data = StructureData::new(FormatId::BCF);
    data.size = Size(2, 1, 2);
    data.origin = Pos(-40_000, 64, 40_000);
    data.blocks
        .insert(Pos(0, 0, 0), Block::new(BlockState::new("stone")));
    data.blocks
        .insert(Pos(1, 0, 1), Block::new(BlockState::new("glass")));
    let bytes = binary::encode(FormatId::BCF, &data).unwrap();
    let reloaded = binary::decode(FormatId::BCF, &bytes).unwrap();
    assert_eq!(reloaded.origin, data.origin);
    assert_eq!(reloaded.size, data.size);
    assert_eq!(reloaded.non_air_count(), 2);
    let oracle = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("work/oracle.exe");
    if oracle.exists() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large-coordinate.bcf");
        std::fs::write(&path, bytes).unwrap();
        let output = std::process::Command::new(oracle)
            .arg(path)
            .arg("BCF")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["size"], serde_json::json!([2, 1, 2]));
        assert_eq!(result["non_air"], 2);
    }
}

#[test]
fn real_binary_fixtures_match_go_reader_sizes_and_counts() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../WaterStructure/test/测试文件");
    if !directory.exists() {
        return;
    }
    let cases = [
        (FormatId::KBDX, "初见黑市.kbdx", Size(65, 24, 33), 5222, 206),
        (
            FormatId::IBImport,
            "二级雪球菜单(含仓库-公益-by虹芡).ibi",
            Size(14, 30, 10),
            874,
            288,
        ),
        (FormatId::BDX, "七天签到(1).bdx", Size(41, 25, 2), 73, 70),
    ];
    for (format, filename, size, count, nbt_count) in cases {
        let bytes = std::fs::read(directory.join(filename)).unwrap();
        let data = binary::decode(format, &bytes).unwrap();
        assert_eq!(data.size, size, "{filename}");
        assert_eq!(data.non_air_count(), count, "{filename}");
        assert_eq!(
            data.blocks.values().filter(|b| b.nbt.is_some()).count(),
            nbt_count,
            "{filename}"
        );
    }
}

#[test]
fn original_go_readers_accept_every_binary_encoder() {
    use rust_water_structure::{Block, BlockState, StructureData};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let oracle = root.join("work/oracle.exe");
    if !oracle.exists() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let mut data = StructureData::new(FormatId::MCFunction);
    data.blocks
        .insert(Pos(0, 0, 0), Block::new(BlockState::new("stone")));
    data.blocks
        .insert(Pos(2, 1, 3), Block::new(BlockState::new("glass")));
    data.blocks.insert(
        Pos(1, 0, 0),
        Block::new(rust_water_structure::blocks::legacy("oak_stairs", 2)),
    );
    data.blocks.insert(
        Pos(0, 1, 1),
        Block::new(rust_water_structure::blocks::legacy("wool", 14)),
    );
    data.blocks.insert(
        Pos(1, 1, 2),
        Block::new(rust_water_structure::blocks::legacy("chest", 3)),
    );
    data.size = Size(3, 2, 4);
    let mut expected_hash = None;
    for format in [
        FormatId::BDX,
        FormatId::MCFunction,
        FormatId::KBDX,
        FormatId::IBImport,
        FormatId::BDS,
        FormatId::BCF,
        FormatId::TIBI,
        FormatId::NexusNP,
    ] {
        let path = directory.path().join(format!("test.{}", format.name()));
        std::fs::write(&path, binary::encode(format, &data).unwrap()).unwrap();
        let output = std::process::Command::new(&oracle)
            .arg(&path)
            .arg(format.name())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{format:?}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            value["size"],
            serde_json::json!([3, 2, 4]),
            "{format:?}: {value}"
        );
        assert_eq!(value["non_air"], 5, "{format:?}: {value}");
        if let Some(expected) = &expected_hash {
            assert_eq!(&value["blocks_sha256"], expected, "{format:?}: {value}");
        } else {
            expected_hash = Some(value["blocks_sha256"].clone());
        }
    }
}

#[test]
fn original_go_reader_compares_real_sample_exports() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let oracle = root.join("work/oracle.exe");
    let samples = root.join("../WaterStructure/test/测试文件");
    if !oracle.exists() || !samples.exists() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    for (format, name) in [
        (FormatId::BDX, "七天签到(1).bdx"),
        (FormatId::IBImport, "二级雪球菜单(含仓库-公益-by虹芡).ibi"),
        (FormatId::KBDX, "初见黑市.kbdx"),
    ] {
        let input = samples.join(name);
        let bytes = std::fs::read(&input).unwrap();
        let data = binary::decode(format, &bytes).unwrap();
        let output = directory.path().join(name);
        let exported = binary::encode(format, &data).unwrap();
        let reloaded = binary::decode(format, &exported).unwrap();
        let differences: Vec<_> = data
            .blocks
            .iter()
            .filter_map(|(pos, before)| {
                let after = reloaded.blocks.get(pos)?;
                (before.state != after.state).then(|| {
                    format!(
                        "{pos:?}: {} -> {}",
                        before.state.string(),
                        after.state.string()
                    )
                })
            })
            .take(10)
            .collect();
        assert!(differences.is_empty(), "{format:?}: {differences:?}");
        std::fs::write(&output, exported).unwrap();
        let inspect = |path: &std::path::Path| -> serde_json::Value {
            let output = std::process::Command::new(&oracle)
                .arg(path)
                .arg(format.name())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{format:?}: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            serde_json::from_slice(&output.stdout).unwrap()
        };
        let expected = inspect(&input);
        let actual = inspect(&output);
        for field in [
            "size",
            "non_air",
            "nbt_count",
            "blocks_sha256",
            "nbt_sha256",
        ] {
            assert_eq!(
                actual[field], expected[field],
                "{format:?} {field}: {actual}"
            );
        }
    }
}

#[test]
#[ignore = "百万方块真实样例专项验证，按需显式运行"]
fn large_real_mcfunction_matches_original_go_statistics() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../WaterStructure/test/测试文件/螺旋跑酷_optimized.mcfunction");
    if !source.exists() {
        return;
    }
    let data = binary::decode(FormatId::MCFunction, &std::fs::read(source).unwrap()).unwrap();
    assert_eq!(data.size, Size(105, 203, 105));
    assert_eq!(data.non_air_count(), 1_326_819);
}

#[test]
fn additional_real_bdx_samples_decode() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../WaterStructure/test/测试文件");
    if !directory.exists() {
        return;
    }
    for name in [
        "A空岛指令.bdx",
        "二维地皮(1) (2) (1).bdx",
        "nor实验室地图.bdx",
    ] {
        let bytes = std::fs::read(directory.join(name)).unwrap();
        let data = binary::decode(FormatId::BDX, &bytes).unwrap();
        assert!(data.non_air_count() > 0, "{name}");
        if name.starts_with("二维地皮") {
            assert_eq!(data.size, Size(76, 19, 59));
            assert_eq!(data.non_air_count(), 21484);
            assert_eq!(data.blocks.values().filter(|b| b.nbt.is_some()).count(), 92);
        }
    }
}
