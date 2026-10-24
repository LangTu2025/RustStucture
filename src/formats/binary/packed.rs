use super::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) fn decode_msgpack(format: FormatId, bytes: &[u8]) -> Result<StructureData> {
    let value: Value = rmp_serde::from_slice(bytes).map_err(|e| Error::invalid(e.to_string()))?;
    let rows = value
        .get(0)
        .and_then(Value::as_array)
        .ok_or_else(|| Error::invalid("缺少 MessagePack 方块列表"))?;
    let mut result = StructureData::new(format);
    if format == FormatId::NexusNP && value.get(1).is_none() {
        return Err(Error::invalid("NP 缺少 actor_data"));
    }
    for row in rows {
        let row = row
            .as_array()
            .ok_or_else(|| Error::invalid("方块必须是数组"))?;
        let minimum = if format == FormatId::BDS { 6 } else { 5 };
        if row.len() < minimum {
            return Err(Error::invalid("方块数组字段不足"));
        }
        let name = row[0]
            .as_str()
            .ok_or_else(|| Error::invalid("方块名称错误"))?;
        if format == FormatId::BDS
            && row[5]
                .as_bool()
                .ok_or_else(|| Error::invalid("BDS 空气标记必须为布尔值"))?
        {
            continue;
        }
        let coord = |index: usize| -> Result<i32> {
            row[index]
                .as_i64()
                .and_then(|v| i32::try_from(v).ok())
                .ok_or_else(|| Error::invalid("MessagePack 坐标错误"))
        };
        let properties = row[4]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| row[4].to_string());
        let state = super::text::state(name, &properties);
        if state.is_air() || name.trim().is_empty() {
            continue;
        }
        result
            .blocks
            .insert(Pos(coord(1)?, coord(2)?, coord(3)?), Block::new(state));
    }
    if result.blocks.is_empty() {
        return Err(Error::invalid("MessagePack 无有效方块"));
    }
    if let Some(actor) = value.get(1) {
        result
            .metadata
            .insert("block_actor_data".into(), Nbt::from_json(actor));
    }
    result.normalize()?;
    Ok(result)
}

pub(super) fn encode_msgpack(format: FormatId, data: &StructureData) -> Result<Vec<u8>> {
    let mut rows = Vec::new();
    for (pos, block) in &data.blocks {
        let pos = world_pos(data, *pos)?;
        let (name, aux) = crate::blocks::to_legacy(&block.state);
        if format == FormatId::BDS {
            rows.push(json!([
                name,
                pos.0,
                pos.1,
                pos.2,
                super::text::bedrock_states(&block.state),
                block.state.is_air()
            ]));
        } else {
            rows.push(json!([name, pos.0, pos.1, pos.2, aux]));
        }
    }
    let value = if format == FormatId::BDS {
        json!([rows])
    } else {
        json!([
            rows,
            data.metadata
                .get("block_actor_data")
                .map(Nbt::to_json)
                .unwrap_or(json!([]))
        ])
    };
    rmp_serde::to_vec(&value).map_err(|e| Error::Other(e.to_string()))
}

fn entity_id(name: &str) -> &str {
    match name.trim_start_matches("minecraft:") {
        "chest" | "trapped_chest" => "Chest",
        "barrel" => "Barrel",
        "hopper" => "Hopper",
        "furnace" => "Furnace",
        "blast_furnace" => "BlastFurnace",
        "smoker" => "Smoker",
        "dispenser" => "Dispenser",
        "dropper" => "Dropper",
        "crafter" => "Crafter",
        "command_block" | "repeating_command_block" | "chain_command_block" => "CommandBlock",
        other if other.contains("shulker_box") => "ShulkerBox",
        other if other.contains("sign") => "Sign",
        _ => name,
    }
}

