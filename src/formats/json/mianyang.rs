use super::*;
use crate::Size;
use flate2::{read::GzDecoder, write::GzEncoder};
use std::io::Cursor;

pub(super) fn decode(format: FormatId, bytes: &[u8]) -> Result<StructureData> {
    if format == FormatId::MianYangV4 {
        return decode_buildingx(bytes);
    }
    let plain = if format == FormatId::MianYangV3 {
        zlib_decode(bytes)?
    } else {
        bytes.to_vec()
    };
    let root: Value = serde_json::from_slice(&plain)?;
    let names = strings(&root["namespaces"])?;
    let mut output = StructureData::new(format);
    for chunk in array(&root["chunkedBlocks"])? {
        let start_x = integer_or(&chunk["startX"], 0);
        let start_z = integer_or(&chunk["startZ"], 0);
        for entry in array(&chunk["blocks"])? {
            let values = array(entry)?;
            if values.len() < 5 {
                return Err(Error::invalid("MianYang 方块条目长度不足"));
            }
            let pos = Pos(
                add(start_x, integer(&values[2])?)?,
                integer(&values[3])?,
                add(start_z, integer(&values[4])?)?,
            );
            let mut block = Block::new(palette_state(&names, &values[0], &values[1])?);
            if let Some(value) = values.get(5) {
                block.nbt = parse_nbt(text(value)?)?;
            }
            insert(&mut output, pos, block, false);
        }
    }
    finish(output)
}

fn query_unescape(raw: &str) -> Result<String> {
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => decoded.push(b' '),
            b'%' => {
                if index + 2 >= bytes.len() {
                    return Err(Error::invalid("NBT 百分号转义不完整"));
                }
                let hex = |b: u8| (b as char).to_digit(16).map(|v| v as u8);
                let high =
                    hex(bytes[index + 1]).ok_or_else(|| Error::invalid("NBT 百分号转义无效"))?;
                let low =
                    hex(bytes[index + 2]).ok_or_else(|| Error::invalid("NBT 百分号转义无效"))?;
                decoded.push(high * 16 + low);
                index += 2;
            }
            value => decoded.push(value),
        }
        index += 1;
    }
    String::from_utf8(decoded).map_err(|_| Error::invalid("NBT 不是有效 UTF-8"))
}

fn parse_nbt(raw: &str) -> Result<Option<Compound>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    if let Ok(value) = serde_json::from_str::<Value>(raw)
        && value.as_object().is_some_and(|m| !m.is_empty())
    {
        if let Some(encoded) = value["blockCompleteNBT"].as_str() {
            return parse_encoded_snbt(encoded).map(Some);
        }
        return compound(&value).map(Some);
    }
    parse_encoded_snbt(raw).map(Some)
}

fn parse_encoded_snbt(encoded: &str) -> Result<Compound> {
    let mut decoded = query_unescape(encoded)?.trim().to_owned();
    // 早期导出器遗漏根大括号，且命令中的引号没有正确转义。
    if let Some(start) = decoded.find("Command:\"") {
        let start = start + "Command:\"".len();
        if let Some(end) = decoded[start..].find("\",CustomName:\"") {
            let end = start + end;
            let command = serde_json::to_string(&decoded[start..end])?;
            decoded = format!(
                "{}{}{}",
                &decoded[..start - 1],
                command,
                &decoded[end + 1..]
            );
        }
    }
    if decoded.starts_with("\"\"") {
        decoded = format!("{{{decoded}}}");
    }
    if decoded.starts_with('{') || decoded.starts_with('[') {
        return snbt(&decoded);
    }
    Ok(BTreeMap::from([(String::new(), Nbt::String(decoded))]))
}

