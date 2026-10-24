use super::*;
use std::collections::BTreeMap;

fn seek<'a>(bytes: &'a [u8], offset: u64) -> Result<Cursor<'a>> {
    let offset = usize::try_from(offset).map_err(|_| Error::invalid("BCF 偏移溢出"))?;
    if offset == 0 || offset >= bytes.len() {
        return Err(Error::invalid("BCF 偏移越界"));
    }
    Ok(Cursor::new(&bytes[offset..]))
}

fn strings(bytes: &[u8], offset: u64, byte_key: bool) -> Result<BTreeMap<u16, String>> {
    if offset == 0 {
        return Ok(BTreeMap::new());
    }
    let mut reader = seek(bytes, offset)?;
    let count = reader.u32()?;
    if count as usize > bytes.len() {
        return Err(Error::invalid("BCF 字典过大"));
    }
    let mut result = BTreeMap::new();
    for _ in 0..count {
        let key = if byte_key {
            u16::from(reader.u8()?)
        } else {
            reader.u16()?
        };
        let length = reader.u16()? as usize;
        result.insert(key, reader.string(length)?);
    }
    Ok(result)
}

pub(super) fn decode(bytes: &[u8]) -> Result<StructureData> {
    let mut reader = Cursor::new(bytes);
    if reader.take(3)? != b"BCF" {
        return Err(Error::invalid("无效 BCF 头"));
    }
    let version = reader.u8()?;
    let width = reader.u16()?;
    let length = reader.u16()?;
    let height = reader.u16()?;
    let base_size = reader.u8()?;
    let count = reader.u64()?;
    let table_offset = reader.u64()?;
    let palette_offset = reader.u64()?;
    let types_offset = reader.u64()?;
    let names_offset = reader.u64()?;
    let values_offset = reader.u64()?;
    let types = strings(bytes, types_offset, false)?;
    let names = strings(bytes, names_offset, true)?;
    let values = strings(bytes, values_offset, true)?;
    let mut palette_reader = seek(bytes, palette_offset)?;
    let palette_count = palette_reader.u32()?;
    if palette_count as usize > bytes.len() / 8 {
        return Err(Error::invalid("BCF 调色板过大"));
    }
    let mut palette = BTreeMap::new();
    for _ in 0..palette_count {
        let id = palette_reader.u32()?;
        let type_id = palette_reader.u16()?;
        let state_count = palette_reader.u16()?;
        let name = types
            .get(&type_id)
            .ok_or_else(|| Error::invalid("BCF 类型索引越界"))?;
        let mut states = Compound::new();
        for _ in 0..state_count {
            let name_id = u16::from(palette_reader.u8()?);
            let value_id = u16::from(palette_reader.u8()?);
            let key = names
                .get(&name_id)
                .ok_or_else(|| Error::invalid("BCF 状态名索引越界"))?;
            let value = values
                .get(&value_id)
                .ok_or_else(|| Error::invalid("BCF 状态值索引越界"))?;
            let value = match value.as_str() {
                "true" => Nbt::Byte(1),
                "false" => Nbt::Byte(0),
                _ => value
                    .parse::<i32>()
                    .map(Nbt::Int)
                    .unwrap_or_else(|_| Nbt::String(value.trim_matches('"').into())),
            };
            states.insert(key.clone(), value);
        }
        palette.insert(id, crate::blocks::bedrock(name, states));
    }
    let mut offsets = seek(bytes, table_offset)?;
    let offset_count = offsets.u64()?;
    if offset_count > bytes.len() as u64 / 8 {
        return Err(Error::invalid("BCF 子区块表过大"));
    }
    let mut result = StructureData::new(FormatId::BCF);
    result
        .metadata
        .insert("version".into(), Nbt::Int(i32::from(version)));
    result.metadata.insert(
        "declared_size".into(),
        Nbt::IntArray(vec![i32::from(width), i32::from(height), i32::from(length)]),
    );
    result
        .metadata
        .insert("subchunk_base_size".into(), Nbt::Int(i32::from(base_size)));
    result
        .metadata
        .insert("subchunk_count".into(), Nbt::Long(count as i64));
    for _ in 0..offset_count {
        let offset = offsets.u64()?;
        if offset == 0 {
            continue;
        }
        let mut subchunk = seek(bytes, offset)?;
        let _size = subchunk.u64()?;
        let origin = Pos(
            i32::from(subchunk.u16()? as i16),
            i32::from(subchunk.u16()? as i16),
            i32::from(subchunk.u16()? as i16),
        );
        let count = subchunk.u32()?;
        if count as usize > bytes.len() / 16 {
            return Err(Error::invalid("BCF 区域数量过大"));
        }
        for _ in 0..count {
            let id = subchunk.u32()?;
            let coordinate = |reader: &mut Cursor<'_>| -> Result<Pos> {
                Ok(Pos(
                    origin.0 + i32::from(reader.u16()? as i16),
                    origin.1 + i32::from(reader.u16()? as i16),
                    origin.2 + i32::from(reader.u16()? as i16),
                ))
            };
            let start = coordinate(&mut subchunk)?;
            let end = coordinate(&mut subchunk)?;
            let state = palette
                .get(&id)
                .ok_or_else(|| Error::invalid("BCF 调色板索引越界"))?;
            fill(&mut result, start, end, Block::new(state.clone()))?;
        }
    }
    if result.blocks.is_empty() {
        return Err(Error::invalid("BCF 无有效区域"));
    }
    result.normalize()?;
    Ok(result)
}

