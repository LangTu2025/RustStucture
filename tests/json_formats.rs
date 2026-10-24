use rust_water_structure::formats::json::{decode, encode};
use rust_water_structure::{Block, BlockState, FormatId, Pos, Size, StructureData};
use serde_json::{Value, json};

fn one_block(format: FormatId, input: Value, origin: Pos) {
    let structure = decode(format, &serde_json::to_vec(&input).unwrap()).unwrap();
    assert_eq!(structure.origin, origin, "{format:?}");
    assert_eq!(structure.size, Size(1, 1, 1), "{format:?}");
    assert_eq!(structure.blocks.len(), 1, "{format:?}");
    assert_eq!(
        structure.blocks[&Pos(0, 0, 0)].state.name,
        "minecraft:stone"
    );
}

#[test]
fn manual_sparse_formats_preserve_world_origin() {
    for format in [FormatId::MianYangV1, FormatId::MianYangV2] {
        one_block(
            format,
            json!({"namespaces":["minecraft:stone"],"chunkedBlocks":[{"startX":-16,"startZ":32,"blocks":[[0,0,1,70,2]]}]}),
            Pos(-15, 70, 34),
        );
    }
    for format in [FormatId::FuHongV1, FormatId::RunAway] {
        one_block(
            format,
            json!([{"name":"minecraft:stone","aux":0,"x":-15,"y":70,"z":34}]),
            Pos(-15, 70, 34),
        );
    }
    one_block(
        FormatId::TimeBuilderV1,
        json!({"version":"TimeBuilder","block":[{"name":"minecraft:stone","aux":0,"pos":[[-15,70,34]]}]}),
        Pos(-15, 70, 34),
    );
    one_block(
        FormatId::FuHongV2,
        json!({"FuHongBuild_FinalFormat":[{"block":[{"n":"minecraft:stone","a":0,"x":[-15],"y":[70],"z":[34]}]}]}),
        Pos(-15, 70, 34),
    );
    for format in [FormatId::FuHongV3, FormatId::FuHongV4] {
        let coordinates = if format == FormatId::FuHongV3 {
            json!([[0, 0, [1], [70], [2]]])
        } else {
            json!([[0, 0, [-15], [70], [34]]])
        };
        one_block(
            format,
            json!({"BlocksList":["minecraft:stone"],"FuHongBuild":[{"startX":-16,"startZ":32,"block":coordinates}]}),
            Pos(-15, 70, 34),
        );
    }
    let point = json!({"Name":"stone","X":-15,"Y":70,"Z":34}).to_string();
    let chunk = json!({"totalPoints":1,"0":point}).to_string();
    one_block(
        FormatId::QingXuV1,
        json!({"totalBlocks":1,"0":chunk}),
        Pos(-15, 70, 34),
    );
}

#[test]
fn gangban_variants_follow_declared_and_sparse_coordinate_rules() {
    for format in [FormatId::GangBanV1, FormatId::GangBanV2] {
        let bytes = serde_json::to_vec(&json!([{"id":0,"aux":0,"p":[-15,70,34]},{"start":[-16,69,32],"end":[-13,72,36]},{"list":["stone"]}])).unwrap();
        let structure = decode(format, &bytes).unwrap();
        assert_eq!(structure.size, Size(4, 4, 5));
        assert_eq!(structure.origin, Pos(-16, 69, 32));
        assert_eq!(
            structure.blocks[&Pos(1, 1, 2)].state.name,
            "minecraft:stone"
        );
    }
    let chunk = json!({"grids":{"x":-16,"z":32},"data":[[0,0,1,70,2]]});
    let bytes = serde_json::to_vec(
        &json!([{"x":-16,"y":69,"z":32,"xcha":4,"ycha":4,"zcha":5},"[0]stone[0]",chunk]),
    )
    .unwrap();
    let structure = decode(FormatId::GangBanV3, &bytes).unwrap();
    assert_eq!(structure.size, Size(4, 4, 5));
    assert_eq!(
        structure.blocks[&Pos(1, 1, 2)].state.name,
        "minecraft:stone"
    );
    one_block(
        FormatId::GangBanV4,
        json!([{}, ["stone"], chunk]),
        Pos(-15, 70, 34),
    );
    one_block(
        FormatId::GangBanV5,
        json!([0,-15,70,34,0,0,{"ep":[0,0,0]},["stone"]]),
        Pos(-15, 70, 34),
    );
    one_block(
        FormatId::GangBanV6,
        json!([[-15, 70, 34, 0, 0], ["stone"]]),
        Pos(-15, 70, 34),
    );
}

