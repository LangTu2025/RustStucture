//! Java NBT、基岩结构方块及 Construction 格式编解码。
use crate::nbt::{self, Compound, Encoding, Nbt};
use crate::{Block, BlockState, Error, FormatId, Pos, Result, Size, StructureData, blocks};
use flate2::{
    Compression,
    read::{GzDecoder, ZlibDecoder},
    write::{GzEncoder, ZlibEncoder},
};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
};

mod construction;
pub mod dense;
mod regions;

pub fn decode(format: FormatId, data: &[u8]) -> Result<StructureData> {
    match format {
        FormatId::Construction => construction::decode(data),
        FormatId::AxiomBP => regions::decode_axiom(data),
        FormatId::Litematic => regions::decode_litematic(data),
        FormatId::MCStructure => decode_mcstructure(data),
        FormatId::Schematic | FormatId::SchemV1 | FormatId::SchemV2 => {
            decode_schematic(format, data)
        }
        _ => Err(Error::Unsupported(format!(
            "{} 不属于 NBT 结构格式",
            format.name()
        ))),
    }
}

pub fn encode(format: FormatId, data: &StructureData) -> Result<Vec<u8>> {
    validate_size(data.size)?;
    match format {
        FormatId::Construction => construction::encode(data),
        FormatId::AxiomBP => regions::encode_axiom(data),
        FormatId::Litematic => regions::encode_litematic(data),
        FormatId::MCStructure => encode_mcstructure(data),
        FormatId::Schematic | FormatId::SchemV1 | FormatId::SchemV2 => {
            encode_schematic(format, data)
        }
        _ => Err(Error::Unsupported(format!(
            "{} 不属于 NBT 结构格式",
            format.name()
        ))),
    }
}

