use super::*;

struct SectionIndex {
    start: Pos,
    size: Size,
    position: usize,
    length: usize,
}

fn read_index(bytes: &[u8]) -> Result<Vec<SectionIndex>> {
    if !bytes.len().is_multiple_of(23) {
        return Err(invalid("Construction 区段索引长度无效"));
    }
    bytes
        .as_chunks::<23>()
        .0
        .iter()
        .map(|entry| {
            let read = |i| i32::from_le_bytes(entry[i..i + 4].try_into().unwrap());
            let size = Size(entry[12] as u32, entry[13] as u32, entry[14] as u32);
            validate_size(size)?;
            Ok(SectionIndex {
                start: Pos(read(0), read(4), read(8)),
                size,
                position: usize::try_from(read(15))
                    .map_err(|_| invalid("Construction 区段偏移无效"))?,
                length: usize::try_from(read(19))
                    .map_err(|_| invalid("Construction 区段长度无效"))?,
            })
        })
        .collect()
}

fn parse_palette(metadata: &Compound) -> Result<Vec<Block>> {
    let entries = list(metadata, "block_palette")?;
    let states: Vec<BlockState> = entries
        .iter()
        .map(|value| {
            let value = value
                .as_compound()
                .ok_or_else(|| invalid("Construction 调色板必须为 Compound"))?;
            let namespace = value
                .get("namespace")
                .and_then(Nbt::as_str)
                .ok_or_else(|| invalid("Construction 调色板缺少命名空间"))?;
            let name = value
                .get("blockname")
                .and_then(Nbt::as_str)
                .ok_or_else(|| invalid("Construction 调色板缺少名称"))?;
            let mut properties = value
                .get("properties")
                .and_then(Nbt::as_compound)
                .cloned()
                .unwrap_or_default();
            properties.remove("__version__");
            // Construction 的 Go 读取器先把所有整数属性转换为 TAG_Int，
            // 再交给基岩状态表匹配；保持同样的默认状态选择规则。
            for value in properties.values_mut() {
                if let Some(number) = value.as_i64() {
                    *value = Nbt::Int(number as i32);
                }
            }
            Ok(blocks::bedrock_exact(
                &format!("{namespace}:{name}"),
                properties,
            ))
        })
        .collect::<Result<_>>()?;
    entries
        .iter()
        .zip(&states)
        .map(|(entry, state)| {
            let mut block = Block::new(state.clone());
            let extras = entry
                .as_compound()
                .and_then(|entry| entry.get("extra_blocks"))
                .and_then(Nbt::as_list)
                .unwrap_or(&[]);
            if extras.len() > 1 {
                return Err(Error::Unsupported(
                    "Construction 包含超过两层方块，当前结构模型只能表示两层".into(),
                ));
            }
            if let Some(extra) = extras.first() {
                let index = extra
                    .as_i64()
                    .and_then(|id| usize::try_from(id).ok())
                    .ok_or_else(|| invalid("Construction 附加层索引无效"))?;
                let extra = states
                    .get(index)
                    .ok_or_else(|| invalid("Construction 附加层索引越界"))?;
                if !extra.is_air() {
                    block.extra = Some(extra.clone());
                }
            }
            Ok(block)
        })
        .collect()
}

fn bounds(metadata: &Compound, entries: &[SectionIndex]) -> Result<(Pos, Size)> {
    let mut minimum = Pos(i32::MAX, i32::MAX, i32::MAX);
    let mut maximum = Pos(i32::MIN, i32::MIN, i32::MIN);
    if let Some(Nbt::IntArray(boxes)) = metadata.get("selection_boxes") {
        for selection in boxes.as_chunks::<6>().0 {
            minimum = Pos(
                minimum.0.min(selection[0]),
                minimum.1.min(selection[1]),
                minimum.2.min(selection[2]),
            );
            maximum = Pos(
                maximum.0.max(selection[3]),
                maximum.1.max(selection[4]),
                maximum.2.max(selection[5]),
            );
        }
    }
    if maximum.0 <= minimum.0 || maximum.1 <= minimum.1 || maximum.2 <= minimum.2 {
        minimum = Pos(i32::MAX, i32::MAX, i32::MAX);
        maximum = Pos(i32::MIN, i32::MIN, i32::MIN);
        for entry in entries {
            minimum = Pos(
                minimum.0.min(entry.start.0),
                minimum.1.min(entry.start.1),
                minimum.2.min(entry.start.2),
            );
            let end = add(
                entry.start,
                Pos(
                    entry.size.0 as i32,
                    entry.size.1 as i32,
                    entry.size.2 as i32,
                ),
            );
            maximum = Pos(
                maximum.0.max(end.0),
                maximum.1.max(end.1),
                maximum.2.max(end.2),
            );
        }
    }
    let size = Size(
        (maximum.0 - minimum.0) as u32,
        (maximum.1 - minimum.1) as u32,
        (maximum.2 - minimum.2) as u32,
    );
    validate_size(size)?;
    Ok((minimum, size))
}

