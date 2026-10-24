use super::*;
use crate::Size;

pub(super) fn decode(format: FormatId, bytes: &[u8]) -> Result<StructureData> {
    let plain = if format == FormatId::GangBanV7 {
        zlib_decode(bytes)?
    } else {
        bytes.to_vec()
    };
    let root: Value = serde_json::from_slice(&plain)?;
    let entries = array(&root)?;
    match format {
        FormatId::GangBanV1 | FormatId::GangBanV2 => decode_v1(format, entries),
        FormatId::GangBanV3 | FormatId::GangBanV4 => decode_chunks(format, entries),
        _ => decode_stream(format, entries),
    }
}

fn declared(output: &mut StructureData, start: Pos, end: Pos) -> Result<()> {
    output.origin = Pos(start.0.min(end.0), start.1.min(end.1), start.2.min(end.2));
    let dimension = |a: i32, b: i32| {
        u32::try_from((i64::from(a) - i64::from(b)).abs() + 1)
            .map_err(|_| Error::invalid("GangBan 尺寸溢出"))
    };
    output.size = Size(
        dimension(start.0, end.0)?,
        dimension(start.1, end.1)?,
        dimension(start.2, end.2)?,
    );
    Ok(())
}

fn local(pos: Pos, origin: Pos) -> Result<Pos> {
    let subtract = |a: i32, b: i32| {
        a.checked_sub(b)
            .ok_or_else(|| Error::invalid("GangBan 相对坐标溢出"))
    };
    Ok(Pos(
        subtract(pos.0, origin.0)?,
        subtract(pos.1, origin.1)?,
        subtract(pos.2, origin.2)?,
    ))
}

fn decode_v1(format: FormatId, entries: &[Value]) -> Result<StructureData> {
    if entries.len() < 2 {
        return Err(Error::invalid("GangBan V1/V2 缺少范围及调色板"));
    }
    let names = strings(&entries[entries.len() - 1]["list"])?;
    let range = &entries[entries.len() - 2];
    let mut output = StructureData::new(format);
    let has_range = range["start"].is_array() && range["end"].is_array();
    if !has_range && format != FormatId::GangBanV2 {
        return Err(Error::invalid("GangBan V1 缺少声明范围"));
    }
    if has_range {
        declared(
            &mut output,
            triple(&range["start"])?,
            triple(&range["end"])?,
        )?;
    }
    let suffix = if has_range { 2 } else { 1 };
    for entry in &entries[..entries.len() - suffix] {
        let aux = entry.get("aux").cloned().unwrap_or(json!(0));
        let mut block = Block::new(palette_state(&names, &entry["id"], &aux)?);
        let pos = local(triple(&entry["p"])?, output.origin)?;
        if has_range
            && (pos.0 < 0
                || pos.1 < 0
                || pos.2 < 0
                || pos.0 as u32 >= output.size.0
                || pos.1 as u32 >= output.size.1
                || pos.2 as u32 >= output.size.2)
        {
            return Err(Error::invalid("GangBan 方块超出声明范围"));
        }
        if entry["cmds"].is_object() {
            block.nbt = Some(command_v1(&entry["cmds"]));
        }
        insert(&mut output, pos, block, false);
    }
    if has_range {
        Ok(output)
    } else {
        finish(output)
    }
}

fn command_v1(value: &Value) -> Compound {
    let mut output = BTreeMap::from([
        ("id".into(), Nbt::String("CommandBlock".into())),
        (
            "Command".into(),
            Nbt::String(value["cmd"].as_str().unwrap_or_default().into()),
        ),
        (
            "CustomName".into(),
            Nbt::String(value["name"].as_str().unwrap_or_default().into()),
        ),
        (
            "LastOutput".into(),
            Nbt::String(value["last"].as_str().unwrap_or_default().into()),
        ),
        ("TickDelay".into(), Nbt::Int(integer_or(&value["tick"], 0))),
        ("ExecuteOnFirstTick".into(), byte_bool(&value["auto"])),
        ("TrackOutput".into(), byte_bool(&value["should"])),
        ("conditionalMode".into(), byte_bool(&value["condition"])),
        ("auto".into(), byte_bool(&value["auto"])),
        ("Powered".into(), byte_bool(&value["on"])),
    ]);
    let mode = match value["mode"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "tick" | "impulse" | "normal" => Some(0),
        "repeating" | "repeat" => Some(1),
        "chain" => Some(2),
        _ => None,
    };
    if let Some(mode) = mode {
        output.insert("LPCommandMode".into(), Nbt::Int(mode));
    }
    output
}

