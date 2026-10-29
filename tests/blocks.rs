use rust_water_structure::nbt::{Compound, Nbt};
use rust_water_structure::{BlockState, blocks};

#[test]
fn legacy_java_and_runtime_conversions_agree_for_stone() {
    let legacy = blocks::legacy("stone", 0);
    assert_eq!(legacy.name, "minecraft:stone");
    assert_eq!(blocks::java("minecraft:stone"), legacy);
    let runtime_id = blocks::runtime_id(&legacy);
    assert_eq!(blocks::from_runtime_id(runtime_id), legacy);
    assert_eq!(blocks::from_schematic(1, 0), legacy);
    assert_eq!(blocks::to_schematic(&legacy), (1, 0));
}

#[test]
fn unknown_state_uses_original_unknown_block() {
    assert_eq!(
        blocks::legacy("does_not_exist", 0).name,
        "minecraft:unknown"
    );
    assert_eq!(
        blocks::runtime_id(&BlockState::new("unknown")),
        u32::MAX - 1
    );
}

#[test]
fn legacy_direction_and_slab_values_match_go_reference() {
    for (name, value, expected) in [
        ("chest", 2, 3162850062),
        ("chest", 3, 2001328343),
        ("chest", 4, 3162850062),
        ("chest", 5, 3162850062),
        ("furnace", 4, 4132236718),
        ("wooden_slab", 8, 1728771929),
        ("wooden_slab", 9, 2055113703),
    ] {
        let state = blocks::legacy(name, value);
        assert_eq!(
            blocks::runtime_id(&state),
            expected,
            "{name}:{value} actual={state:?}, expected={:?}",
            blocks::from_runtime_id(expected)
        );
    }
}

#[test]
fn names_without_properties_use_go_default_anchor() {
    for (name, expected) in [
        ("cobblestone_wall", 4033176459),
        ("furnace", 860458561),
        ("hay_block", 1514836139),
    ] {
        assert_eq!(blocks::runtime_id(&blocks::java(name)), expected, "{name}");
    }
}

#[test]
fn bedrock_properties_match_go_fuzzy_runtime_search() {
    let mut exact = Compound::new();
    exact.insert("upside_down_bit".into(), Nbt::Byte(0));
    exact.insert("weirdo_direction".into(), Nbt::Int(2));
    let state = blocks::bedrock("minecraft:oak_stairs", exact);
    assert_eq!(state.name, "minecraft:oak_stairs");
    assert_eq!(state.properties["upside_down_bit"], Nbt::Byte(0));
    assert_eq!(state.properties["weirdo_direction"], Nbt::Int(2));

    let mut float = Compound::new();
    float.insert("upside_down_bit".into(), Nbt::Byte(0));
    float.insert("weirdo_direction".into(), Nbt::Double(2.9));
    let state = blocks::bedrock("minecraft:oak_stairs", float);
    assert_eq!(state.name, "minecraft:oak_stairs");
    assert_eq!(state.properties["weirdo_direction"], Nbt::Int(2));

    let mut byte_string = Compound::new();
    byte_string.insert("upside_down_bit".into(), Nbt::String("0b".into()));
    byte_string.insert("weirdo_direction".into(), Nbt::Int(2));
    let state = blocks::bedrock("minecraft:oak_stairs", byte_string);
    assert_eq!(state.properties["upside_down_bit"], Nbt::Byte(0));
    assert_eq!(state.properties["weirdo_direction"], Nbt::Int(2));

    let mut extra = Compound::new();
    extra.insert("upside_down_bit".into(), Nbt::Byte(0));
    extra.insert("weirdo_direction".into(), Nbt::Int(2));
    extra.insert("extra".into(), Nbt::Int(1));
    let state = blocks::bedrock("minecraft:oak_stairs", extra);
    assert_eq!(state.name, "minecraft:oak_stairs");
    assert_eq!(state.properties["weirdo_direction"], Nbt::Int(2));

    let mut unknown_value = Compound::new();
    unknown_value.insert("upside_down_bit".into(), Nbt::Byte(0));
    unknown_value.insert("weirdo_direction".into(), Nbt::String("unknown".into()));
    let state = blocks::bedrock("minecraft:oak_stairs", unknown_value);
    assert_eq!(state.name, "minecraft:oak_stairs");
    assert_eq!(state.properties["weirdo_direction"], Nbt::Int(0));

    let mut textual = Compound::new();
    textual.insert("upside_down_bit".into(), Nbt::String("false".into()));
    textual.insert("weirdo_direction".into(), Nbt::String("2".into()));
    let state = blocks::bedrock("minecraft:oak_stairs", textual);
    assert_eq!(state.name, "minecraft:oak_stairs");
    assert_eq!(state.properties["upside_down_bit"], Nbt::Byte(0));
    assert_eq!(state.properties["weirdo_direction"], Nbt::Int(2));

    let mut nonzero_bool = Compound::new();
    nonzero_bool.insert("upside_down_bit".into(), Nbt::Int(2));
    nonzero_bool.insert("weirdo_direction".into(), Nbt::Int(2));
    let state = blocks::bedrock("minecraft:oak_stairs", nonzero_bool);
    assert_eq!(state.name, "minecraft:oak_stairs");
    assert_eq!(state.properties["upside_down_bit"], Nbt::Byte(1));
    assert_eq!(state.properties["weirdo_direction"], Nbt::Int(2));

    let mut door = Compound::new();
    door.insert("direction".into(), Nbt::Int(0));
    door.insert("door_hinge_bit".into(), Nbt::Byte(0));
    door.insert("open_bit".into(), Nbt::Byte(0));
    door.insert("upper_block_bit".into(), Nbt::Byte(0));
    let state = blocks::bedrock("minecraft:oak_door", door);
    assert_eq!(state.name, "minecraft:wooden_door");
    assert_eq!(state.properties["direction"], Nbt::Int(0));
    assert_eq!(state.properties["door_hinge_bit"], Nbt::Byte(0));
    assert_eq!(state.properties["open_bit"], Nbt::Byte(0));
    assert_eq!(state.properties["upper_block_bit"], Nbt::Byte(1));

    let mut invalid = Compound::new();
    invalid.insert("open_bit".into(), Nbt::Long(1));
    assert_eq!(
        blocks::bedrock("minecraft:oak_door", invalid),
        BlockState::air()
    );
    let mut invalid_byte = Compound::new();
    invalid_byte.insert("open_bit".into(), Nbt::Byte(2));
    assert_eq!(
        blocks::bedrock("minecraft:oak_door", invalid_byte),
        BlockState::air()
    );
    assert_eq!(
        blocks::bedrock("minecraft:not_a_real_block", Compound::new()),
        BlockState::air()
    );
}