fn block_array(section: &Compound) -> Result<Vec<i32>> {
    match (num(section, "blocks_array_type")?, section.get("blocks")) {
        (-1, None) => Ok(Vec::new()),
        (7, Some(Nbt::ByteArray(v))) => Ok(v.iter().map(|v| *v as u8 as i32).collect()),
        (11, Some(Nbt::IntArray(v))) => Ok(v.clone()),
        (12, Some(Nbt::LongArray(v))) => v
            .iter()
            .map(|v| i32::try_from(*v).map_err(|_| invalid("Construction 方块索引溢出")))
            .collect(),
        _ => Err(invalid("Construction 方块数组类型无效")),
    }
}

pub(super) fn decode(bytes_in: &[u8]) -> Result<StructureData> {
    if !bytes_in.starts_with(b"constrct\0")
        || !bytes_in.ends_with(b"constrct")
        || bytes_in.len() < 21
    {
        return Err(invalid("Construction 魔数或版本无效"));
    }
    let pointer_pos = bytes_in.len() - 12;
    let pointer =
        u32::from_be_bytes(bytes_in[pointer_pos..pointer_pos + 4].try_into().unwrap()) as usize;
    if pointer < 9 || pointer >= pointer_pos {
        return Err(invalid("Construction 元数据指针越界"));
    }
    let metadata = root(
        &decompress(&bytes_in[pointer..pointer_pos])?,
        Encoding::BigEndian,
    )?;
    let palette = parse_palette(&metadata)?;
    let entries = read_index(&bytes(&metadata, "section_index_table")?)?;
    let (origin, size) = bounds(&metadata, &entries)?;
    let mut data = StructureData::new(FormatId::Construction);
    data.origin = origin;
    data.size = size;
    for entry in entries {
        let end = entry
            .position
            .checked_add(entry.length)
            .ok_or_else(|| invalid("Construction 区段位置溢出"))?;
        let compressed = bytes_in
            .get(entry.position..end)
            .ok_or_else(|| invalid("Construction 区段数据越界"))?;
        let section = root(&decompress(compressed)?, Encoding::BigEndian)?;
        let values = block_array(&section)?;
        if !values.is_empty() && values.len() != entry.size.volume() as usize {
            return Err(invalid("Construction 区段方块数量不匹配"));
        }
        let offset = subtract(entry.start, origin);
        for (i, index) in values.into_iter().enumerate() {
            if index < 0 {
                continue;
            }
            let block = palette
                .get(index as usize)
                .ok_or_else(|| invalid("Construction 调色板索引越界"))?;
            let p = add(offset, xyz_position(i, entry.size));
            if (!block.state.is_air() || block.extra.is_some())
                && p.0 >= 0
                && p.1 >= 0
                && p.2 >= 0
                && p.0 < size.0 as i32
                && p.1 < size.1 as i32
                && p.2 < size.2 as i32
            {
                data.insert(p, block.clone());
            }
        }
        for entity in entities(&section, "block_entities") {
            if let Ok(p) = position(&entity) {
                let mut nbt = entity
                    .get("nbt")
                    .and_then(Nbt::as_compound)
                    .cloned()
                    .unwrap_or_default();
                if !nbt.contains_key("id")
                    && let Some(name) = entity.get("base_name").and_then(Nbt::as_str)
                {
                    let namespace = entity.get("namespace").and_then(Nbt::as_str).unwrap_or("");
                    nbt.insert(
                        "id".into(),
                        Nbt::String(if namespace.is_empty() {
                            name.into()
                        } else {
                            format!("{namespace}:{name}")
                        }),
                    );
                }
                set_nbt(&mut data, subtract(p, origin), nbt);
            }
        }
        for entity in entities(&section, "entities") {
            let Some(mut nbt) = entity.get("nbt").and_then(Nbt::as_compound).cloned() else {
                data.entities.push(entity);
                continue;
            };
            if !nbt.contains_key("id") {
                let namespace = entity
                    .get("namespace")
                    .and_then(Nbt::as_str)
                    .unwrap_or("minecraft");
                let name = entity.get("base_name").and_then(Nbt::as_str).unwrap_or("");
                nbt.insert("id".into(), Nbt::String(format!("{namespace}:{name}")));
            }
            let coordinates: Option<Vec<_>> = ["x", "y", "z"]
                .iter()
                .zip([origin.0, origin.1, origin.2])
                .map(|(key, origin)| {
                    entity
                        .get(*key)?
                        .scalar_string()
                        .parse::<f64>()
                        .ok()
                        .map(|value| Nbt::Double(value - f64::from(origin)))
                })
                .collect();
            if let Some(coordinates) = coordinates {
                nbt.insert("Pos".into(), Nbt::List(coordinates));
            }
            data.entities.push(nbt);
        }
    }
    data.metadata = metadata;
    for key in ["block_palette", "section_index_table", "selection_boxes"] {
        data.metadata.remove(key);
    }
    Ok(data)
}

