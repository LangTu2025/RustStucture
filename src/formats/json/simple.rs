use super::*;
use crate::Size;

pub(super) fn decode(format: FormatId, bytes: &[u8]) -> Result<StructureData> {
    let root: Value = serde_json::from_slice(bytes)?;
    let mut output = StructureData::new(format);
    match format {
        FormatId::RunAway => {
            for entry in array(&root)? {
                let name = text(&entry["name"])?;
                if name.trim().is_empty() {
                    return Err(Error::invalid("方块名称为空"));
                }
                let pos = Pos(
                    integer(&entry["x"])?,
                    integer(&entry["y"])?,
                    integer(&entry["z"])?,
                );
                output.insert(
                    pos,
                    Block::new(blocks::legacy(name, integer_or(&entry["aux"], 0) as u16)),
                );
            }
        }
        FormatId::TimeBuilderV1 => {
            if text(&root["version"])?.trim() != "TimeBuilder" {
                return Err(Error::invalid("TimeBuilder 版本标识无效"));
            }
            for entry in array(&root["block"])? {
                let state =
                    blocks::legacy(text(&entry["name"])?, integer_or(&entry["aux"], 0) as u16);
                for pos in array(&entry["pos"])? {
                    let values = array(pos)?;
                    if values.len() >= 3 {
                        let position = Pos(
                            integer(&values[0])?,
                            integer(&values[1])?,
                            integer(&values[2])?,
                        );
                        output.insert(position, Block::new(state.clone()));
                    }
                }
            }
        }
        FormatId::QingXuV1 => decode_qingxu(&root, &mut output)?,
        FormatId::CovStructure => return decode_cov(&root, output),
        _ => unreachable!(),
    }
    finish(output)
}

fn decode_qingxu(root: &Value, output: &mut StructureData) -> Result<()> {
    let count = integer(&root["totalBlocks"])?;
    for i in 0..count {
        let value = &root[i.to_string()];
        if value.is_null() || value.as_str() == Some("") {
            continue;
        }
        let chunk: Value = serde_json::from_str(text(value)?)?;
        let count = integer_or(&chunk["totalPoints"], 0);
        for j in 0..count {
            let value = &chunk[j.to_string()];
            if value.is_null() || value.as_str() == Some("") {
                continue;
            }
            let entry: Value = serde_json::from_str(text(value)?)?;
            let raw = text(&entry["Name"])?;
            let name = match raw.split_once('.') {
                Some((prefix, suffix)) => format!("{suffix}_{prefix}"),
                None => raw.to_owned(),
            };
            let pos = Pos(
                integer(&entry["X"])?,
                integer(&entry["Y"])?,
                integer(&entry["Z"])?,
            );
            output.insert(pos, Block::new(blocks::java(&name)));
        }
    }
    Ok(())
}

fn find_palette(value: &Value) -> Vec<&Value> {
    if let Some(values) = value.as_array() {
        return values.iter().filter(|v| v.is_object()).collect();
    }
    if let Some(values) = value.as_object() {
        for value in values.values() {
            if value["block_palette"].is_array() {
                return find_palette(&value["block_palette"]);
            }
        }
        return values
            .values()
            .filter(|v| {
                [
                    "name",
                    "states",
                    "properties",
                    "id",
                    "block",
                    "data",
                    "meta",
                ]
                .iter()
                .any(|key| !v[*key].is_null())
            })
            .collect();
    }
    Vec::new()
}

fn cov_state(value: &Value) -> BlockState {
    let name = ["name", "block", "id"]
        .iter()
        .find_map(|key| value[*key].as_str())
        .unwrap_or("minecraft:air");
    let data = ["data", "meta", "damage", "value"]
        .iter()
        .find_map(|key| integer(&value[*key]).ok());
    match data {
        Some(data) => blocks::legacy(name, data as u16),
        None => blocks::java(name),
    }
}

fn flatten<'a>(value: &'a Value, output: &mut Vec<&'a Value>) {
    if let Some(values) = value.as_array() {
        for value in values {
            flatten(value, output);
        }
    } else {
        output.push(value);
    }
}