fn invalid(message: &str) -> Error {
    Error::invalid(message)
}
fn compound(values: impl IntoIterator<Item = (&'static str, Nbt)>) -> Compound {
    values
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}
fn num(map: &Compound, name: &str) -> Result<i32> {
    map.get(name)
        .and_then(Nbt::as_i64)
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| Error::invalid(format!("缺少有效整数 {name}")))
}
fn sub<'a>(map: &'a Compound, name: &str) -> Result<&'a Compound> {
    map.get(name)
        .and_then(Nbt::as_compound)
        .ok_or_else(|| Error::invalid(format!("缺少复合标签 {name}")))
}
fn list<'a>(map: &'a Compound, name: &str) -> Result<&'a [Nbt]> {
    map.get(name)
        .and_then(Nbt::as_list)
        .ok_or_else(|| Error::invalid(format!("缺少列表 {name}")))
}
fn bytes(map: &Compound, name: &str) -> Result<Vec<u8>> {
    match map.get(name) {
        Some(Nbt::ByteArray(v)) => Ok(v.iter().map(|v| *v as u8).collect()),
        _ => Err(Error::invalid(format!("缺少字节数组 {name}"))),
    }
}
fn ints(value: &Nbt) -> Result<Vec<i32>> {
    match value {
        Nbt::IntArray(v) => Ok(v.clone()),
        Nbt::List(v) => v
            .iter()
            .map(|v| {
                v.as_i64()
                    .and_then(|n| i32::try_from(n).ok())
                    .ok_or_else(|| invalid("无效整数列表"))
            })
            .collect(),
        _ => Err(invalid("期望整数数组或列表")),
    }
}
fn vector(value: &Nbt) -> Result<Pos> {
    if let Some(v) = value.as_compound() {
        return Ok(Pos(num(v, "x")?, num(v, "y")?, num(v, "z")?));
    }
    let v = ints(value)?;
    if v.len() != 3 {
        return Err(invalid("坐标必须包含三个整数"));
    }
    Ok(Pos(v[0], v[1], v[2]))
}
fn pos_compound(p: Pos) -> Nbt {
    Nbt::Compound(compound([
        ("x", Nbt::Int(p.0)),
        ("y", Nbt::Int(p.1)),
        ("z", Nbt::Int(p.2)),
    ]))
}
fn pos_list(p: Pos) -> Nbt {
    Nbt::List(vec![Nbt::Int(p.0), Nbt::Int(p.1), Nbt::Int(p.2)])
}
fn validate_size(size: Size) -> Result<usize> {
    if size.0 == 0 || size.1 == 0 || size.2 == 0 || size.volume() > 512_000_000 {
        return Err(invalid("结构尺寸无效或体积超过 512000000"));
    }
    Ok(size.volume() as usize)
}
fn dimensions(root: &Compound) -> Result<Size> {
    let s = Size(
        u32::try_from(num(root, "Width")?).map_err(|_| invalid("无效宽度"))?,
        u32::try_from(num(root, "Height")?).map_err(|_| invalid("无效高度"))?,
        u32::try_from(num(root, "Length")?).map_err(|_| invalid("无效长度"))?,
    );
    validate_size(s)?;
    Ok(s)
}
/// Y→Z→X 遍历布局：X 最快，供 Schematic、Schem、Litematic、Axiom 使用。
fn yzx_position(index: usize, size: Size) -> Pos {
    Pos(
        (index % size.0 as usize) as i32,
        (index / size.0 as usize / size.2 as usize) as i32,
        (index / size.0 as usize % size.2 as usize) as i32,
    )
}
/// X→Y→Z 遍历布局：Z 最快，供 MCStructure、Construction 使用。
fn xyz_position(index: usize, size: Size) -> Pos {
    Pos(
        (index / size.2 as usize / size.1 as usize) as i32,
        (index / size.2 as usize % size.1 as usize) as i32,
        (index % size.2 as usize) as i32,
    )
}
fn add(a: Pos, b: Pos) -> Pos {
    Pos(a.0 + b.0, a.1 + b.1, a.2 + b.2)
}
fn subtract(a: Pos, b: Pos) -> Pos {
    Pos(a.0 - b.0, a.1 - b.1, a.2 - b.2)
}
fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    if data.starts_with(&[0x1f, 0x8b]) {
        GzDecoder::new(data).read_to_end(&mut out)?;
    } else if data.first() == Some(&0x78) {
        ZlibDecoder::new(data).read_to_end(&mut out)?;
    } else {
        out.extend_from_slice(data);
    }
    Ok(out)
}
fn compress(data: &[u8], gzip: bool) -> Result<Vec<u8>> {
    if gzip {
        let mut writer = GzEncoder::new(Vec::new(), Compression::fast());
        writer.write_all(data)?;
        Ok(writer.finish()?)
    } else {
        let mut writer = ZlibEncoder::new(Vec::new(), Compression::fast());
        writer.write_all(data)?;
        Ok(writer.finish()?)
    }
}
fn root(data: &[u8], encoding: Encoding) -> Result<Compound> {
    let (_, value) = nbt::decode(data, encoding)?;
    match value {
        Nbt::Compound(v) => Ok(v),
        _ => Err(invalid("根标签必须为 Compound")),
    }
}
fn gzip_nbt(name: &str, data: Compound) -> Result<Vec<u8>> {
    compress(
        &nbt::encode(name, &Nbt::Compound(data), Encoding::BigEndian)?,
        true,
    )
}
fn entities(root: &Compound, name: &str) -> Vec<Compound> {
    root.get(name)
        .and_then(Nbt::as_list)
        .unwrap_or(&[])
        .iter()
        .filter_map(|v| v.as_compound().cloned())
        .collect()
}
fn entity_list(values: &[Compound]) -> Nbt {
    Nbt::List(values.iter().cloned().map(Nbt::Compound).collect())
}
fn set_nbt(data: &mut StructureData, p: Pos, nbt: Compound) {
    data.blocks
        .entry(p)
        .or_insert_with(|| Block::new(BlockState::air()))
        .nbt = Some(nbt);
}
fn position(entity: &Compound) -> Result<Pos> {
    if let Some(p) = entity.get("Pos") {
        return vector(p);
    }
    Ok(Pos(num(entity, "x")?, num(entity, "y")?, num(entity, "z")?))
}
fn attach_entities(data: &mut StructureData, root: &Compound, name: &str, offset: Pos) {
    for entity in entities(root, name) {
        if let Ok(p) = position(&entity) {
            set_nbt(data, add(p, offset), entity);
        }
    }
}
fn block_entities(data: &StructureData, sponge: bool) -> Vec<Compound> {
    data.blocks
        .iter()
        .filter_map(|(p, b)| {
            b.nbt.as_ref().map(|n| {
                let mut n = n.clone();
                if sponge {
                    n.insert("Pos".into(), Nbt::IntArray(vec![p.0, p.1, p.2]));
                } else {
                    for (k, v) in [("x", p.0), ("y", p.1), ("z", p.2)] {
                        n.insert(k.into(), Nbt::Int(v));
                    }
                }
                n
            })
        })
        .collect()
}
fn palette_state(value: &Nbt, bedrock: bool) -> Result<BlockState> {
    let entry = value
        .as_compound()
        .ok_or_else(|| invalid("调色板条目必须是 Compound"))?;
    let name = entry
        .get(if bedrock { "name" } else { "Name" })
        .and_then(Nbt::as_str)
        .ok_or_else(|| invalid("调色板缺少方块名称"))?;
    if !bedrock && matches!(name, "minecraft:void_air" | "minecraft:cave_air") {
        return Ok(BlockState::air());
    }
    let properties = entry
        .get(if bedrock { "states" } else { "Properties" })
        .and_then(Nbt::as_compound)
        .cloned()
        .unwrap_or_default();
    if bedrock {
        Ok(blocks::bedrock_exact(name, properties))
    } else {
        let state = BlockState {
            name: name.into(),
            properties,
        };
        Ok(blocks::java(&state.string()))
    }
}
fn palette_entry(state: &BlockState, bedrock: bool) -> Nbt {
    if bedrock {
        Nbt::Compound(compound([
            ("name", Nbt::String(state.name.clone())),
            ("states", Nbt::Compound(state.properties.clone())),
            ("version", Nbt::Int(18168865)),
        ]))
    } else {
        let text = blocks::to_java(state);
        let property_start = text.find('[').unwrap_or(text.len());
        let name = &text[..property_start];
        let properties = text[property_start..]
            .trim_matches(['[', ']'])
            .split(',')
            .filter_map(|entry| entry.split_once('='))
            .map(|(key, value)| (key.to_string(), Nbt::String(value.to_string())))
            .collect();
        Nbt::Compound(compound([
            ("Name", Nbt::String(name.to_string())),
            ("Properties", Nbt::Compound(properties)),
        ]))
    }
}
fn palette_add(
    palette: &mut Vec<BlockState>,
    lookup: &mut BTreeMap<String, usize>,
    state: &BlockState,
) -> usize {
    let key = state.string();
    if let Some(index) = lookup.get(&key) {
        return *index;
    }
    let index = palette.len();
    palette.push(state.clone());
    lookup.insert(key, index);
    index
}
fn decode_schematic(format: FormatId, bytes_in: &[u8]) -> Result<StructureData> {
    let original = root(&decompress(bytes_in)?, Encoding::BigEndian)?;
    let root = original
        .get("Schematic")
        .and_then(Nbt::as_compound)
        .unwrap_or(&original);
    let mut data = StructureData::new(format);
    data.size = dimensions(root)?;
    data.metadata = root.clone();
    data.entities = entities(root, "Entities");
    if format == FormatId::Schematic {
        data.origin = Pos(
            num(root, "WEOriginX").unwrap_or(0),
            num(root, "WEOriginY").unwrap_or(0),
            num(root, "WEOriginZ").unwrap_or(0),
        );
        let ids = bytes(root, "Blocks")?;
        let values = bytes(root, "Data")?;
        if ids.len() != data.size.volume() as usize || values.len() != ids.len() {
            return Err(invalid("Schematic 方块数组长度不匹配"));
        }
        for (index, (&id, &value)) in ids.iter().zip(&values).enumerate() {
            let state = blocks::from_schematic(id, value);
            if !state.is_air() {
                data.insert(yzx_position(index, data.size), Block::new(state));
            }
        }
        attach_entities(&mut data, root, "TileEntities", Pos::default());
        for key in ["Blocks", "Data", "TileEntities", "Entities"] {
            data.metadata.remove(key);
        }
    } else {
        if let Some(offset) = root.get("Offset") {
            data.origin = vector(offset)?;
        }
        let container = if format == FormatId::SchemV1 {
            root
        } else {
            sub(root, "Blocks")?
        };
        let palette: BTreeMap<i32, BlockState> = sub(container, "Palette")?
            .iter()
            .map(|(s, id)| {
                Ok((
                    i32::try_from(id.as_i64().ok_or_else(|| invalid("调色板索引无效"))?)
                        .map_err(|_| invalid("调色板索引溢出"))?,
                    blocks::java(s),
                ))
            })
            .collect::<Result<_>>()?;
        let indices = bytes(
            container,
            if format == FormatId::SchemV1 {
                "BlockData"
            } else {
                "Data"
            },
        )?;
        let mut cursor = 0;
        for index in 0..data.size.volume() as usize {
            let value = read_varint(&indices, &mut cursor)?;
            let state = palette
                .get(&(value as i32))
                .ok_or_else(|| invalid("方块索引超出调色板"))?
                .clone();
            if !state.is_air() {
                data.insert(yzx_position(index, data.size), Block::new(state));
            }
        }
        attach_entities(&mut data, container, "BlockEntities", Pos::default());
        for key in [
            "Blocks",
            "BlockData",
            "Palette",
            "BlockEntities",
            "Entities",
        ] {
            data.metadata.remove(key);
        }
    }
    Ok(data)
}
fn read_varint(data: &[u8], cursor: &mut usize) -> Result<u32> {
    let mut value = 0;
    for shift in (0..35).step_by(7) {
        let b = *data
            .get(*cursor)
            .ok_or_else(|| invalid("Varint 数据截断"))?;
        *cursor += 1;
        if shift == 28 && b > 15 {
            return Err(invalid("Varint 溢出"));
        }
        value |= u32::from(b & 127) << shift;
        if b & 128 == 0 {
            return Ok(value);
        }
    }
    Err(invalid("Varint 无效"))
}
fn write_varint(mut value: u32, data: &mut Vec<i8>) {
    while value >= 128 {
        data.push((value as u8 | 128) as i8);
        value >>= 7;
    }
    data.push(value as i8);
}
fn encode_schematic(format: FormatId, data: &StructureData) -> Result<Vec<u8>> {
    if [data.size.0, data.size.1, data.size.2]
        .iter()
        .any(|v| *v > i16::MAX as u32)
    {
        return Err(invalid("Schematic 尺寸超过 Short 范围"));
    }
    let mut root = data.metadata.clone();
    for (k, v) in [
        ("Width", data.size.0),
        ("Height", data.size.1),
        ("Length", data.size.2),
    ] {
        root.insert(k.into(), Nbt::Short(v as i16));
    }
    root.insert("Entities".into(), entity_list(&data.entities));
    if format == FormatId::Schematic {
        root.insert("Materials".into(), Nbt::String("Alpha".into()));
        for (k, v) in [
            ("WEOriginX", data.origin.0),
            ("WEOriginY", data.origin.1),
            ("WEOriginZ", data.origin.2),
        ] {
            root.insert(k.into(), Nbt::Int(v));
        }
        let mut ids = Vec::new();
        let mut values = Vec::new();
        for i in 0..data.size.volume() as usize {
            let state = data
                .blocks
                .get(&yzx_position(i, data.size))
                .map(|b| b.state.clone())
                .unwrap_or_else(BlockState::air);
            let (id, value) = blocks::to_schematic(&state);
            ids.push(id as i8);
            values.push(value as i8);
        }
        root.insert("Blocks".into(), Nbt::ByteArray(ids));
        root.insert("Data".into(), Nbt::ByteArray(values));
        root.insert(
            "TileEntities".into(),
            entity_list(&block_entities(data, false)),
        );
    } else {
        root.insert(
            "Version".into(),
            Nbt::Int(if format == FormatId::SchemV1 { 1 } else { 2 }),
        );
        root.entry("DataVersion".into()).or_insert(Nbt::Int(4556));
        root.insert(
            "Offset".into(),
            Nbt::IntArray(vec![data.origin.0, data.origin.1, data.origin.2]),
        );
        let mut palette = Compound::new();
        let mut indices = Vec::new();
        for i in 0..data.size.volume() as usize {
            let state = data
                .blocks
                .get(&yzx_position(i, data.size))
                .map(|b| b.state.clone())
                .unwrap_or_else(BlockState::air);
            let key = blocks::to_java(&state);
            let next = palette.len() as i32;
            let id = palette
                .entry(key)
                .or_insert(Nbt::Int(next))
                .as_i64()
                .unwrap() as u32;
            write_varint(id, &mut indices);
        }
        let mut block = compound([
            ("Palette", Nbt::Compound(palette.clone())),
            ("BlockEntities", entity_list(&block_entities(data, true))),
        ]);
        block.insert(
            if format == FormatId::SchemV1 {
                "BlockData"
            } else {
                "Data"
            }
            .into(),
            Nbt::ByteArray(indices),
        );
        if format == FormatId::SchemV1 {
            root.extend(block);
            root.insert("PaletteMax".into(), Nbt::Int(palette.len() as i32));
        } else {
            root.insert("Blocks".into(), Nbt::Compound(block));
        }
    }
    gzip_nbt("Schematic", root)
}