fn construction_palette(state: &BlockState, extra_index: Option<usize>) -> Nbt {
    let (namespace, name) = state
        .name
        .split_once(':')
        .unwrap_or(("minecraft", &state.name));
    Nbt::Compound(compound([
        ("namespace", Nbt::String(namespace.into())),
        ("blockname", Nbt::String(name.into())),
        ("properties", Nbt::Compound(state.properties.clone())),
        (
            "extra_blocks",
            Nbt::List(
                extra_index
                    .into_iter()
                    .map(|index| Nbt::Int(index as i32))
                    .collect(),
            ),
        ),
    ]))
}

fn section_entities(data: &StructureData, start: Pos, size: Size) -> Vec<Compound> {
    data.blocks
        .iter()
        .filter_map(|(p, block)| {
            if p.0 < start.0
                || p.1 < start.1
                || p.2 < start.2
                || p.0 >= start.0 + size.0 as i32
                || p.1 >= start.1 + size.1 as i32
                || p.2 >= start.2 + size.2 as i32
            {
                return None;
            }
            let nbt = block.nbt.as_ref()?;
            let id = nbt.get("id").and_then(Nbt::as_str).unwrap_or("");
            let (namespace, name) = id.split_once(':').unwrap_or(("", id));
            let p = add(*p, data.origin);
            Some(compound([
                ("x", Nbt::Int(p.0)),
                ("y", Nbt::Int(p.1)),
                ("z", Nbt::Int(p.2)),
                ("namespace", Nbt::String(namespace.into())),
                ("base_name", Nbt::String(name.into())),
                ("nbt", Nbt::Compound(nbt.clone())),
            ]))
        })
        .collect()
}

fn entity_position(nbt: &Compound) -> [f64; 3] {
    let positions = nbt.get("Pos").and_then(Nbt::as_list).unwrap_or(&[]);
    std::array::from_fn(|axis| {
        positions
            .get(axis)
            .and_then(|value| value.scalar_string().parse().ok())
            .unwrap_or(0.0)
    })
}

fn moving_entities(data: &StructureData, start: Pos, size: Size) -> Vec<Compound> {
    data.entities
        .iter()
        .filter_map(|nbt| {
            let p = entity_position(nbt);
            if p[0] < f64::from(start.0)
                || p[1] < f64::from(start.1)
                || p[2] < f64::from(start.2)
                || p[0] >= f64::from(start.0) + f64::from(size.0)
                || p[1] >= f64::from(start.1) + f64::from(size.1)
                || p[2] >= f64::from(start.2) + f64::from(size.2)
            {
                return None;
            }
            let id = nbt.get("id").and_then(Nbt::as_str).unwrap_or("");
            let (namespace, name) = id.split_once(':').unwrap_or(("minecraft", id));
            Some(compound([
                ("namespace", Nbt::String(namespace.into())),
                ("base_name", Nbt::String(name.into())),
                ("x", Nbt::Double(p[0] + f64::from(data.origin.0))),
                ("y", Nbt::Double(p[1] + f64::from(data.origin.1))),
                ("z", Nbt::Double(p[2] + f64::from(data.origin.2))),
                ("nbt", Nbt::Compound(nbt.clone())),
            ]))
        })
        .collect()
}

fn palette_index(
    palette: &mut Vec<(BlockState, Option<usize>)>,
    lookup: &mut BTreeMap<String, usize>,
    block: &Block,
) -> usize {
    let extra_index = block
        .extra
        .as_ref()
        .filter(|state| !state.is_air())
        .map(|state| palette_index(palette, lookup, &Block::new(state.clone())));
    let key = format!("{}|{extra_index:?}", block.state.string());
    if let Some(index) = lookup.get(&key) {
        return *index;
    }
    let index = palette.len();
    palette.push((block.state.clone(), extra_index));
    lookup.insert(key, index);
    index
}