#[test]
fn cov_uses_x_z_y_flattening_and_declared_air_boundary() {
    let input = json!({"size":[2,2,2],"structure":{"palette":[{"name":"air"},{"name":"stone"}],"block_indices":[0,1,-1,0,0,0,1,0]}});
    let structure = decode(FormatId::CovStructure, &serde_json::to_vec(&input).unwrap()).unwrap();
    assert_eq!(structure.size, Size(2, 2, 2));
    assert_eq!(
        structure.blocks[&Pos(1, 0, 0)].state.name,
        "minecraft:stone"
    );
    assert_eq!(
        structure.blocks[&Pos(0, 1, 1)].state.name,
        "minecraft:stone"
    );
}

#[test]
fn encoders_emit_original_json_wire_shapes() {
    let mut structure = StructureData::new(FormatId::RunAway);
    structure.size = Size(1, 1, 1);
    structure.origin = Pos(-15, 70, 34);
    structure.insert(Pos(0, 0, 0), Block::new(BlockState::new("stone")));
    for format in [
        FormatId::MianYangV1,
        FormatId::MianYangV2,
        FormatId::GangBanV1,
        FormatId::GangBanV2,
        FormatId::GangBanV3,
        FormatId::GangBanV4,
        FormatId::GangBanV5,
        FormatId::GangBanV6,
        FormatId::FuHongV1,
        FormatId::FuHongV2,
        FormatId::FuHongV3,
        FormatId::FuHongV4,
        FormatId::RunAway,
        FormatId::QingXuV1,
        FormatId::TimeBuilderV1,
        FormatId::CovStructure,
    ] {
        let bytes = encode(format, &structure).unwrap();
        let wire: Value = serde_json::from_slice(&bytes).unwrap();
        match format {
            FormatId::MianYangV1 | FormatId::MianYangV2 => {
                assert_eq!(wire["chunkedBlocks"][0]["blocks"][0][3], 70)
            }
            FormatId::GangBanV1 | FormatId::GangBanV2 => {
                assert_eq!(wire[0]["p"], json!([-15, 70, 34]))
            }
            FormatId::GangBanV3 | FormatId::GangBanV4 => assert!(wire[2]["data"].is_array()),
            FormatId::GangBanV5 => {
                assert!(wire[wire.as_array().unwrap().len() - 2]["ep"].is_array())
            }
            FormatId::GangBanV6 => assert_eq!(wire[0][0], -15),
            FormatId::FuHongV1 | FormatId::RunAway => {
                assert_eq!(wire[0]["name"], "minecraft:stone")
            }
            FormatId::FuHongV2 => assert!(wire["FuHongBuild_FinalFormat"][0]["block"].is_array()),
            FormatId::FuHongV3 | FormatId::FuHongV4 => {
                assert!(wire["FuHongBuild"][0]["block"].is_array())
            }
            FormatId::QingXuV1 => assert!(wire["0"].is_string()),
            FormatId::TimeBuilderV1 => assert_eq!(wire["version"], "TimeBuilder"),
            FormatId::CovStructure => assert_eq!(wire["structure"]["block_indices"][0], 0),
            _ => unreachable!(),
        }
    }
}

