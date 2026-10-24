use super::*;

const KEY: &[u8] = b"FuHongBuild";

pub(super) fn decode(format: FormatId, bytes: &[u8]) -> Result<StructureData> {
    let plain = if format == FormatId::FuHongV5 {
        let encrypted = zlib_decode(bytes)?;
        let encrypted = std::str::from_utf8(&encrypted)
            .map_err(|_| Error::invalid("FuHong V5 密文不是 UTF-8"))?;
        let mut plain = String::new();
        for (index, value) in encrypted.chars().enumerate() {
            let value = (value as u32)
                .checked_sub((index % 3) as u32)
                .ok_or_else(|| Error::invalid("FuHong V5 字符无效"))?;
            let value = char::from_u32(value ^ u32::from(KEY[index % KEY.len()]))
                .ok_or_else(|| Error::invalid("FuHong V5 解密字符无效"))?;
            plain.push(value);
        }
        plain.into_bytes()
    } else {
        bytes.to_vec()
    };
    let root: Value = serde_json::from_slice(&plain)?;
    let mut output = StructureData::new(format);
    match format {
        FormatId::FuHongV1 => decode_v1(&root, &mut output)?,
        FormatId::FuHongV2 => decode_v2(&root, &mut output)?,
        _ => decode_groups(&root, &mut output)?,
    }
    finish(output)
}

fn state(name: &str, data: i32) -> BlockState {
    if name.contains('[') {
        blocks::java(name)
    } else {
        blocks::legacy(name, data as u16)
    }
}

fn first(value: &Value) -> Result<i32> {
    if let Some(values) = value.as_array() {
        return values
            .first()
            .ok_or_else(|| Error::invalid("FuHong 坐标数组为空"))
            .and_then(integer);
    }
    integer(value)
}

fn decode_v1(root: &Value, output: &mut StructureData) -> Result<()> {
    for entry in array(root)? {
        let name = text(&entry["name"])?;
        if name.trim().is_empty() {
            return Err(Error::invalid("FuHong 方块名称为空"));
        }
        let pos = Pos(
            first(&entry["x"])?,
            first(&entry["y"])?,
            first(&entry["z"])?,
        );
        output.insert(pos, Block::new(state(name, integer_or(&entry["aux"], 0))));
    }
    Ok(())
}

fn coordinates<'a>(
    x: &'a Value,
    y: &'a Value,
    z: &'a Value,
) -> Result<(&'a Vec<Value>, &'a Vec<Value>, &'a Vec<Value>)> {
    let xs = array(x)?;
    let ys = array(y)?;
    let zs = array(z)?;
    if xs.len() != ys.len() || xs.len() != zs.len() {
        return Err(Error::invalid("FuHong 坐标数组长度不一致"));
    }
    Ok((xs, ys, zs))
}

fn indexed(value: &Value, index: usize) -> &Value {
    value
        .as_array()
        .and_then(|values| values.get(index).or_else(|| values.last()))
        .unwrap_or(&Value::Null)
}