pub(super) fn encode(format: FormatId, data: &StructureData) -> Result<Vec<u8>> {
    if format == FormatId::MianYangV4 {
        return encode_buildingx(data);
    }
    let (names, indices) = palette(data);
    let mut chunks = BTreeMap::<(i32, i32), Vec<Value>>::new();
    for (pos, block) in &data.blocks {
        let pos = world(data, *pos)?;
        let (name, aux) = blocks::to_legacy(&block.state);
        let chunk_x = pos.0.div_euclid(16) * 16;
        let chunk_z = pos.2.div_euclid(16) * 16;
        let mut entry = vec![
            json!(indices[&name]),
            json!(aux),
            json!(pos.0.rem_euclid(16)),
            json!(pos.1),
            json!(pos.2.rem_euclid(16)),
        ];
        if let Some(nbt) = &block.nbt {
            // JSON 包装器中的 SNBT 不经过 URL 解码，必须百分号编码 '+' 等字符。
            let encoded = percent_encode(&nbt_string(nbt));
            entry.push(json!(json!({"blockCompleteNBT":encoded}).to_string()));
        }
        chunks
            .entry((chunk_x, chunk_z))
            .or_default()
            .push(Value::Array(entry));
    }
    let chunks: Vec<Value> = chunks
        .into_iter()
        .map(|((x, z), blocks)| json!({"startX":x,"startZ":z,"blocks":blocks}))
        .collect();
    let plain = serde_json::to_vec(
        &json!({"namespaces":names,"chunkedBlocks":chunks,"totalBlocks":data.blocks.len()}),
    )?;
    if format == FormatId::MianYangV3 {
        zlib_encode(&plain)
    } else {
        Ok(plain)
    }
}