fn parse_palette(raw: &str) -> Result<Vec<String>> {
    let mut values = BTreeMap::<usize, String>::new();
    let mut rest = raw;
    while let Some(start) = rest.find('[') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find(']') else {
            break;
        };
        let id = rest[..end]
            .parse::<usize>()
            .map_err(|_| Error::invalid("GangBan 调色板标记无效"))?;
        rest = &rest[end + 1..];
        let marker = format!("[{id}]");
        let end = rest
            .find(&marker)
            .ok_or_else(|| Error::invalid("GangBan 调色板缺少闭合标记"))?;
        values.insert(id, rest[..end].to_owned());
        rest = &rest[end + marker.len()..];
    }
    if values.is_empty() {
        return Err(Error::invalid("GangBan 调色板为空"));
    }
    let mut output = Vec::new();
    for (index, name) in values {
        if index != output.len() {
            return Err(Error::invalid("GangBan 调色板索引不连续"));
        }
        output.push(name);
    }
    Ok(output)
}

fn decode_chunks(format: FormatId, entries: &[Value]) -> Result<StructureData> {
    if entries.len() < 3 {
        return Err(Error::invalid("GangBan V3/V4 缺少头部或区块"));
    }
    let names = if format == FormatId::GangBanV3 {
        parse_palette(text(&entries[1])?)?
    } else {
        strings(&entries[1])?
    };
    let mut output = StructureData::new(format);
    if format == FormatId::GangBanV3 {
        let header = &entries[0];
        let dimensions = [
            integer(&header["xcha"])?,
            integer(&header["ycha"])?,
            integer(&header["zcha"])?,
        ];
        if dimensions.iter().any(|v| *v <= 0) {
            return Err(Error::invalid("GangBan V3 尺寸必须为正"));
        }
        output.size = Size(
            dimensions[0] as u32,
            dimensions[1] as u32,
            dimensions[2] as u32,
        );
        output.origin = Pos(
            integer_or(&header["x"], 0),
            integer_or(&header["y"], 0),
            integer_or(&header["z"], 0),
        );
    }
    for chunk in &entries[2..] {
        let x = integer_or(&chunk["grids"]["x"], 0);
        let z = integer_or(&chunk["grids"]["z"], 0);
        for entry in array(&chunk["data"])? {
            let values = array(entry)?;
            if values.len() < 5 {
                return Err(Error::invalid("GangBan 区块方块条目长度不足"));
            }
            let mut pos = Pos(
                add(x, integer(&values[2])?)?,
                integer(&values[3])?,
                add(z, integer(&values[4])?)?,
            );
            let mut block = Block::new(palette_state(&names, &values[0], &values[1])?);
            if values.len() >= 7 && values[5].as_str() == Some("nbt") {
                block.nbt = values[6].as_str().and_then(|raw| snbt(raw).ok());
            }
            if format == FormatId::GangBanV3 {
                pos = local(pos, output.origin)?;
            }
            insert(&mut output, pos, block, format == FormatId::GangBanV3);
        }
    }
    if format == FormatId::GangBanV3 {
        Ok(output)
    } else {
        finish(output)
    }
}

fn decode_stream(format: FormatId, entries: &[Value]) -> Result<StructureData> {
    let suffix = if format == FormatId::GangBanV5 { 2 } else { 1 };
    if entries.len() < suffix {
        return Err(Error::invalid("GangBan 流缺少调色板"));
    }
    let names = strings(&entries[entries.len() - 1])?;
    if format == FormatId::GangBanV5 {
        triple(&entries[entries.len() - 2]["ep"])?;
    }
    let mut output = StructureData::new(format);
    let stream = &entries[..entries.len() - suffix];
    if format == FormatId::GangBanV5 {
        let mut index = 0;
        while index < stream.len() {
            if stream.len() - index < 6 {
                return Err(Error::invalid("GangBan V5 存在悬空方块条目"));
            }
            let pos = Pos(
                integer(&stream[index + 1])?,
                integer(&stream[index + 2])?,
                integer(&stream[index + 3])?,
            );
            let primary = integer(&stream[index + 4])?;
            let secondary = integer(&stream[index + 5])?;
            index += 6;
            let payload = stream.get(index).filter(|v| v.is_object() || v.is_array());
            let block = stream_block(&names, primary, secondary, payload)?;
            if payload.is_some() {
                index += 1;
            }
            insert(&mut output, pos, block, true);
        }
    } else {
        let mut pos = Pos(0, 0, 0);
        for entry in stream {
            let values = array(entry)?;
            if values.len() < 5 {
                return Err(Error::invalid("GangBan 差分方块条目长度不足"));
            }
            if values[3].is_string() && values[4].is_string() {
                continue;
            }
            pos = Pos(
                add(pos.0, integer(&values[0])?)?,
                add(pos.1, integer(&values[1])?)?,
                add(pos.2, integer(&values[2])?)?,
            );
            let block = stream_block(
                &names,
                integer(&values[3])?,
                integer(&values[4])?,
                values.get(5),
            )?;
            insert(&mut output, pos, block, true);
        }
    }
    finish(output)
}