fn decode_v2(root: &Value, output: &mut StructureData) -> Result<()> {
    for chunk in array(&root["FuHongBuild_FinalFormat"])? {
        for entry in array(&chunk["block"])? {
            let Some(name) = entry["n"].as_str() else {
                continue;
            };
            let (xs, ys, zs) = coordinates(&entry["x"], &entry["y"], &entry["z"])?;
            let mut properties = Compound::new();
            if let Some(states) = entry["state"].as_array() {
                for value in states.iter().filter_map(Value::as_str) {
                    if let Some((key, value)) = value.split_once('=') {
                        let key = key.trim().trim_matches('"');
                        let value = value.trim().trim_matches('"');
                        let property = match value {
                            "true" => Nbt::Byte(1),
                            "false" => Nbt::Byte(0),
                            _ => value
                                .parse::<i32>()
                                .map(Nbt::Int)
                                .unwrap_or_else(|_| Nbt::String(value.into())),
                        };
                        properties.insert(key.to_owned(), property);
                    }
                }
            }
            for i in 0..xs.len() {
                let pos = Pos(integer(&xs[i])?, integer(&ys[i])?, integer(&zs[i])?);
                let aux = if entry["a"].is_array() {
                    integer_or(indexed(&entry["a"], i), 0)
                } else {
                    integer_or(&entry["a"], 0)
                };
                let block_state = if properties.is_empty() {
                    state(name, aux)
                } else {
                    blocks::bedrock(name, properties.clone())
                };
                let mut block = Block::new(block_state);
                if entry["c"].is_object() {
                    let command = &entry["c"];
                    let values = vec![
                        indexed(&command["c"], i).clone(),
                        indexed(&command["t"], i).clone(),
                        indexed(&command["a"], i).clone(),
                        indexed(&command["n"], i).clone(),
                    ];
                    block.nbt = Some(command_nbt(&values));
                }
                if let Some(payload) = entry["d"].as_array().and_then(|values| values.get(i)) {
                    let container = if let Some(raw) = payload["e"].as_str() {
                        snbt(raw).ok()
                    } else {
                        payload["d"]
                            .as_array()
                            .map(|items| container_nbt(name, items, false))
                    };
                    if let Some(container) = container {
                        block
                            .nbt
                            .get_or_insert_with(Compound::new)
                            .extend(container);
                    }
                }
                if let Some(nbt) = &mut block.nbt {
                    if let Some(value) = properties.get("conditional_bit") {
                        nbt.insert(
                            "conditionalMode".into(),
                            Nbt::Byte(i8::from(
                                value.scalar_string() == "true" || value.as_i64() == Some(1),
                            )),
                        );
                    }
                    if nbt.get("CustomName").and_then(Nbt::as_str) == Some("") {
                        nbt.remove("CustomName");
                    }
                }
                insert(output, pos, block, false);
            }
        }
    }
    Ok(())
}

fn decode_groups(root: &Value, output: &mut StructureData) -> Result<()> {
    let names = strings(&root["BlocksList"])?;
    for chunk in array(&root["FuHongBuild"])? {
        let chunk_origin = if output.format == FormatId::FuHongV3 {
            Pos(integer(&chunk["startX"])?, 0, integer(&chunk["startZ"])?)
        } else {
            Pos(0, 0, 0)
        };
        for entry in array(&chunk["block"])? {
            let values = array(entry)?;
            if values.len() < 5 {
                return Err(Error::invalid("FuHong 分组方块条目长度不足"));
            }
            if values[0].is_string() {
                continue;
            }
            let index = integer(&values[0])?;
            let name = names
                .get(index as usize)
                .ok_or_else(|| Error::invalid("FuHong 调色板索引越界"))?;
            let block_state = state(name, integer(&values[1])?);
            let (xs, ys, zs) = coordinates(&values[2], &values[3], &values[4])?;
            let extras = values.get(5).and_then(Value::as_array);
            for i in 0..xs.len() {
                let pos = Pos(
                    add(chunk_origin.0, integer(&xs[i])?)?,
                    integer(&ys[i])?,
                    add(chunk_origin.2, integer(&zs[i])?)?,
                );
                let mut block = Block::new(block_state.clone());
                if let Some(payload) = extras.and_then(|values| values.get(i)) {
                    block.nbt = extra_nbt(name, payload);
                }
                insert(output, pos, block, true);
            }
        }
    }
    Ok(())
}

fn display(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "<nil>".into(),
        _ => value.to_string(),
    }
}

fn command_nbt(values: &[Value]) -> Compound {
    let get = |index| values.get(index).unwrap_or(&Value::Null);
    let command = display(get(0));
    let version = if command.contains("execute")
        && (command.contains(" run ") || command.contains(" as ") || command.contains(" at "))
    {
        38
    } else {
        19
    };
    BTreeMap::from([
        ("id".into(), Nbt::String("CommandBlock".into())),
        ("Command".into(), Nbt::String(command)),
        (
            "CustomName".into(),
            Nbt::String(values.get(3).map(display).unwrap_or_default()),
        ),
        ("TickDelay".into(), Nbt::Int(integer_or(get(1), 0))),
        ("auto".into(), byte_bool(get(2))),
        ("ExecuteOnFirstTick".into(), Nbt::Byte(1)),
        ("conditionalMode".into(), Nbt::Byte(0)),
        ("TrackOutput".into(), Nbt::Byte(1)),
        ("Version".into(), Nbt::Int(version)),
    ])
}