fn decode_cov(root: &Value, mut output: StructureData) -> Result<StructureData> {
    let dimensions = root
        .get("size")
        .or_else(|| root.get("dimensions"))
        .ok_or_else(|| Error::invalid("CovStructure 缺少尺寸"))?;
    let Pos(x, y, z) = triple(dimensions)?;
    if x <= 0 || y <= 0 || z <= 0 {
        return Err(Error::invalid("CovStructure 尺寸必须大于零"));
    }
    output.size = Size(x as u32, y as u32, z as u32);
    let structure = &root["structure"];
    let palette_value = structure
        .get("palette")
        .or_else(|| root.get("palette"))
        .unwrap_or(&Value::Null);
    let palette: BTreeMap<i32, BlockState> = find_palette(palette_value)
        .into_iter()
        .enumerate()
        .map(|(index, value)| (integer_or(&value["val"], index as i32), cov_state(value)))
        .collect();
    let indices = structure
        .get("block_indices")
        .or_else(|| structure.get("blocks"));
    if let Some(indices) = indices {
        let mut flat = Vec::new();
        flatten(indices, &mut flat);
        for (index, value) in flat
            .into_iter()
            .take(output.size.volume() as usize)
            .enumerate()
        {
            let state = if value.is_object() {
                Some(cov_state(value))
            } else {
                integer(value).ok().and_then(|id| palette.get(&id).cloned())
            };
            if let Some(state) = state {
                let index = index as i64;
                let pos = Pos(
                    (index % i64::from(x)) as i32,
                    (index / (i64::from(x) * i64::from(z))) as i32,
                    ((index / i64::from(x)) % i64::from(z)) as i32,
                );
                output.insert(pos, Block::new(state));
            }
        }
    }
    Ok(output)
}

pub(super) fn encode(format: FormatId, data: &StructureData) -> Result<Vec<u8>> {
    let mut entries = Vec::new();
    for (pos, block) in &data.blocks {
        let Pos(x, y, z) = world(data, *pos)?;
        let (name, aux) = blocks::to_legacy(&block.state);
        entries.push(json!({"name":name,"aux":aux,"x":x,"y":y,"z":z}));
    }
    let root = match format {
        FormatId::RunAway => Value::Array(entries),
        FormatId::TimeBuilderV1 => {
            let groups: Vec<Value> = entries.iter().map(|entry| {
                json!({"name":entry["name"],"aux":entry["aux"],"pos":[[entry["x"],entry["y"],entry["z"]]]})
            }).collect();
            json!({"version":"TimeBuilder","block":groups})
        }
        FormatId::QingXuV1 => {
            let mut chunk = serde_json::Map::new();
            chunk.insert("totalPoints".into(), json!(data.blocks.len()));
            for (i, (pos, block)) in data.blocks.iter().enumerate() {
                let Pos(x, y, z) = world(data, *pos)?;
                chunk.insert(
                    i.to_string(),
                    Value::String(
                        json!({
                        "Name":block.state.string(),"X":x,"Y":y,"Z":z
                            })
                        .to_string(),
                    ),
                );
            }
            json!({"totalBlocks":1,"0":Value::Object(chunk).to_string()})
        }
        FormatId::CovStructure => encode_cov(data)?,
        _ => unreachable!(),
    };
    Ok(serde_json::to_vec(&root)?)
}

fn encode_cov(data: &StructureData) -> Result<Value> {
    let volume =
        usize::try_from(data.size.volume()).map_err(|_| Error::invalid("CovStructure 尺寸过大"))?;
    let mut indices = Vec::new();
    indices
        .try_reserve_exact(volume)
        .map_err(|_| Error::invalid("CovStructure 方块索引过大"))?;
    indices.resize(volume, -1_i32);
    let mut palette = Vec::<Value>::new();
    let mut state_ids = BTreeMap::<String, i32>::new();
    for (pos, block) in &data.blocks {
        if pos.0 < 0
            || pos.1 < 0
            || pos.2 < 0
            || pos.0 as u32 >= data.size.0
            || pos.1 as u32 >= data.size.1
            || pos.2 as u32 >= data.size.2
        {
            return Err(Error::invalid("CovStructure 方块超出尺寸"));
        }
        let key = block.state.string();
        let id = *state_ids.entry(key).or_insert_with(|| {
            let id = palette.len() as i32;
            let (name, aux) = blocks::to_legacy(&block.state);
            palette.push(json!({"name":name,"data":aux,"val":id}));
            id
        });
        let index = pos.0 as u64
            + u64::from(data.size.0) * (pos.2 as u64 + u64::from(data.size.2) * pos.1 as u64);
        indices[index as usize] = id;
    }
    Ok(
        json!({"size":[data.size.0,data.size.1,data.size.2],"structure":{"palette":palette,"block_indices":indices}}),
    )
}
