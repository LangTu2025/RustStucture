use super::*;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use serde_json::{Value, json};

pub(super) fn coordinate(text: &str) -> Result<i32> {
    let text = text.strip_prefix('~').unwrap_or(text);
    if text.is_empty() {
        return Ok(0);
    }
    text.parse()
        .map_err(|_| Error::invalid(format!("无效坐标: {text}")))
}

pub(super) fn state(name: &str, properties: &str) -> BlockState {
    let properties = properties.trim();
    if let Ok(value) = properties.parse::<u16>() {
        return crate::blocks::legacy(name, value);
    }
    // 复用共享转换表，兼容旧方块别名以及冒号/等号两种状态字符串。
    crate::blocks::java(&format!("{name}{}", properties.replace("\\\"", "\"")))
}

fn parse_script(script: &str, format: FormatId) -> Result<StructureData> {
    let mut result = StructureData::new(format);
    for (line_number, source) in script.lines().enumerate() {
        let source = source.trim().trim_start_matches('/');
        if source.is_empty() || source.starts_with('#') {
            continue;
        }
        let (line, properties) =
            if let (Some(start), Some(end)) = (source.find('['), source.rfind(']')) {
                if end <= start {
                    return Err(Error::invalid("无效方块状态"));
                }
                (
                    format!("{} {}", &source[..start], &source[end + 1..]),
                    &source[start..=end],
                )
            } else {
                (source.to_owned(), "")
            };
        let fields: Vec<_> = line.split_whitespace().collect();
        let Some(command) = fields.first() else {
            continue;
        };
        let (count, block_index) = match command.to_ascii_lowercase().as_str() {
            "setblock" if fields.len() >= 5 => (3, 4),
            "fill" if fields.len() >= 8 => (6, 7),
            _ => continue,
        };
        let coords = fields[1..=count]
            .iter()
            .map(|value| coordinate(value))
            .collect::<Result<Vec<_>>>()
            .map_err(|e| Error::invalid(format!("第 {} 行: {e}", line_number + 1)))?;
        let start = Pos(coords[0], coords[1], coords[2]);
        let end = if count == 6 {
            Pos(coords[3], coords[4], coords[5])
        } else {
            start
        };
        let properties = if properties.is_empty() {
            fields
                .get(block_index + 1)
                .copied()
                .filter(|s| s.parse::<u16>().is_ok())
                .unwrap_or("")
        } else {
            properties
        };
        fill(
            &mut result,
            start,
            end,
            Block::new(state(fields[block_index], properties)),
        )?;
    }
    if result.blocks.is_empty() {
        return Err(Error::invalid("指令文件不包含方块"));
    }
    result.normalize()?;
    Ok(result)
}

pub(super) fn decode(bytes: &[u8]) -> Result<StructureData> {
    let script = std::str::from_utf8(bytes).map_err(|e| Error::invalid(e.to_string()))?;
    parse_script(script.trim_start_matches('\u{feff}'), FormatId::MCFunction)
}

pub(super) fn bedrock_states(state: &BlockState) -> String {
    let pairs = state
        .properties
        .iter()
        .map(|(key, value)| {
            let value = match value {
                Nbt::String(text) => serde_json::to_string(text).unwrap(),
                Nbt::Byte(value) if *value == 0 || *value == 1 => (*value != 0).to_string(),
                value => value.scalar_string(),
            };
            format!("{}={value}", serde_json::to_string(key).unwrap())
        })
        .collect::<Vec<_>>();
    format!("[{}]", pairs.join(","))
}

pub(super) fn encode(data: &StructureData) -> String {
    let mut output = String::new();
    for (pos, block) in &data.blocks {
        // 结构坐标恢复为来源坐标，允许负坐标。
        let x = i64::from(pos.0) + i64::from(data.origin.0);
        let y = i64::from(pos.1) + i64::from(data.origin.1);
        let z = i64::from(pos.2) + i64::from(data.origin.2);
        output.push_str(&format!(
            "setblock ~{x} ~{y} ~{z} {} {}\n",
            block.state.name,
            bedrock_states(&block.state)
        ));
    }
    output
}

fn boolean(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_i64().unwrap_or(0) != 0,
        Some(Value::String(value)) => matches!(value.as_str(), "true" | "1"),
        _ => false,
    }
}