#[test]
fn real_nordic_samples_decode_all_json_variants() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../WaterStructure/test/测试文件");
    for (format, suffix) in [
        (FormatId::MianYangV1, "MianYang_V1.json"),
        (FormatId::MianYangV2, "MianYang_V2.json"),
        (FormatId::MianYangV3, "MianYang_V3.building"),
        (FormatId::GangBanV1, "GangBan_V1.json"),
        (FormatId::GangBanV2, "GangBan_V2.json"),
        (FormatId::GangBanV3, "GangBan_V3.json"),
        (FormatId::GangBanV4, "GangBan_V4.json"),
        (FormatId::GangBanV5, "GangBan_V5.json"),
        (FormatId::GangBanV6, "GangBan_V6.json"),
        (FormatId::GangBanV7, "GangBan_V7.reb"),
        (FormatId::FuHongV3, "FuHong_V3.json"),
        (FormatId::FuHongV4, "FuHong_V4.json"),
        (FormatId::FuHongV5, "FuHong_V5.fhbuild"),
        (FormatId::RunAway, "RunAway.json"),
        (FormatId::QingXuV1, "QingXu_V1.json"),
        (FormatId::TimeBuilderV1, "TimeBuilder_V1.json"),
    ] {
        let path = root.join(format!("北欧之家_{suffix}"));
        let data = decode(format, &std::fs::read(path).unwrap())
            .unwrap_or_else(|error| panic!("{format:?}: {error}"));
        let expected_size = if format == FormatId::GangBanV3 {
            Size(24, 28, 27)
        } else {
            Size(24, 27, 28)
        };
        assert_eq!(data.size, expected_size, "{format:?}");
        assert!(data.blocks.len() > 2000, "{format:?}");
    }
}

#[test]
fn compressed_formats_and_buildingx_roundtrip() {
    let mut input = StructureData::new(FormatId::RunAway);
    input.size = Size(3, 2, 4);
    input.origin = Pos(-15, 70, 34);
    input.insert(Pos(0, 0, 0), Block::new(BlockState::new("stone")));
    input.insert(Pos(2, 1, 3), Block::new(BlockState::new("dirt")));
    for format in [
        FormatId::MianYangV3,
        FormatId::MianYangV4,
        FormatId::GangBanV7,
        FormatId::FuHongV5,
    ] {
        let bytes = encode(format, &input).unwrap();
        let decoded = decode(format, &bytes).unwrap();
        assert_eq!(decoded.size, input.size, "{format:?}");
        assert_eq!(decoded.blocks[&Pos(2, 1, 3)].state.name, "minecraft:dirt");
    }
}

#[test]
fn real_buildingx_keeps_header_dimensions_and_world_origin() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../WaterStructure/test/测试文件/我的世界_475.3万方块.buildingX");
    let source = decode(FormatId::MianYangV4, &std::fs::read(path).unwrap()).unwrap();
    assert_eq!(source.origin, Pos(362, 16, -181));
    assert_eq!(source.size, Size(124, 81, 124));
    // 原 Go 的实际区块结果与 reported_count 397032 不同。
    assert_eq!(source.non_air_count(), 341459);
}

#[test]
fn legacy_command_and_container_payloads_keep_typed_nbt() {
    use rust_water_structure::nbt::Nbt;
    let input = json!([
        [10, 64, -3, 5, 1, {"cmd":"say 中文","name":"测试","delay":4,"auto":true,"condition":true}],
        [1, 0, 0, 0, 0, [{"ns":"minecraft:diamond","aux":0,"num":2,"slot":7}]],
        ["minecraft:chest"]
    ]);
    let data = decode(FormatId::GangBanV6, &serde_json::to_vec(&input).unwrap()).unwrap();
    let command = data.blocks[&Pos(0, 0, 0)].nbt.as_ref().unwrap();
    assert_eq!(command["Command"], Nbt::String("say 中文".into()));
    assert_eq!(command["TickDelay"], Nbt::Int(4));
    assert_eq!(command["LPCommandMode"], Nbt::Int(1));
    assert_eq!(command["conditionalMode"], Nbt::Byte(1));
    let container = data.blocks[&Pos(1, 0, 0)].nbt.as_ref().unwrap();
    assert_eq!(container["id"], Nbt::String("Chest".into()));
    let item = container["Items"].as_list().unwrap()[0]
        .as_compound()
        .unwrap();
    assert_eq!(item["Slot"], Nbt::Byte(7));
    assert_eq!(item["Count"], Nbt::Byte(2));
    assert_eq!(item["Damage"], Nbt::Short(0));
}

#[test]
fn gangban_stream_export_preserves_empty_container_block_entities() {
    let bytes = serde_json::to_vec(&json!([[0, 0, 0, 0, 0, []], ["minecraft:chest"]])).unwrap();
    let source = decode(FormatId::GangBanV6, &bytes).unwrap();
    for format in [
        FormatId::GangBanV5,
        FormatId::GangBanV6,
        FormatId::GangBanV7,
    ] {
        let encoded = encode(format, &source).unwrap();
        let decoded = decode(format, &encoded).unwrap();
        assert!(decoded.blocks[&Pos(0, 0, 0)].nbt.is_some(), "{format:?}");
    }
}

