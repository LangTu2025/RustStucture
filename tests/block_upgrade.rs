use rust_water_structure::blocks::upgrade;
use rust_water_structure::model::BlockState;
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct OracleCase {
    label: String,
    input: BlockState,
    version: i32,
    expected: BlockState,
}

#[test]
fn original_go_schema_corpus() {
    let cases: Vec<OracleCase> = serde_json::from_str(
        include_str!("fixtures/block_upgrade_oracle.json").trim_start_matches('\u{feff}'),
    )
    .unwrap();
    assert_eq!(cases.len(), 7383);
    for case in cases {
        assert_eq!(
            upgrade::upgrade(&case.input, case.version),
            case.expected,
            "{}",
            case.label
        );
    }
}

#[test]
fn rename_boundary_is_inclusive_and_input_is_immutable() {
    let version = (1 << 24) | (20 << 16) | (70 << 8) | 4;
    let mut original = BlockState::new("grass");
    original.properties.insert(
        "keep".into(),
        rust_water_structure::nbt::Nbt::Long(9007199254740993),
    );
    let expected = BlockState {
        name: "minecraft:grass_block".into(),
        properties: original.properties.clone(),
    };
    assert_eq!(upgrade::upgrade(&original, version - 1), expected);
    assert_eq!(upgrade::upgrade(&original, version), expected);
    assert_eq!(upgrade::upgrade(&original, version + 1), original);
    assert_eq!(original.name, "minecraft:grass");
}

#[test]
fn akko_entire_persistent_palette_matches_original_go() {
    let cases: Vec<OracleCase> = serde_json::from_str(
        include_str!("fixtures/akko_block_upgrade.json").trim_start_matches('\u{feff}'),
    )
    .unwrap();
    assert_eq!(cases.len(), 28);
    assert_eq!(
        cases
            .iter()
            .filter(|case| case.input != case.expected)
            .count(),
        9
    );
    for case in cases {
        assert_eq!(case.version, 17959425);
        assert_eq!(
            upgrade::upgrade(&case.input, case.version),
            case.expected,
            "{} {}",
            case.label,
            case.input.name
        );
    }
}

#[test]
fn embedded_schemas_match_original_module_hashes() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/block_upgrade_schemas");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let files = manifest["files"].as_array().unwrap();
    assert_eq!(files.len(), 35);
    for file in files {
        let name = file["file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        assert_eq!(bytes.len() as u64, file["bytes"].as_u64().unwrap());
        assert_eq!(
            hex::encode(Sha256::digest(&bytes)),
            file["sha256"].as_str().unwrap(),
            "{name}"
        );
    }
    for license in manifest["licenses"].as_array().unwrap() {
        let name = license["file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        assert_eq!(
            hex::encode(Sha256::digest(bytes)),
            license["sha256"].as_str().unwrap(),
            "{name}"
        );
    }
}