fn json_integer(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value as i64)),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn modern_execute(command: &str) -> bool {
    const ARGUMENTS: [&str; 11] = [
        "as",
        "at",
        "align",
        "anchored",
        "facing",
        "in",
        "positioned",
        "rotated",
        "if",
        "unless",
        "run",
    ];
    command.match_indices("execute").any(|(index, _)| {
        let tail = command[index + "execute".len()..].trim_start_matches(' ');
        ARGUMENTS.iter().any(|argument| tail.starts_with(argument))
    })
}

fn entity_nbt(entity: &Value) -> Compound {
    let mut nbt = Nbt::from_json(entity)
        .as_compound()
        .cloned()
        .unwrap_or_default();
    for key in ["x", "y", "z"] {
        nbt.remove(key);
    }
    let id = entity["id"].as_str().unwrap_or("");
    nbt.insert("id".into(), Nbt::String(entity_id(id).into()));
    if id.ends_with("command_block") {
        nbt.clear();
        nbt.insert("id".into(), Nbt::String("CommandBlock".into()));
        for key in ["Command", "CustomName", "LastOutput"] {
            let value = match &entity[key] {
                Value::String(value) => value.clone(),
                Value::Null => "<nil>".into(),
                value => value.to_string(),
            };
            nbt.insert(key.into(), Nbt::String(value));
        }
        let boolean = |key: &str| -> i8 {
            match &entity[key] {
                Value::Bool(v) => *v as i8,
                Value::Number(_) => (json_integer(&entity[key]).unwrap_or(0) != 0) as i8,
                Value::String(v) => matches!(v.as_str(), "true" | "1") as i8,
                _ => 0,
            }
        };
        nbt.insert(
            "conditionalMode".into(),
            Nbt::Byte(boolean("isConditional")),
        );
        nbt.insert("auto".into(), Nbt::Byte(1 - boolean("redstone")));
        nbt.insert(
            "ExecuteOnFirstTick".into(),
            Nbt::Byte(boolean("ExecuteOnFirstTick")),
        );
        nbt.insert("TrackOutput".into(), Nbt::Byte(boolean("TrackOutput")));
        nbt.insert(
            "TickDelay".into(),
            Nbt::Int(json_integer(&entity["TickDelay"]).unwrap_or(0) as i32),
        );
        nbt.insert("Powered".into(), Nbt::Byte(0));
        nbt.insert(
            "LPCommandMode".into(),
            Nbt::Int(json_integer(&entity["Mode"]).unwrap_or(0) as i32),
        );
        nbt.insert(
            "Version".into(),
            Nbt::Int(
                if modern_execute(entity["Command"].as_str().unwrap_or("")) {
                    38
                } else {
                    19
                },
            ),
        );
    }
    if let Some(items) = entity["Items"].as_array() {
        let items = items
            .iter()
            .map(|item| {
                let field = |upper: &str, lower: &str| item.get(upper).or_else(|| item.get(lower));
                let name = field("Name", "name").and_then(Value::as_str).unwrap_or("");
                let mut value = Compound::new();
                value.insert(
                    "Name".into(),
                    Nbt::String(if name.contains(':') {
                        name.into()
                    } else {
                        format!("minecraft:{name}")
                    }),
                );
                for (upper, lower) in [("Count", "count"), ("Slot", "slot")] {
                    value.insert(
                        upper.into(),
                        Nbt::Byte(field(upper, lower).and_then(json_integer).unwrap_or(0) as i8),
                    );
                }
                value.insert(
                    "Damage".into(),
                    Nbt::Short(
                        field("Damage", "damage")
                            .and_then(json_integer)
                            .unwrap_or(0) as i16,
                    ),
                );
                Nbt::Compound(value)
            })
            .collect();
        nbt.insert("Items".into(), Nbt::List(items));
    }
    nbt
}