fn extra_nbt(name: &str, payload: &Value) -> Option<Compound> {
    if payload.is_null() {
        return None;
    }
    if name.contains("command_block") {
        return payload
            .as_array()
            .filter(|values| !values.is_empty())
            .map(|values| command_nbt(values));
    }
    if chest_id(name).is_some() {
        return Some(container_nbt(
            name,
            payload.as_array().map(Vec::as_slice).unwrap_or(&[]),
            true,
        ));
    }
    if name.contains("sign") {
        let text = match payload {
            Value::Array(lines) => lines.iter().map(display).collect::<Vec<_>>().join("\n"),
            _ => display(payload),
        };
        return Some(sign_nbt(name, &text));
    }
    None
}

fn first_present<'a>(value: &'a Value, names: &[&str]) -> &'a Value {
    names
        .iter()
        .find_map(|key| value.get(*key).filter(|value| !value.is_null()))
        .unwrap_or(&Value::Null)
}

fn container_nbt(name: &str, values: &[Value], rich: bool) -> Compound {
    let mut output = Compound::new();
    if let Some(id) = chest_id(name) {
        output.insert("id".into(), Nbt::String(id.into()));
    }
    if rich {
        output.insert("Findable".into(), Nbt::Byte(0));
        output.insert("IsOpened".into(), Nbt::Byte(0));
        output.insert("isMovable".into(), Nbt::Byte(1));
    }
    let mut items = Vec::new();
    for item in values {
        let (name, damage, count, slot) =
            if let Some(tuple) = item.as_array().filter(|values| values.len() >= 4) {
                (
                    display(&tuple[0]),
                    integer_or(&tuple[1], 0),
                    integer_or(&tuple[2], 0),
                    integer_or(&tuple[3], 0),
                )
            } else if item.is_object() {
                (
                    display(first_present(item, &["Name", "name", "ns"])),
                    integer_or(first_present(item, &["Damage", "damage", "aux"]), 0),
                    integer_or(first_present(item, &["Count", "count", "num"]), 0),
                    integer_or(first_present(item, &["Slot", "slot"]), 0),
                )
            } else {
                continue;
            };
        let name = if name.contains(':') || name.is_empty() {
            name
        } else {
            format!("minecraft:{name}")
        };
        let mut item = BTreeMap::from([
            ("Name".into(), Nbt::String(name.clone())),
            ("Damage".into(), Nbt::Short(damage as i16)),
            ("Count".into(), Nbt::Byte(count as i8)),
            ("Slot".into(), Nbt::Byte(slot as i8)),
        ]);
        if rich {
            item.insert(
                "Block".into(),
                Nbt::Compound(BTreeMap::from([
                    ("name".into(), Nbt::String(name)),
                    ("states".into(), Nbt::Compound(Compound::new())),
                    ("val".into(), Nbt::Short(0)),
                    ("version".into(), Nbt::Int(17959425)),
                ])),
            );
        }
        items.push(Nbt::Compound(item));
    }
    if rich || !items.is_empty() {
        output.insert("Items".into(), Nbt::List(items));
    }
    output
}

fn sign_nbt(name: &str, text: &str) -> Compound {
    let side = |text: &str| {
        Nbt::Compound(BTreeMap::from([
            ("FilteredText".into(), Nbt::String(String::new())),
            ("HideGlowOutline".into(), Nbt::Byte(0)),
            ("IgnoreLighting".into(), Nbt::Byte(0)),
            ("PersistFormatting".into(), Nbt::Byte(1)),
            ("SignTextColor".into(), Nbt::Int(-16777216)),
            ("Text".into(), Nbt::String(text.to_owned())),
            ("TextOwner".into(), Nbt::String(String::new())),
        ]))
    };
    BTreeMap::from([
        (
            "id".into(),
            Nbt::String(
                if name.ends_with("hanging_sign") {
                    "HangingSign"
                } else {
                    "Sign"
                }
                .into(),
            ),
        ),
        ("IsWaxed".into(), Nbt::Byte(0)),
        ("isMovable".into(), Nbt::Byte(1)),
        ("BackText".into(), side("")),
        ("FrontText".into(), side(text)),
    ])
}