fn write_string(out: &mut Vec<u8>, value: &str) -> Result<()> {
    let length = u16::try_from(value.len()).map_err(|_| Error::invalid("BCF 字符串过长"))?;
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn insert_id(map: &mut BTreeMap<String, u16>, key: String, maximum: usize) -> Result<u16> {
    if let Some(id) = map.get(&key) {
        return Ok(*id);
    }
    if map.len() >= maximum {
        return Err(Error::invalid("BCF 字典条目超过格式上限"));
    }
    let id = map.len() as u16;
    map.insert(key, id);
    Ok(id)
}

fn write_map(out: &mut Vec<u8>, map: BTreeMap<String, u16>, byte_key: bool) -> Result<u64> {
    let offset = out.len() as u64;
    out.extend_from_slice(&(map.len() as u32).to_le_bytes());
    for (text, id) in map {
        if byte_key {
            out.push(id as u8);
        } else {
            out.extend_from_slice(&id.to_le_bytes());
        }
        write_string(out, &text)?;
    }
    Ok(offset)
}

pub(super) fn encode(data: &StructureData) -> Result<Vec<u8>> {
    let mut out = b"BCF".to_vec();
    out.push(
        data.metadata
            .get("version")
            .and_then(Nbt::as_i64)
            .unwrap_or(3) as u8,
    );
    for dimension in [data.size.0, data.size.2, data.size.1] {
        out.extend_from_slice(
            &u16::try_from(dimension)
                .map_err(|_| Error::invalid("BCF 尺寸超过65535"))?
                .to_le_bytes(),
        );
    }
    out.push(16);
    out.extend_from_slice(&0u64.to_le_bytes());
    out.resize(59, 0);
    let mut types = BTreeMap::new();
    let mut names = BTreeMap::new();
    let mut values = BTreeMap::new();
    let mut palette = BTreeMap::<String, (u32, u16, Vec<(u16, u16)>)>::new();
    let mut subchunks = BTreeMap::<Pos, Vec<(Pos, u32)>>::new();
    for (pos, block) in &data.blocks {
        let pos = world_pos(data, *pos)?;
        let key = block.state.string();
        if !palette.contains_key(&key) {
            let type_id = insert_id(&mut types, block.state.name.clone(), 65536)?;
            let mut pairs = Vec::new();
            for (name, value) in &block.state.properties {
                let name_id = insert_id(&mut names, name.clone(), 256)?;
                let value = match value {
                    Nbt::Byte(value) if *value == 0 || *value == 1 => (*value != 0).to_string(),
                    Nbt::String(value) => serde_json::to_string(value)?,
                    value => value.scalar_string(),
                };
                pairs.push((name_id, insert_id(&mut values, value, 256)?));
            }
            palette.insert(key.clone(), (palette.len() as u32, type_id, pairs));
        }
        let origin_axis = |coordinate: i32| {
            if coordinate > i32::from(i16::MAX) {
                i32::from(i16::MAX)
            } else if coordinate < i32::from(i16::MIN) {
                i32::from(i16::MIN)
            } else {
                0
            }
        };
        let origin = Pos(origin_axis(pos.0), origin_axis(pos.1), origin_axis(pos.2));
        subchunks
            .entry(origin)
            .or_default()
            .push((pos, palette[&key].0));
    }
    out[11..19].copy_from_slice(&(subchunks.len() as u64).to_le_bytes());
    let mut offsets = Vec::new();
    for (origin, records) in subchunks {
        let size_at = out.len();
        offsets.push(size_at as u64);
        out.extend_from_slice(&0u64.to_le_bytes());
        for coordinate in [origin.0, origin.1, origin.2] {
            out.extend_from_slice(&(coordinate as i16).to_le_bytes());
        }
        let count = u32::try_from(records.len()).map_err(|_| Error::invalid("BCF 区域数量过大"))?;
        out.extend_from_slice(&count.to_le_bytes());
        for (pos, id) in records {
            out.extend_from_slice(&id.to_le_bytes());
            for _ in 0..2 {
                for (coordinate, origin) in
                    [(pos.0, origin.0), (pos.1, origin.1), (pos.2, origin.2)]
                {
                    let local = i16::try_from(coordinate - origin)
                        .map_err(|_| Error::invalid("BCF 世界坐标超出格式可表示范围"))?;
                    out.extend_from_slice(&local.to_le_bytes());
                }
            }
        }
        let size = (out.len() - size_at) as u64;
        out[size_at..size_at + 8].copy_from_slice(&size.to_le_bytes());
    }
    let table_offset = out.len() as u64;
    out.extend_from_slice(&(offsets.len() as u64).to_le_bytes());
    for offset in offsets {
        out.extend_from_slice(&offset.to_le_bytes());
    }
    let palette_offset = out.len() as u64;
    out.extend_from_slice(&(palette.len() as u32).to_le_bytes());
    for (_, (id, type_id, pairs)) in palette {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&type_id.to_le_bytes());
        out.extend_from_slice(&(pairs.len() as u16).to_le_bytes());
        for (name, value) in pairs {
            out.extend_from_slice(&[name as u8, value as u8]);
        }
    }
    let type_offset = write_map(&mut out, types, false)?;
    let names_offset = write_map(&mut out, names, true)?;
    let values_offset = write_map(&mut out, values, true)?;
    for (index, offset) in [
        table_offset,
        palette_offset,
        type_offset,
        names_offset,
        values_offset,
    ]
    .into_iter()
    .enumerate()
    {
        let start = 19 + index * 8;
        out[start..start + 8].copy_from_slice(&offset.to_le_bytes());
    }
    Ok(out)
}