fn decode_mcstructure(bytes: &[u8]) -> Result<StructureData> {
    let root = root(bytes, Encoding::LittleEndian)?;
    let mut data = StructureData::new(FormatId::MCStructure);
    let size = vector(root.get("size").ok_or_else(|| invalid("缺少 size"))?)?;
    data.size = Size(size.0 as u32, size.1 as u32, size.2 as u32);
    let volume = validate_size(data.size)?;
    data.origin = vector(
        root.get("structure_world_origin")
            .ok_or_else(|| invalid("缺少世界原点"))?,
    )?;
    let structure = sub(&root, "structure")?;
    data.entities = entities(structure, "entities");
    let palette = sub(sub(structure, "palette")?, "default")?;
    let states = list(palette, "block_palette")?
        .iter()
        .map(|v| palette_state(v, true))
        .collect::<Result<Vec<_>>>()?;
    let layers = list(structure, "block_indices")?;
    if layers.len() != 2 {
        return Err(invalid("MCStructure 必须有两层方块索引"));
    }
    let layers = [ints(&layers[0])?, ints(&layers[1])?];
    if layers.iter().any(|v| v.len() != volume) {
        return Err(invalid("MCStructure 索引数量与体积不符"));
    }
    for (i, (&primary, &secondary)) in layers[0].iter().zip(&layers[1]).enumerate() {
        let state = |id: i32| -> Result<BlockState> {
            if id == -1 {
                Ok(BlockState::air())
            } else {
                states
                    .get(id as usize)
                    .cloned()
                    .ok_or_else(|| invalid("MCStructure 调色板索引越界"))
            }
        };
        let mut block = Block::new(state(primary)?);
        let extra = state(secondary)?;
        if !extra.is_air() {
            block.extra = Some(extra);
        }
        if !block.state.is_air() || block.extra.is_some() {
            data.insert(xyz_position(i, data.size), block);
        }
    }
    for (index, entry) in sub(palette, "block_position_data")? {
        let index: usize = index.parse().map_err(|_| invalid("方块实体索引无效"))?;
        if index >= volume {
            return Err(invalid("方块实体索引越界"));
        }
        if let Some(nbt) = entry
            .as_compound()
            .and_then(|v| v.get("block_entity_data"))
            .and_then(Nbt::as_compound)
        {
            let p = xyz_position(index, data.size);
            set_nbt(&mut data, p, nbt.clone());
        }
    }
    data.metadata = root;
    data.metadata.remove("structure");
    Ok(data)
}
fn encode_mcstructure(data: &StructureData) -> Result<Vec<u8>> {
    let mut root = data.metadata.clone();
    root.insert("format_version".into(), Nbt::Int(1));
    root.insert(
        "size".into(),
        pos_list(Pos(
            data.size.0 as i32,
            data.size.1 as i32,
            data.size.2 as i32,
        )),
    );
    root.insert("structure_world_origin".into(), pos_list(data.origin));
    let mut palette = vec![BlockState::air()];
    let mut lookup = BTreeMap::from([(BlockState::air().string(), 0)]);
    let mut primary = Vec::new();
    let mut extra = Vec::new();
    let mut positions = Compound::new();
    for i in 0..data.size.volume() as usize {
        if let Some(block) = data.blocks.get(&xyz_position(i, data.size)) {
            primary.push(Nbt::Int(
                palette_add(&mut palette, &mut lookup, &block.state) as i32,
            ));
            extra.push(Nbt::Int(
                block
                    .extra
                    .as_ref()
                    .map(|v| palette_add(&mut palette, &mut lookup, v) as i32)
                    .unwrap_or(-1),
            ));
            if let Some(nbt) = &block.nbt {
                positions.insert(
                    i.to_string(),
                    Nbt::Compound(compound([(
                        "block_entity_data",
                        Nbt::Compound(nbt.clone()),
                    )])),
                );
            }
        } else {
            primary.push(Nbt::Int(0));
            extra.push(Nbt::Int(-1));
        }
    }
    if palette.is_empty() {
        palette.push(BlockState::air());
    }
    let default = compound([
        (
            "block_palette",
            Nbt::List(palette.iter().map(|s| palette_entry(s, true)).collect()),
        ),
        ("block_position_data", Nbt::Compound(positions)),
    ]);
    root.insert(
        "structure".into(),
        Nbt::Compound(compound([
            (
                "block_indices",
                Nbt::List(vec![Nbt::List(primary), Nbt::List(extra)]),
            ),
            ("entities", entity_list(&data.entities)),
            (
                "palette",
                Nbt::Compound(compound([("default", Nbt::Compound(default))])),
            ),
        ])),
    );
    nbt::encode("", &Nbt::Compound(root), Encoding::LittleEndian)
}