fn stream_block(
    names: &[String],
    primary: i32,
    secondary: i32,
    payload: Option<&Value>,
) -> Result<Block> {
    if let Some(payload) = payload.filter(|value| value.is_object()) {
        let command_names = [
            "command_block",
            "repeating_command_block",
            "chain_command_block",
        ];
        let name = command_names
            .get(secondary as usize)
            .unwrap_or(&"command_block");
        let mut block = Block::new(blocks::legacy(name, primary as u16));
        let mut nbt = command_v1(&json!({
            "cmd":payload["cmd"].as_str().unwrap_or_default(),
            "name":payload["name"].as_str().unwrap_or_default(),
            "tick":integer_or(&payload["delay"],0),"auto":payload["auto"],
            "condition":payload["condition"],"should":false,"on":false,"last":""
        }));
        nbt.insert("LPCommandMode".into(), Nbt::Int(secondary));
        block.nbt = Some(nbt);
        return Ok(block);
    }
    let name = names
        .get(primary as usize)
        .ok_or_else(|| Error::invalid("GangBan 调色板索引越界"))?;
    let mut block = Block::new(blocks::legacy(name, secondary as u16));
    if let Some(items) = payload.and_then(Value::as_array) {
        let mut nbt = Compound::new();
        if let Some(id) = chest_id(name) {
            nbt.insert("id".into(), Nbt::String(id.into()));
        }
        let items: Vec<Nbt> = items
            .iter()
            .filter(|item| item.is_object())
            .map(|item| {
                Nbt::Compound(BTreeMap::from([
                    (
                        "Name".into(),
                        Nbt::String(item["ns"].as_str().unwrap_or_default().into()),
                    ),
                    (
                        "Damage".into(),
                        Nbt::Short(integer_or(&item["aux"], 0) as i16),
                    ),
                    ("Count".into(), Nbt::Byte(integer_or(&item["num"], 0) as i8)),
                    ("Slot".into(), Nbt::Byte(integer_or(&item["slot"], 0) as i8)),
                ]))
            })
            .collect();
        if !items.is_empty() {
            nbt.insert("Items".into(), Nbt::List(items));
        }
        if !nbt.is_empty() {
            block.nbt = Some(nbt);
        }
    }
    Ok(block)
}

fn command_payload(nbt: &Compound, old: bool) -> Value {
    if old {
        let mode = match nbt_number(nbt, "LPCommandMode") {
            1 => "repeating",
            2 => "chain",
            _ => "impulse",
        };
        json!({
            "mode":mode,"cmd":nbt_text(nbt,"Command"),"name":nbt_text(nbt,"CustomName"),
            "last":nbt_text(nbt,"LastOutput"),"tick":nbt_number(nbt,"TickDelay"),
            "auto":nbt_number(nbt,"auto")!=0,"condition":nbt_number(nbt,"conditionalMode")!=0,
            "should":nbt_number(nbt,"TrackOutput")!=0,"on":nbt_number(nbt,"Powered")!=0
        })
    } else {
        json!({
            "cmd":nbt_text(nbt,"Command"),"name":nbt_text(nbt,"CustomName"),
            "delay":nbt_number(nbt,"TickDelay"),"auto":nbt_number(nbt,"auto")!=0,
            "condition":nbt_number(nbt,"conditionalMode")!=0
        })
    }
}

fn stream_values(block: &Block, indices: &BTreeMap<String, usize>) -> (usize, u16, Option<Value>) {
    let (name, aux) = blocks::to_legacy(&block.state);
    if name.contains("command_block")
        && let Some(nbt) = &block.nbt
    {
        let variant = if name.contains("repeating") {
            1
        } else if name.contains("chain") {
            2
        } else {
            0
        };
        return (aux as usize, variant, Some(command_payload(nbt, false)));
    }
    let mut items = block
        .nbt
        .as_ref()
        .and_then(|nbt| nbt.get("Items"))
        .and_then(Nbt::as_list)
        .map(|items| {
            Value::Array(
                items
                    .iter()
                    .filter_map(Nbt::as_compound)
                    .map(|item| {
                        json!({"ns":nbt_text(item,"Name"),"aux":nbt_number(item,"Damage"),
                "num":nbt_number(item,"Count"),"slot":nbt_number(item,"Slot")})
                    })
                    .collect(),
            )
        });
    if items.is_none() && block.nbt.is_some() && chest_id(&name).is_some() {
        items = Some(json!([]));
    }
    (indices[&name], aux, items)
}