#[test]
fn block_name_and_state_str_to_runtime_id_matches_go() {
    let cases = [
        (
            "exact",
            "minecraft:oak_stairs",
            "upside_down_bit=0b,weirdo_direction=2",
            1247,
            true,
        ),
        (
            "bool_string",
            "minecraft:oak_stairs",
            "upside_down_bit=false,weirdo_direction=2",
            1247,
            true,
        ),
        (
            "quoted_key_and_value",
            "minecraft:oak_stairs",
            "\"upside_down_bit\"=\"false\",weirdo_direction=\"2\"",
            1247,
            true,
        ),
        (
            "minecraft_prefix_colon_syntax",
            "minecraft:oak_stairs",
            "minecraft:upside_down_bit:0b,minecraft:weirdo_direction:2",
            1247,
            true,
        ),
        (
            "unqualified_block_name",
            "oak_stairs",
            "upside_down_bit=0b,weirdo_direction=2",
            1247,
            true,
        ),
        (
            "bool_nonzero_int",
            "minecraft:oak_stairs",
            "upside_down_bit=2,weirdo_direction=2",
            1251,
            true,
        ),
        (
            "extra_property",
            "minecraft:oak_stairs",
            "upside_down_bit=0b,weirdo_direction=2,extra=1",
            1247,
            true,
        ),
        (
            "unknown_value",
            "minecraft:oak_stairs",
            "upside_down_bit=0b,weirdo_direction=unknown",
            1245,
            true,
        ),
        (
            "duplicate_last_wins",
            "minecraft:oak_stairs",
            "upside_down_bit=0b,upside_down_bit=1b,weirdo_direction=2",
            1251,
            true,
        ),
        ("empty_state", "minecraft:oak_stairs", "", 1245, true),
        (
            "malformed_state",
            "minecraft:oak_stairs",
            "facing",
            11982,
            false,
        ),
        (
            "unknown_name",
            "minecraft:not_a_real_block",
            "",
            11982,
            false,
        ),
    ];
    for (label, name, state, expected_id, expected_found) in cases {
        assert_eq!(
            blocks::block_name_and_state_str_to_runtime_id(name, state),
            (expected_id, expected_found),
            "{label}"
        );
    }
}

#[test]
fn block_name_and_state_str_invalid_namespace_panics_like_go() {
    let result = std::panic::catch_unwind(|| {
        blocks::block_name_and_state_str_to_runtime_id("one:two:three", "")
    });
    let payload = result.expect_err("a block name with multiple namespace separators must panic");
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .expect("panic payload must be printable text");
    assert_eq!(message, "[one two three]");
    assert_eq!(
        blocks::block_name_and_state_str_to_runtime_id("one:two:three", "facing"),
        (11982, false)
    );
}