#[test]
fn duplicate_block_without_nbt_keeps_previous_payload_like_go() {
    let my = json!({"namespaces":["minecraft:stone","minecraft:dirt"],"chunkedBlocks":[{
    "startX":0,"startZ":0,"blocks":[
        [0,0,0,0,0,"{id:\"CommandBlock\",Command:\"say first\"}"],
        [1,0,0,0,0]
    ]}]});
    let gb = json!([
        {"id":0,"aux":0,"p":[0,0,0],"cmds":{"cmd":"say first"}},
        {"id":1,"aux":0,"p":[0,0,0]},
        {"start":[0,0,0],"end":[0,0,0]},
        {"list":["minecraft:stone","minecraft:dirt"]}
    ]);
    for (format, input) in [(FormatId::MianYangV1, my), (FormatId::GangBanV1, gb)] {
        let decoded = decode(format, &serde_json::to_vec(&input).unwrap()).unwrap();
        let block = &decoded.blocks[&Pos(0, 0, 0)];
        assert_eq!(block.state.name, "minecraft:dirt");
        assert!(block.nbt.is_some(), "{format:?}");
    }
}

#[test]
fn container_ids_follow_original_go_supported_block_names() {
    let input = json!([
        [0, 0, 0, 0, 0, []],
        [1, 0, 0, 1, 0, []],
        ["minecraft:crafter", "minecraft:brewing_stand"]
    ]);
    let decoded = decode(FormatId::GangBanV6, &serde_json::to_vec(&input).unwrap()).unwrap();
    assert_eq!(
        decoded.blocks[&Pos(0, 0, 0)].nbt.as_ref().unwrap()["id"].as_str(),
        Some("Crafter")
    );
    assert!(decoded.blocks[&Pos(1, 0, 0)].nbt.is_none());
}

#[test]
fn fuhong_group_extras_and_v2_snbt_are_compatible() {
    use rust_water_structure::nbt::Nbt;
    let input = json!({"BlocksList":["minecraft:command_block","minecraft:chest","minecraft:wall_sign"],
    "FuHongBuild":[{"block":[
        [0,0,[10],[64],[-3],[["say 中文",4,true,"测试"]]],
        [1,0,[11],[64],[-3],[[["diamond",0,2,7]]]],
        [2,0,[12],[64],[-3],[["第一行","第二行"]]]
    ]}]});
    let data = decode(FormatId::FuHongV4, &serde_json::to_vec(&input).unwrap()).unwrap();
    assert_eq!(
        data.blocks[&Pos(0, 0, 0)].nbt.as_ref().unwrap()["auto"],
        Nbt::Byte(1)
    );
    let sign = data.blocks[&Pos(2, 0, 0)].nbt.as_ref().unwrap()["FrontText"]
        .as_compound()
        .unwrap();
    assert_eq!(sign["Text"], Nbt::String("第一行\n第二行".into()));
    let input = json!({"FuHongBuild_FinalFormat":[{"block":[{"n":"minecraft:chest","x":[1],"y":[2],"z":[3],"d":[{"e":"{id:\"Chest\",CustomName:\"箱子\",Items:[]}"}]}]}]});
    let data = decode(FormatId::FuHongV2, &serde_json::to_vec(&input).unwrap()).unwrap();
    assert_eq!(
        data.blocks[&Pos(0, 0, 0)].nbt.as_ref().unwrap()["CustomName"],
        Nbt::String("箱子".into())
    );
}