pub(super) fn encode(format: FormatId, data: &StructureData) -> Result<Vec<u8>> {
    let (names, indices) = palette(data);
    let mut entries = Vec::new();
    match format {
        FormatId::GangBanV1 | FormatId::GangBanV2 => {
            for (pos, block) in &data.blocks {
                let Pos(x, y, z) = world(data, *pos)?;
                let (name, aux) = blocks::to_legacy(&block.state);
                let mut entry = json!({"id":indices[&name],"aux":aux,"p":[x,y,z]});
                if let Some(nbt) = &block.nbt
                    && name.contains("command_block")
                {
                    entry["cmds"] = command_payload(nbt, true);
                }
                entries.push(entry);
            }
            let end = world(
                data,
                Pos(
                    data.size.0.saturating_sub(1) as i32,
                    data.size.1.saturating_sub(1) as i32,
                    data.size.2.saturating_sub(1) as i32,
                ),
            )?;
            entries.push(json!({"start":[data.origin.0,data.origin.1,data.origin.2],"end":[end.0,end.1,end.2]}));
            entries.push(json!({"list":names}));
        }
        FormatId::GangBanV3 | FormatId::GangBanV4 => {
            entries.push(
                json!({"x":data.origin.0,"y":data.origin.1,"z":data.origin.2,
                "xcha":data.size.0,"ycha":data.size.1,"zcha":data.size.2}),
            );
            if format == FormatId::GangBanV3 {
                let raw = names
                    .iter()
                    .enumerate()
                    .map(|(id, name)| format!("[{id}]{name}[{id}]"))
                    .collect::<String>();
                entries.push(json!(raw));
            } else {
                entries.push(json!(names));
            }
            let mut chunks = BTreeMap::<(i32, i32), Vec<Value>>::new();
            for (pos, block) in &data.blocks {
                let pos = world(data, *pos)?;
                let (name, aux) = blocks::to_legacy(&block.state);
                let mut entry = vec![
                    json!(indices[&name]),
                    json!(aux),
                    json!(pos.0.rem_euclid(16)),
                    json!(pos.1),
                    json!(pos.2.rem_euclid(16)),
                ];
                if let Some(nbt) = &block.nbt {
                    entry.extend([json!("nbt"), json!(nbt_string(nbt))]);
                }
                chunks
                    .entry((pos.0.div_euclid(16) * 16, pos.2.div_euclid(16) * 16))
                    .or_default()
                    .push(json!(entry));
            }
            for (id, ((x, z), blocks)) in chunks.into_iter().enumerate() {
                entries.push(json!({"id":id,"grids":{"x":x,"z":z,"x1":i64::from(x)+15,"z1":i64::from(z)+15},"data":blocks}));
            }
        }
        _ => {
            let mut previous = Pos(0, 0, 0);
            for (pos, block) in &data.blocks {
                let pos = world(data, *pos)?;
                let (primary, secondary, payload) = stream_values(block, &indices);
                if format == FormatId::GangBanV5 {
                    entries.extend([
                        json!(0),
                        json!(pos.0),
                        json!(pos.1),
                        json!(pos.2),
                        json!(primary),
                        json!(secondary),
                    ]);
                    if let Some(payload) = payload {
                        entries.push(payload);
                    }
                } else {
                    let delta = local(pos, previous)?;
                    let mut entry = vec![
                        json!(delta.0),
                        json!(delta.1),
                        json!(delta.2),
                        json!(primary),
                        json!(secondary),
                    ];
                    if let Some(payload) = payload {
                        entry.push(payload);
                    }
                    entries.push(json!(entry));
                }
                previous = pos;
            }
            if format == FormatId::GangBanV5 {
                entries.push(json!({"ep":[data.size.0.saturating_sub(1),data.size.1.saturating_sub(1),data.size.2.saturating_sub(1)]}));
            }
            entries.push(json!(names));
        }
    }
    let plain = serde_json::to_vec(&entries)?;
    if format == FormatId::GangBanV7 {
        zlib_encode(&plain)
    } else {
        Ok(plain)
    }
}
