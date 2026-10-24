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