#[test]
fn original_go_reader_accepts_every_rust_json_encoder() {
    let oracle = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("work/oracle.exe");
    if !oracle.exists() {
        return;
    }
    let mut input = StructureData::new(FormatId::RunAway);
    input.size = Size(3, 2, 4);
    input.insert(Pos(0, 0, 0), Block::new(BlockState::new("stone")));
    input.insert(Pos(2, 1, 3), Block::new(BlockState::new("dirt")));
    let temp = tempfile::tempdir().unwrap();
    for format in [
        FormatId::MianYangV1,
        FormatId::MianYangV2,
        FormatId::MianYangV3,
        FormatId::MianYangV4,
        FormatId::GangBanV1,
        FormatId::GangBanV2,
        FormatId::GangBanV3,
        FormatId::GangBanV4,
        FormatId::GangBanV5,
        FormatId::GangBanV6,
        FormatId::GangBanV7,
        FormatId::FuHongV1,
        FormatId::FuHongV2,
        FormatId::FuHongV3,
        FormatId::FuHongV4,
        FormatId::FuHongV5,
        FormatId::RunAway,
        FormatId::QingXuV1,
        FormatId::TimeBuilderV1,
        FormatId::CovStructure,
    ] {
        let path = temp.path().join(format!("{}.bin", format.name()));
        std::fs::write(&path, encode(format, &input).unwrap()).unwrap();
        let result = std::process::Command::new(&oracle)
            .arg(&path)
            .arg(format.name())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{format:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let summary: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(summary["size"], json!([3, 2, 4]), "{format:?}: {summary}");
        assert_eq!(summary["non_air"], 2, "{format:?}: {summary}");
    }
}

#[test]
fn original_go_reader_keeps_command_nbt_from_rust_writers() {
    use rust_water_structure::nbt::{Compound, Nbt};
    let oracle = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("work/oracle.exe");
    if !oracle.exists() {
        return;
    }
    let mut input = StructureData::new(FormatId::RunAway);
    input.size = Size(1, 1, 1);
    let mut block = Block::new(rust_water_structure::blocks::legacy(
        "minecraft:command_block",
        0,
    ));
    block.nbt = Some(Compound::from([
        ("id".into(), Nbt::String("CommandBlock".into())),
        ("Command".into(), Nbt::String("say 中文 + % 测试".into())),
        ("CustomName".into(), Nbt::String("测试方块".into())),
        ("TickDelay".into(), Nbt::Int(7)),
        ("auto".into(), Nbt::Byte(1)),
        ("conditionalMode".into(), Nbt::Byte(1)),
    ]));
    input.insert(Pos(0, 0, 0), block);
    let temp = tempfile::tempdir().unwrap();
    for format in [
        FormatId::MianYangV1,
        FormatId::MianYangV2,
        FormatId::MianYangV3,
        FormatId::MianYangV4,
        FormatId::GangBanV1,
        FormatId::GangBanV2,
        FormatId::GangBanV3,
        FormatId::GangBanV4,
        FormatId::GangBanV5,
        FormatId::GangBanV6,
        FormatId::GangBanV7,
        FormatId::FuHongV2,
        FormatId::FuHongV3,
        FormatId::FuHongV4,
        FormatId::FuHongV5,
    ] {
        let bytes = encode(format, &input).unwrap();
        let decoded = decode(format, &bytes).unwrap();
        let nbt = decoded.blocks[&Pos(0, 0, 0)].nbt.as_ref().unwrap();
        assert_eq!(
            nbt["Command"],
            Nbt::String("say 中文 + % 测试".into()),
            "{format:?}"
        );
        let path = temp.path().join(format!("{}.bin", format.name()));
        std::fs::write(&path, bytes).unwrap();
        let result = std::process::Command::new(&oracle)
            .arg(&path)
            .arg(format.name())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{format:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let summary: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(summary["non_air"], 1, "{format:?}: {summary}");
        assert_eq!(summary["nbt_count"], 1, "{format:?}: {summary}");
    }
}

#[test]
fn real_samples_reexport_match_original_go_block_and_nbt_summaries() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let oracle = root.join("work/oracle.exe");
    if !oracle.exists() {
        return;
    }
    let samples = root.join("../WaterStructure/test/测试文件");
    let temp = tempfile::tempdir().unwrap();
    for (format, suffix) in [
        (FormatId::MianYangV1, "MianYang_V1.json"),
        (FormatId::MianYangV2, "MianYang_V2.json"),
        (FormatId::MianYangV3, "MianYang_V3.building"),
        (FormatId::GangBanV1, "GangBan_V1.json"),
        (FormatId::GangBanV3, "GangBan_V3.json"),
        (FormatId::GangBanV4, "GangBan_V4.json"),
        (FormatId::GangBanV5, "GangBan_V5.json"),
        (FormatId::GangBanV6, "GangBan_V6.json"),
        (FormatId::GangBanV7, "GangBan_V7.reb"),
        (FormatId::FuHongV3, "FuHong_V3.json"),
        (FormatId::FuHongV4, "FuHong_V4.json"),
        (FormatId::FuHongV5, "FuHong_V5.fhbuild"),
        (FormatId::RunAway, "RunAway.json"),
        (FormatId::QingXuV1, "QingXu_V1.json"),
        (FormatId::TimeBuilderV1, "TimeBuilder_V1.json"),
    ] {
        let path = samples.join(format!("北欧之家_{suffix}"));
        let source = decode(format, &std::fs::read(&path).unwrap()).unwrap();
        let exported = temp.path().join(format!("{}.bin", format.name()));
        std::fs::write(&exported, encode(format, &source).unwrap()).unwrap();
        let summarize = |path: &std::path::Path| {
            let result = std::process::Command::new(&oracle)
                .arg(path)
                .arg(format.name())
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{format:?}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            serde_json::from_slice::<Value>(&result.stdout).unwrap()
        };
        let before = summarize(&path);
        let after = summarize(&exported);
        for field in [
            "size",
            "non_air",
            "blocks_sha256",
            "nbt_count",
            "nbt_sha256",
        ] {
            assert_eq!(after[field], before[field], "{format:?} {field}");
        }
    }
}

#[test]
fn legacy_palette_inverse_mapping_preserves_go_runtime_ids() {
    use std::io::Write;
    use std::process::Stdio;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let probe = root.join("work/json-probe.exe");
    if !probe.exists() {
        return;
    }
    let bytes =
        std::fs::read(root.join("../WaterStructure/test/测试文件/北欧之家_MianYang_V1.json"))
            .unwrap();
    let input: Value = serde_json::from_slice(&bytes).unwrap();
    let mut pairs = std::collections::BTreeSet::new();
    for chunk in input["chunkedBlocks"].as_array().unwrap() {
        for entry in chunk["blocks"].as_array().unwrap() {
            let name = input["namespaces"][entry[0].as_u64().unwrap() as usize]
                .as_str()
                .unwrap();
            pairs.insert((name.to_owned(), entry[1].as_u64().unwrap() as u16));
        }
    }
    let records: Vec<Value> = pairs
        .into_iter()
        .map(|(name, aux)| {
            let state = rust_water_structure::blocks::legacy(&name, aux);
            let (output_name, output_aux) = rust_water_structure::blocks::to_legacy(&state);
            json!({"Name":name,"Aux":aux,"OutputName":output_name,"OutputAux":output_aux})
        })
        .collect();
    let mut child = std::process::Command::new(probe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&records).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn qingxu_names_match_original_go_state_conversion() {
    use std::io::Write;
    use std::process::Stdio;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let probe = root.join("work/json-probe.exe");
    if !probe.exists() {
        return;
    }
    let bytes = std::fs::read(root.join("../WaterStructure/test/测试文件/北欧之家_QingXu_V1.json"))
        .unwrap();
    let input: Value = serde_json::from_slice(&bytes).unwrap();
    let mut names = std::collections::BTreeSet::new();
    for index in 0..input["totalBlocks"].as_u64().unwrap() {
        let chunk: Value =
            serde_json::from_str(input[index.to_string()].as_str().unwrap()).unwrap();
        for index in 0..chunk["totalPoints"].as_u64().unwrap() {
            let entry: Value =
                serde_json::from_str(chunk[index.to_string()].as_str().unwrap()).unwrap();
            let name = entry["Name"].as_str().unwrap();
            let name = match name.split_once('.') {
                Some((first, last)) => format!("{last}_{first}"),
                None => name.to_owned(),
            };
            names.insert(name);
        }
    }
    let records:Vec<Value>=names.into_iter().map(|name| {
        let state=rust_water_structure::blocks::java(&name);
        json!({"Name":name,"OutputName":state.string(),"String":true,"RustRuntime":rust_water_structure::blocks::runtime_id(&state)})
    }).collect();
    let mut child = std::process::Command::new(probe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&records).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
