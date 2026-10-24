use std::process::Command;

#[test]
fn formats_lists_all_original_ids_and_marks_sibi_unavailable() {
    let output = Command::new(env!("CARGO_BIN_EXE_rust-water-structure"))
        .arg("formats")
        .output()
        .unwrap();
    assert!(output.status.success());
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 37);
    assert_eq!(rows[32]["name"], "SIBI");
    assert_eq!(rows[32]["capabilities"]["write"], false);
}

#[test]
fn cli_inspect_reads_real_structure_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("sample.json");
    std::fs::write(&file, br#"[{"name":"stone","aux":0,"x":-10,"y":64,"z":4}]"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rust-water-structure"))
        .arg("inspect")
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["format"], "RunAway");
    assert_eq!(value["size"], serde_json::json!([1, 1, 1]));
    assert_eq!(value["non_air"], 1);
}

#[test]
fn directory_output_applies_offset() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.json");
    let output_path = temp.path().join("world-output");
    std::fs::write(&source, br#"[{"name":"stone","aux":0,"x":0,"y":0,"z":0}]"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rust-water-structure"))
        .arg("convert")
        .arg(&source)
        .arg(&output_path)
        .args(["--to", "MCWorld", "--offset", "16,0,0"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = rust_water_structure::world::read_directory(&output_path, None).unwrap();
    assert_eq!(data.size, rust_water_structure::Size(17, 1, 1));
    assert!(
        data.blocks
            .contains_key(&rust_water_structure::Pos(16, 0, 0))
    );
}
#[test]
fn compact_conversion_reports_entity_and_metadata_loss() {
    use rust_water_structure::{
        Block, BlockState, FormatId, Pos, Size, StructureData, formats, nbt::Nbt,
    };
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("entity.mcstructure");
    let output_path = temporary.path().join("output.mcworld");
    let mut data = StructureData::new(FormatId::MCStructure);
    data.size = Size(1, 1, 1);
    data.insert(Pos::default(), Block::new(BlockState::new("stone")));
    data.entities.push(std::collections::BTreeMap::from([(
        "identifier".into(),
        Nbt::String("minecraft:pig".into()),
    )]));
    data.metadata.insert("custom_metadata".into(), Nbt::Long(7));
    std::fs::write(
        &source,
        formats::java::encode(FormatId::MCStructure, &data).unwrap(),
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rust-water-structure"))
        .arg("convert")
        .arg(source)
        .arg(output_path)
        .args(["--to", "MCWorld"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["loss"]["entities"], 1);
    assert_eq!(value["loss"]["metadata"], true);
}
#[test]
#[cfg(windows)]
fn windows_case_alias_cannot_overwrite_the_input() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source.json");
    let original = br#"[{"name":"stone","aux":0,"x":0,"y":0,"z":0}]"#;
    std::fs::write(&source, original).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rust-water-structure"))
        .arg("convert")
        .arg(&source)
        .arg(source.to_string_lossy().to_uppercase())
        .args(["--to", "MCFunction"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(std::fs::read(source).unwrap(), original);
}