pub(super) fn decode_kbdx(bytes: &[u8]) -> Result<StructureData> {
    let mut reader = Cursor::new(bytes);
    let count = reader.u32()? as usize;
    if count == 0 || count > bytes.len().saturating_sub(4) / 20 {
        return Err(Error::invalid("KBDX 方块数量错误"));
    }
    let mut raw = Vec::with_capacity(count);
    for _ in 0..count {
        raw.push((
            Pos(
                reader.u32()? as i32,
                reader.u32()? as i32,
                reader.u32()? as i32,
            ),
            reader.u32()?,
            reader.u32()?,
        ));
    }
    let metadata: Value = serde_json::from_slice(&bytes[reader.pos..])?;
    let map = metadata
        .as_object()
        .ok_or_else(|| Error::invalid("KBDX 元数据必须是对象"))?;
    let palette: BTreeMap<_, _> = map
        .iter()
        .filter_map(|(name, id)| {
            json_integer(id)
                .and_then(|id| u32::try_from(id).ok())
                .map(|id| (id, name))
        })
        .collect();
    if palette.is_empty() {
        return Err(Error::invalid("KBDX 缺少调色板"));
    }
    let mut result = StructureData::new(FormatId::KBDX);
    for (pos, index, aux) in raw {
        let name = palette
            .get(&index)
            .ok_or_else(|| Error::invalid("KBDX 调色板索引越界"))?;
        result
            .blocks
            .insert(pos, Block::new(crate::blocks::legacy(name, aux as u16)));
    }
    if let Some(entities) = metadata["BlockEntityData"].as_array() {
        for entity in entities {
            let coordinate =
                |key: &str| json_integer(&entity[key]).and_then(|value| i32::try_from(value).ok());
            let (Some(x), Some(y), Some(z)) = (coordinate("x"), coordinate("y"), coordinate("z"))
            else {
                continue;
            };
            let pos = Pos(x, y, z);
            if let Some(block) = result.blocks.get_mut(&pos) {
                block.nbt = Some(entity_nbt(entity));
            }
        }
    }
    result.normalize()?;
    Ok(result)
}

pub(super) fn encode_kbdx(data: &StructureData) -> Result<Vec<u8>> {
    let mut palette = BTreeMap::<String, u32>::new();
    let mut output = u32::try_from(data.blocks.len())
        .map_err(|_| Error::invalid("方块数量过大"))?
        .to_le_bytes()
        .to_vec();
    let mut entities = Vec::new();
    for (pos, block) in &data.blocks {
        let pos = world_pos(data, *pos)?;
        let (name, aux) = crate::blocks::to_legacy(&block.state);
        let next = palette.len() as u32;
        let index = *palette.entry(name).or_insert(next);
        for value in [
            pos.0 as u32,
            pos.1 as u32,
            pos.2 as u32,
            index,
            u32::from(aux),
        ] {
            output.extend_from_slice(&value.to_le_bytes());
        }
        if let Some(nbt) = &block.nbt {
            let mut entity = Nbt::Compound(nbt.clone()).to_json();
            entity["x"] = json!(pos.0);
            entity["y"] = json!(pos.1);
            entity["z"] = json!(pos.2);
            if nbt.contains_key("Command") {
                entity["id"] = json!(block.state.name.trim_start_matches("minecraft:"));
                entity["isConditional"] = json!(
                    nbt.get("conditionalMode")
                        .and_then(Nbt::as_i64)
                        .unwrap_or(0)
                        != 0
                );
                entity["redstone"] = json!(nbt.get("auto").and_then(Nbt::as_i64).unwrap_or(0) == 0);
                entity["Mode"] = json!(
                    nbt.get("LPCommandMode")
                        .and_then(Nbt::as_i64)
                        .unwrap_or_else(|| {
                            if block.state.name.contains("repeating") {
                                1
                            } else if block.state.name.contains("chain") {
                                2
                            } else {
                                0
                            }
                        })
                );
            }
            entities.push(entity);
        }
    }
    let mut metadata = serde_json::Map::new();
    for (name, id) in palette {
        metadata.insert(name, json!(id));
    }
    metadata.insert("BlockEntityData".into(), json!(entities));
    output.extend(serde_json::to_vec(&metadata)?);
    Ok(output)
}