fn extra_payload(block: &Block) -> Option<Value> {
    let nbt = block.nbt.as_ref()?;
    if block.state.name.contains("command_block") {
        return Some(json!([
            nbt_text(nbt, "Command"),
            nbt_number(nbt, "TickDelay"),
            nbt_number(nbt, "auto"),
            nbt_text(nbt, "CustomName")
        ]));
    }
    if chest_id(&block.state.name).is_some() {
        let items: Vec<Value> = nbt
            .get("Items")
            .and_then(Nbt::as_list)
            .unwrap_or_default()
            .iter()
            .filter_map(Nbt::as_compound)
            .map(|item| {
                json!([
                    nbt_text(item, "Name"),
                    nbt_number(item, "Damage"),
                    nbt_number(item, "Count"),
                    nbt_number(item, "Slot")
                ])
            })
            .collect();
        return Some(json!(items));
    }
    if block.state.name.contains("sign") {
        let text = nbt
            .get("FrontText")
            .and_then(Nbt::as_compound)
            .map(|front| nbt_text(front, "Text"))
            .unwrap_or_else(|| nbt_text(nbt, "Text"));
        return Some(json!(text));
    }
    None
}

pub(super) fn encode(format: FormatId, data: &StructureData) -> Result<Vec<u8>> {
    let mut entries = Vec::new();
    let (names, indices) = palette(data);
    for (pos, block) in &data.blocks {
        let Pos(x, y, z) = world(data, *pos)?;
        let (name, aux) = blocks::to_legacy(&block.state);
        let entry = match format {
            FormatId::FuHongV1 => json!({"name":name,"aux":aux,"x":[x],"y":[y],"z":[z]}),
            FormatId::FuHongV2 => {
                let mut entry = json!({"n":name,"a":aux,"x":[x],"y":[y],"z":[z]});
                if !block.state.properties.is_empty() {
                    entry["state"] = json!(
                        block
                            .state
                            .properties
                            .iter()
                            .map(|(key, value)| format!("{key}={}", value.scalar_string()))
                            .collect::<Vec<_>>()
                    );
                }
                if let Some(nbt) = &block.nbt {
                    entry["d"] = json!([{"e":nbt_string(nbt)}]);
                }
                entry
            }
            _ => {
                let mut entry = vec![
                    json!(indices[&name]),
                    json!(aux),
                    json!([x]),
                    json!([y]),
                    json!([z]),
                ];
                if let Some(extra) = extra_payload(block) {
                    entry.push(json!([extra]));
                }
                json!(entry)
            }
        };
        entries.push(entry);
    }
    let root = match format {
        FormatId::FuHongV1 => json!(entries),
        FormatId::FuHongV2 => {
            json!({"Build_Info":{},"FuHongBuild_FinalFormat":[{"block":entries}]})
        }
        _ => {
            json!({"FuHongBuild":[{"startX":0,"startZ":0,"block":entries}],"BlocksList":names,"BlockCalculationPos":false})
        }
    };
    let plain = serde_json::to_vec(&root)?;
    if format != FormatId::FuHongV5 {
        return Ok(plain);
    }
    let plain =
        std::str::from_utf8(&plain).map_err(|_| Error::invalid("FuHong V5 JSON 不是 UTF-8"))?;
    let mut encrypted = String::new();
    for (index, value) in plain.chars().enumerate() {
        let value = (value as u32 ^ u32::from(KEY[index % KEY.len()])) + (index % 3) as u32;
        encrypted
            .push(char::from_u32(value).ok_or_else(|| Error::invalid("FuHong V5 加密字符无效"))?);
    }
    zlib_encode(encrypted.as_bytes())
}