fn percent_encode(raw: &str) -> String {
    let mut output = String::new();
    for byte in raw.as_bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(byte) {
            output.push(*byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn read_bytes<const N: usize>(cursor: &mut Cursor<&[u8]>) -> Result<[u8; N]> {
    let mut bytes = [0; N];
    cursor.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn read_i32(cursor: &mut Cursor<&[u8]>) -> Result<i32> {
    Ok(i32::from_le_bytes(read_bytes(cursor)?))
}

fn read_string(cursor: &mut Cursor<&[u8]>, length: usize) -> Result<String> {
    let remaining = cursor
        .get_ref()
        .len()
        .saturating_sub(cursor.position() as usize);
    if length > remaining {
        return Err(Error::invalid("BuildingX 字符串超出文件边界"));
    }
    let mut bytes = vec![0; length];
    cursor.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|_| Error::invalid("BuildingX 字符串不是 UTF-8"))
}

fn decode_buildingx(bytes: &[u8]) -> Result<StructureData> {
    let mut plain = Vec::new();
    GzDecoder::new(bytes).read_to_end(&mut plain)?;
    let mut cursor = Cursor::new(plain.as_slice());
    let mut output = StructureData::new(FormatId::MianYangV4);
    output.origin = Pos(
        read_i32(&mut cursor)?,
        read_i32(&mut cursor)?,
        read_i32(&mut cursor)?,
    );
    let width = read_i32(&mut cursor)?;
    let height = read_i32(&mut cursor)?;
    let length = read_i32(&mut cursor)?;
    let names_count = read_i32(&mut cursor)?;
    let block_count = read_i32(&mut cursor)?;
    if width <= 0 || height <= 0 || length <= 0 || names_count <= 0 || block_count <= 0 {
        return Err(Error::invalid("BuildingX 头部数量必须为正数"));
    }
    if names_count as usize > plain.len() / 3 || block_count as usize > plain.len() / 16 {
        return Err(Error::invalid("BuildingX 头部数量超出文件大小"));
    }
    output.size = Size(width as u32, height as u32, length as u32);
    let mut names = Vec::new();
    for _ in 0..names_count {
        let size = u16::from_le_bytes(read_bytes(&mut cursor)?) as usize;
        if size == 0 {
            return Err(Error::invalid("BuildingX 调色板名称为空"));
        }
        names.push(read_string(&mut cursor, size)?);
    }
    let mut position = Pos(0, 0, 0);
    for _ in 0..block_count {
        position.0 = add(
            position.0,
            i16::from_le_bytes(read_bytes(&mut cursor)?) as i32,
        )?;
        position.1 = add(
            position.1,
            i16::from_le_bytes(read_bytes(&mut cursor)?) as i32,
        )?;
        position.2 = add(
            position.2,
            i16::from_le_bytes(read_bytes(&mut cursor)?) as i32,
        )?;
        let [index, aux] = read_bytes::<2>(&mut cursor)?;
        let name = names
            .get(index as usize)
            .ok_or_else(|| Error::invalid("BuildingX 调色板索引越界"))?;
        let mut block = Block::new(blocks::legacy(name, aux as u16));
        let nbt_len = usize::try_from(read_i32(&mut cursor)?)
            .map_err(|_| Error::invalid("BuildingX NBT 长度为负"))?;
        if nbt_len > 0 {
            block.nbt = parse_nbt(&read_string(&mut cursor, nbt_len)?)?;
        }
        let entity_len = usize::try_from(read_i32(&mut cursor)?)
            .map_err(|_| Error::invalid("BuildingX 实体长度为负"))?;
        if entity_len > 0 {
            let entity = read_string(&mut cursor, entity_len)?;
            // 原读取器跳过实体；可以解析的 SNBT 实体保存在公共模型中。
            if let Ok(entity) = snbt(&entity) {
                output.entities.push(entity);
            }
        }
        insert(
            &mut output,
            Pos(
                position.0.rem_euclid(width),
                position.1.rem_euclid(height),
                position.2.rem_euclid(length),
            ),
            block,
            false,
        );
    }
    Ok(output)
}

fn encode_buildingx(data: &StructureData) -> Result<Vec<u8>> {
    let (names, indices) = palette(data);
    if names.len() > 256 || data.blocks.is_empty() {
        return Err(Error::invalid(
            "BuildingX 需要 1..256 个调色板名称及非空方块列表",
        ));
    }
    let dimensions = [data.size.0, data.size.1, data.size.2];
    if dimensions.iter().any(|v| *v == 0 || *v > i32::MAX as u32) {
        return Err(Error::invalid("BuildingX 尺寸超出范围"));
    }
    let mut plain = Vec::new();
    for value in [
        data.origin.0,
        data.origin.1,
        data.origin.2,
        data.size.0 as i32,
        data.size.1 as i32,
        data.size.2 as i32,
        names.len() as i32,
        i32::try_from(data.blocks.len()).map_err(|_| Error::invalid("BuildingX 方块过多"))?,
    ] {
        plain.extend(value.to_le_bytes());
    }
    for name in &names {
        let length =
            u16::try_from(name.len()).map_err(|_| Error::invalid("BuildingX 方块名称过长"))?;
        plain.extend(length.to_le_bytes());
        plain.extend(name.as_bytes());
    }
    let mut previous = Pos(0, 0, 0);
    for (pos, block) in &data.blocks {
        for (current, previous) in [
            (pos.0, previous.0),
            (pos.1, previous.1),
            (pos.2, previous.2),
        ] {
            let delta = i16::try_from(i64::from(current) - i64::from(previous))
                .map_err(|_| Error::invalid("BuildingX 相邻方块坐标差超出 int16"))?;
            plain.extend(delta.to_le_bytes());
        }
        previous = *pos;
        let (name, aux) = blocks::to_legacy(&block.state);
        plain.push(indices[&name] as u8);
        plain.push(u8::try_from(aux).map_err(|_| Error::invalid("BuildingX aux 超出 uint8"))?);
        let nbt = block
            .nbt
            .as_ref()
            .map(|nbt| json!({"blockCompleteNBT":percent_encode(&nbt_string(nbt))}).to_string())
            .unwrap_or_default();
        plain.extend(
            i32::try_from(nbt.len())
                .map_err(|_| Error::invalid("BuildingX NBT 过长"))?
                .to_le_bytes(),
        );
        plain.extend(nbt.as_bytes());
        plain.extend(0_i32.to_le_bytes());
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&plain)?;
    Ok(encoder.finish()?)
}
