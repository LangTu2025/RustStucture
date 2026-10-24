use super::*;
use md5::{Digest, Md5};
use std::collections::BTreeMap;

fn xor_payload(bytes: &mut [u8]) -> Result<()> {
    if bytes.len() < 15 {
        return Err(Error::invalid("TIBI 缺少15字节头"));
    }
    let mut digest = Md5::new();
    digest.update(&bytes[..15]);
    digest.update(format!("TIBI_2025/5/19-Start{}", bytes.len() - 15).as_bytes());
    let key = digest.finalize();
    for (index, byte) in bytes[15..].iter_mut().enumerate() {
        *byte ^= key[index % key.len()];
    }
    Ok(())
}

fn dictionary(reader: &mut Cursor<'_>) -> Result<Vec<String>> {
    let count = reader.varint()?;
    if count > reader.data.len() as u64 {
        return Err(Error::invalid("TIBI 字典数量错误"));
    }
    let mut result = Vec::with_capacity(count as usize);
    for _ in 0..count {
        reader.varint()?;
        result.push(reader.varstring()?);
    }
    Ok(result)
}

pub(super) fn decode(bytes: &[u8]) -> Result<StructureData> {
    let mut inflated = Vec::new();
    flate2::read::DeflateDecoder::new(bytes).read_to_end(&mut inflated)?;
    xor_payload(&mut inflated)?;
    let mut reader = Cursor::new(&inflated[15..]);
    let blocks = dictionary(&mut reader)?;
    let properties = dictionary(&mut reader)?;
    let count = reader.varint()?;
    if count > reader.data.len() as u64 {
        return Err(Error::invalid("TIBI 指令数量错误"));
    }
    let mut result = StructureData::new(FormatId::TIBI);
    result.metadata.insert(
        "header".into(),
        Nbt::ByteArray(inflated[..15].iter().map(|b| *b as i8).collect()),
    );
    for _ in 0..count {
        let kind = reader.varint()?;
        let index = reader.varint()? as usize;
        let start = Pos(
            reader.varint()? as i32,
            reader.varint()? as i32,
            reader.varint()? as i32,
        );
        let end = if kind == 1 {
            Pos(
                reader.varint()? as i32,
                reader.varint()? as i32,
                reader.varint()? as i32,
            )
        } else if kind == 0 {
            start
        } else {
            return Err(Error::invalid("未知 TIBI 指令"));
        };
        let prop_index = reader.varint()? as usize;
        let name = blocks
            .get(index)
            .ok_or_else(|| Error::invalid("TIBI 方块索引越界"))?;
        let property = properties
            .get(prop_index)
            .ok_or_else(|| Error::invalid("TIBI 状态索引越界"))?;
        let (name, states) = if let Some(index) = name.find('[') {
            (&name[..index], &name[index..])
        } else {
            (name.as_str(), "")
        };
        let property = property.split_whitespace().next().unwrap_or("");
        let states = if property.parse::<u16>().is_ok() {
            property
        } else {
            states
        };
        fill(
            &mut result,
            start,
            end,
            Block::new(super::text::state(name, states)),
        )?;
    }
    if result.blocks.is_empty() {
        return Err(Error::invalid("TIBI 无有效方块"));
    }
    result.normalize()?;
    Ok(result)
}

pub(super) fn encode(data: &StructureData) -> Result<Vec<u8>> {
    let mut dictionary = BTreeMap::<String, usize>::new();
    let mut commands = Vec::new();
    for (pos, block) in &data.blocks {
        let name = format!(
            "{}{}",
            block.state.name,
            super::text::bedrock_states(&block.state)
        );
        let next = dictionary.len();
        let index = *dictionary.entry(name).or_insert(next);
        commands.push((world_pos(data, *pos)?, index));
    }
    let mut payload = Vec::new();
    put_varint(&mut payload, dictionary.len() as u64);
    let mut ordered: Vec<_> = dictionary.into_iter().collect();
    ordered.sort_by_key(|(_, index)| *index);
    for (name, index) in ordered {
        put_varint(&mut payload, index as u64);
        put_varstring(&mut payload, &name);
    }
    put_varint(&mut payload, 1);
    put_varint(&mut payload, 0);
    put_varstring(&mut payload, "");
    put_varint(&mut payload, commands.len() as u64);
    for (pos, index) in commands {
        put_varint(&mut payload, 0);
        put_varint(&mut payload, index as u64);
        for coordinate in [pos.0, pos.1, pos.2] {
            put_varint(&mut payload, coordinate as i64 as u64);
        }
        put_varint(&mut payload, 0);
    }
    let mut clear = match data.metadata.get("header") {
        Some(Nbt::ByteArray(header)) if header.len() == 15 => {
            header.iter().map(|b| *b as u8).collect::<Vec<_>>()
        }
        _ => b"TIBI RustCodec!".to_vec(),
    };
    clear.extend(payload);
    xor_payload(&mut clear)?;
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&clear)?;
    Ok(encoder.finish()?)
}