fn decode_command(text: &str) -> String {
    for engine in [&STANDARD, &STANDARD_NO_PAD] {
        if let Ok(bytes) = engine.decode(text)
            && engine.encode(&bytes) == text
            && let Ok(value) = String::from_utf8(bytes)
        {
            return value;
        }
    }
    text.to_owned()
}

pub(super) fn decode_ibi(bytes: &[u8]) -> Result<StructureData> {
    let mut reader = Cursor::new(bytes);
    if reader.take(9)? != b"IBImport " {
        return Err(Error::invalid("无效 IBImport 头"));
    }
    let mut segments = Vec::new();
    while reader.pos < bytes.len() {
        let length = usize::try_from(reader.varint()?).map_err(|_| Error::invalid("段过长"))?;
        let key = reader.u8()?;
        segments.push(
            reader
                .take(length)?
                .iter()
                .map(|b| *b ^ key)
                .collect::<Vec<_>>(),
        );
    }
    let script = segments
        .first()
        .ok_or_else(|| Error::invalid("IBImport 缺少指令段"))?;
    let script = std::str::from_utf8(script).map_err(|e| Error::invalid(e.to_string()))?;
    let mut result = parse_script(script, FormatId::IBImport)?;
    if let Some(commands) = segments.get(1) {
        let commands: Vec<Value> = serde_json::from_slice(commands)?;
        for command in commands {
            let coord = |key: &str| coordinate(command[key].as_str().unwrap_or("~"));
            let pos = Pos(
                coord("posX")? - result.origin.0,
                coord("posY")? - result.origin.1,
                coord("posZ")? - result.origin.2,
            );
            let Some(block) = result.blocks.get_mut(&pos) else {
                continue;
            };
            let mut nbt = Compound::new();
            nbt.insert("id".into(), Nbt::String("CommandBlock".into()));
            nbt.insert(
                "Command".into(),
                Nbt::String(decode_command(
                    command["CommandMessage"].as_str().unwrap_or(""),
                )),
            );
            nbt.insert(
                "CustomName".into(),
                Nbt::String(command["Commandtitle"].as_str().unwrap_or("").into()),
            );
            nbt.insert(
                "TickDelay".into(),
                Nbt::Int(command["isTime"].as_i64().unwrap_or(0) as i32),
            );
            nbt.insert(
                "conditionalMode".into(),
                Nbt::Byte(boolean(command.get("Conditional")) as i8),
            );
            nbt.insert(
                "auto".into(),
                Nbt::Byte((!boolean(command.get("isRedstone"))) as i8),
            );
            nbt.insert("Version".into(), Nbt::Int(38));
            nbt.insert("ExecuteOnFirstTick".into(), Nbt::Byte(0));
            nbt.insert("TrackOutput".into(), Nbt::Byte(0));
            nbt.insert("LastOutput".into(), Nbt::String(String::new()));
            block.nbt = Some(nbt);
        }
    }
    Ok(result)
}

pub(super) fn encode_ibi(data: &StructureData) -> Result<Vec<u8>> {
    let mut commands = Vec::new();
    for (pos, block) in &data.blocks {
        let Some(nbt) = &block.nbt else {
            continue;
        };
        let Some(command) = nbt.get("Command").and_then(Nbt::as_str) else {
            continue;
        };
        let pos = world_pos(data, *pos)?;
        let integer = |key: &str| nbt.get(key).and_then(Nbt::as_i64).unwrap_or(0);
        let mode = if block.state.name.contains("repeating") {
            1
        } else if block.state.name.contains("chain") {
            2
        } else {
            0
        };
        commands.push(json!({
            "posX": format!("~{}", pos.0), "posY": format!("~{}", pos.1), "posZ": format!("~{}", pos.2),
            "CommandMessage": STANDARD.encode(command), "Commandtitle": nbt.get("CustomName").and_then(Nbt::as_str).unwrap_or(""),
            "mode": mode, "isTime": integer("TickDelay"), "Conditional": integer("conditionalMode"), "isRedstone": integer("auto") == 0
        }));
    }
    let mut output = b"IBImport ".to_vec();
    for segment in [encode(data).into_bytes(), serde_json::to_vec(&commands)?] {
        put_varint(&mut output, segment.len() as u64);
        output.push(0x5a);
        output.extend(segment.into_iter().map(|byte| byte ^ 0x5a));
    }
    Ok(output)
}
