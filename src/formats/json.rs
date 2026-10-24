//! 基于原 WaterStructure 读取器的 JSON 家族格式。
mod fuhong;
mod gangban;
mod mianyang;
mod simple;

use crate::nbt::{self, Compound, Nbt};
use crate::{Block, BlockState, Error, FormatId, Pos, Result, StructureData, blocks};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{Read, Write};

pub fn decode(format: FormatId, bytes: &[u8]) -> Result<StructureData> {
    use FormatId::*;
    match format {
        MianYangV1 | MianYangV2 | MianYangV3 | MianYangV4 => mianyang::decode(format, bytes),
        GangBanV1 | GangBanV2 | GangBanV3 | GangBanV4 | GangBanV5 | GangBanV6 | GangBanV7 => {
            gangban::decode(format, bytes)
        }
        FuHongV1 | FuHongV2 | FuHongV3 | FuHongV4 | FuHongV5 => fuhong::decode(format, bytes),
        RunAway | QingXuV1 | TimeBuilderV1 | CovStructure => simple::decode(format, bytes),
        _ => Err(Error::Unsupported(format!(
            "{} 不属于 JSON 格式",
            format.name()
        ))),
    }
}

pub fn encode(format: FormatId, data: &StructureData) -> Result<Vec<u8>> {
    use FormatId::*;
    match format {
        MianYangV1 | MianYangV2 | MianYangV3 | MianYangV4 => mianyang::encode(format, data),
        GangBanV1 | GangBanV2 | GangBanV3 | GangBanV4 | GangBanV5 | GangBanV6 | GangBanV7 => {
            gangban::encode(format, data)
        }
        FuHongV1 | FuHongV2 | FuHongV3 | FuHongV4 | FuHongV5 => fuhong::encode(format, data),
        RunAway | QingXuV1 | TimeBuilderV1 | CovStructure => simple::encode(format, data),
        _ => Err(Error::Unsupported(format!(
            "{} 不属于 JSON 格式",
            format.name()
        ))),
    }
}

fn array(value: &Value) -> Result<&Vec<Value>> {
    value
        .as_array()
        .ok_or_else(|| Error::invalid("应为 JSON 数组"))
}

fn text(value: &Value) -> Result<&str> {
    value
        .as_str()
        .ok_or_else(|| Error::invalid("应为 JSON 字符串"))
}

fn integer(value: &Value) -> Result<i32> {
    let number = value
        .as_i64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()));
    number
        .and_then(|i| i32::try_from(i).ok())
        .ok_or_else(|| Error::invalid("整数缺失或超出 i32 范围"))
}

fn integer_or(value: &Value, default: i32) -> i32 {
    integer(value).unwrap_or(default)
}

fn aux(value: &Value) -> Result<u16> {
    u16::try_from(integer(value)?).map_err(|_| Error::invalid("aux 超出 uint16 范围"))
}

fn triple(value: &Value) -> Result<Pos> {
    let values = array(value)?;
    if values.len() != 3 {
        return Err(Error::invalid("三维坐标必须包含三个整数"));
    }
    Ok(Pos(
        integer(&values[0])?,
        integer(&values[1])?,
        integer(&values[2])?,
    ))
}

fn add(a: i32, b: i32) -> Result<i32> {
    a.checked_add(b)
        .ok_or_else(|| Error::invalid("坐标相加溢出"))
}

fn world(data: &StructureData, pos: Pos) -> Result<Pos> {
    Ok(Pos(
        add(data.origin.0, pos.0)?,
        add(data.origin.1, pos.1)?,
        add(data.origin.2, pos.2)?,
    ))
}

fn strings(value: &Value) -> Result<Vec<String>> {
    let values = array(value)?;
    if values.is_empty() {
        return Err(Error::invalid("调色板为空"));
    }
    values.iter().map(|v| text(v).map(String::from)).collect()
}

fn palette_state(palette: &[String], index: &Value, data: &Value) -> Result<BlockState> {
    let index = usize::try_from(integer(index)?).map_err(|_| Error::invalid("调色板索引为负"))?;
    let name = palette
        .get(index)
        .ok_or_else(|| Error::invalid("调色板索引越界"))?;
    Ok(blocks::legacy(name, aux(data)?))
}

fn palette(data: &StructureData) -> (Vec<String>, BTreeMap<String, usize>) {
    let mut names = Vec::new();
    let mut indices = BTreeMap::new();
    for block in data.blocks.values() {
        let (name, _) = blocks::to_legacy(&block.state);
        if !indices.contains_key(&name) {
            indices.insert(name.clone(), names.len());
            names.push(name);
        }
    }
    if names.is_empty() {
        names.push("minecraft:air".into());
        indices.insert(names[0].clone(), 0);
    }
    (names, indices)
}

fn finish(mut data: StructureData) -> Result<StructureData> {
    if data.blocks.is_empty() {
        return Err(Error::invalid("结构中没有方块"));
    }
    data.normalize()?;
    Ok(data)
}

fn insert(data: &mut StructureData, pos: Pos, mut block: Block, keep_first_nbt: bool) {
    if let Some(old) = data.blocks.get(&pos)
        && ((keep_first_nbt && old.nbt.is_some()) || block.nbt.is_none())
    {
        block.nbt = old.nbt.clone();
    }
    data.insert(pos, block);
}

fn compound(value: &Value) -> Result<Compound> {
    match Nbt::from_json(value) {
        Nbt::Compound(c) => Ok(c),
        _ => Err(Error::invalid("NBT 根必须为 Compound")),
    }
}

fn snbt(raw: &str) -> Result<Compound> {
    match nbt::parse_snbt(raw)? {
        Nbt::Compound(mut value) => {
            if let Some(Nbt::Compound(inner)) = value.remove("") {
                return Ok(inner);
            }
            Ok(value)
        }
        _ => Err(Error::invalid("SNBT 根必须为 Compound")),
    }
}

fn nbt_string(value: &Compound) -> String {
    nbt::to_snbt(&Nbt::Compound(value.clone()))
}

fn zlib_decode(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    ZlibDecoder::new(bytes).read_to_end(&mut output)?;
    Ok(output)
}

fn zlib_encode(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?)
}

fn byte_bool(value: &Value) -> Nbt {
    Nbt::Byte(i8::from(value.as_bool().unwrap_or_else(|| {
        value
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("true"))
            || integer_or(value, 0) != 0
    })))
}

fn nbt_number(nbt: &Compound, key: &str) -> i64 {
    nbt.get(key).and_then(Nbt::as_i64).unwrap_or_default()
}

fn nbt_text(nbt: &Compound, key: &str) -> String {
    nbt.get(key)
        .and_then(Nbt::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn chest_id(name: &str) -> Option<&'static str> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    match name {
        "chest" | "trapped_chest" => Some("Chest"),
        "barrel" => Some("Barrel"),
        "hopper" => Some("Hopper"),
        "dispenser" => Some("Dispenser"),
        "dropper" => Some("Dropper"),
        "furnace" | "lit_furnace" => Some("Furnace"),
        "blast_furnace" | "lit_blast_furnace" => Some("BlastFurnace"),
        "smoker" | "lit_smoker" => Some("Smoker"),
        "crafter" => Some("Crafter"),
        _ if name.contains("shulker_box") => Some("ShulkerBox"),
        _ => None,
    }
}