fn axis_sections(origin: i32, length: u32) -> Vec<(i32, u32)> {
    let mut sections = Vec::new();
    let mut offset = 0;
    while offset < length {
        let world = i64::from(origin) + i64::from(offset);
        let available = (16 - world.rem_euclid(16)) as u32;
        let size = (length - offset).min(available);
        sections.push((offset as i32, size));
        offset += size;
    }
    sections
}

pub(super) fn encode(data: &StructureData) -> Result<Vec<u8>> {
    let mut output = b"constrct\0".to_vec();
    let mut index_table = Vec::new();
    let mut palette = vec![(BlockState::air(), None)];
    let mut lookup = BTreeMap::from([(format!("{}|None", BlockState::air().string()), 0)]);
    // 区段必须位于单个世界子区块内，非 16 对齐的原点在边缘处分段。
    for (sx, width) in axis_sections(data.origin.0, data.size.0) {
        for (sy, height) in axis_sections(data.origin.1, data.size.1) {
            for (sz, length) in axis_sections(data.origin.2, data.size.2) {
                let start = Pos(sx, sy, sz);
                let size = Size(width, height, length);
                let mut values = Vec::with_capacity(size.volume() as usize);
                for i in 0..size.volume() as usize {
                    let p = add(start, xyz_position(i, size));
                    let block = data
                        .blocks
                        .get(&p)
                        .cloned()
                        .unwrap_or_else(|| Block::new(BlockState::air()));
                    values.push(palette_index(&mut palette, &mut lookup, &block) as i32);
                }
                let section = compound([
                    ("blocks_array_type", Nbt::Byte(11)),
                    ("blocks", Nbt::IntArray(values)),
                    (
                        "shape",
                        pos_list(Pos(size.0 as i32, size.1 as i32, size.2 as i32)),
                    ),
                    (
                        "block_entities",
                        entity_list(&section_entities(data, start, size)),
                    ),
                    ("entities", entity_list(&moving_entities(data, start, size))),
                ]);
                let compressed = compress(
                    &nbt::encode("", &Nbt::Compound(section), Encoding::BigEndian)?,
                    true,
                )?;
                let position = i32::try_from(output.len())
                    .map_err(|_| invalid("Construction 文件超过 2GiB"))?;
                let length = i32::try_from(compressed.len())
                    .map_err(|_| invalid("Construction 区段过大"))?;
                let world = add(start, data.origin);
                for value in [world.0, world.1, world.2] {
                    index_table.extend_from_slice(&value.to_le_bytes());
                }
                index_table.extend_from_slice(&[size.0 as u8, size.1 as u8, size.2 as u8]);
                index_table.extend_from_slice(&position.to_le_bytes());
                index_table.extend_from_slice(&length.to_le_bytes());
                output.extend_from_slice(&compressed);
            }
        }
    }
    let mut metadata = data.metadata.clone();
    metadata.insert("section_version".into(), Nbt::Byte(0));
    metadata.insert(
        "block_palette".into(),
        Nbt::List(
            palette
                .iter()
                .map(|(state, extra)| construction_palette(state, *extra))
                .collect(),
        ),
    );
    metadata.insert(
        "section_index_table".into(),
        Nbt::ByteArray(index_table.into_iter().map(|v| v as i8).collect()),
    );
    metadata
        .entry("created_with".into())
        .or_insert(Nbt::String("RustWaterStructure".into()));
    metadata.entry("export_version".into()).or_insert_with(|| {
        Nbt::Compound(compound([
            ("edition", Nbt::String("bedrock".into())),
            (
                "version",
                Nbt::List(vec![Nbt::Int(1), Nbt::Int(21), Nbt::Int(60)]),
            ),
        ]))
    });
    let end = add(
        data.origin,
        Pos(data.size.0 as i32, data.size.1 as i32, data.size.2 as i32),
    );
    metadata.insert(
        "selection_boxes".into(),
        Nbt::IntArray(vec![
            data.origin.0,
            data.origin.1,
            data.origin.2,
            end.0,
            end.1,
            end.2,
        ]),
    );
    let metadata_pos =
        u32::try_from(output.len()).map_err(|_| invalid("Construction 文件超过 4GiB"))?;
    output.extend_from_slice(&compress(
        &nbt::encode("", &Nbt::Compound(metadata), Encoding::BigEndian)?,
        true,
    )?);
    output.extend_from_slice(&metadata_pos.to_be_bytes());
    output.extend_from_slice(b"constrct");
    Ok(output)
}
