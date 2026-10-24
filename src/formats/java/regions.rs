use super::*;

fn bit_width(palette_size: usize, minimum: usize) -> usize {
    ((usize::BITS - palette_size.saturating_sub(1).leading_zeros()) as usize).max(minimum)
}

fn unpack(values: &[i64], count: usize, palette_size: usize, padded: bool) -> Result<Vec<usize>> {
    let bits = bit_width(palette_size, if padded { 4 } else { 2 });
    let per_long = 64 / bits;
    let required = if padded {
        count.div_ceil(per_long)
    } else {
        (count * bits).div_ceil(64)
    };
    if values.len() < required {
        return Err(invalid("位打包方块数组截断"));
    }
    let mask = (1u64 << bits) - 1;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let (index, shift) = if padded {
            (i / per_long, i % per_long * bits)
        } else {
            (i * bits / 64, i * bits % 64)
        };
        let mut value = (values[index] as u64) >> shift;
        if !padded && shift + bits > 64 {
            value |= (values[index + 1] as u64) << (64 - shift);
        }
        let value = (value & mask) as usize;
        if value >= palette_size {
            return Err(invalid("位打包调色板索引越界"));
        }
        out.push(value);
    }
    Ok(out)
}

fn pack(values: &[usize], palette_size: usize, padded: bool) -> Vec<i64> {
    let bits = bit_width(palette_size, if padded { 4 } else { 2 });
    let per_long = 64 / bits;
    let length = if padded {
        values.len().div_ceil(per_long)
    } else {
        (values.len() * bits).div_ceil(64)
    };
    let mut packed = vec![0u64; length];
    for (i, &value) in values.iter().enumerate() {
        let (index, shift) = if padded {
            (i / per_long, i % per_long * bits)
        } else {
            (i * bits / 64, i * bits % 64)
        };
        packed[index] |= (value as u64) << shift;
        if !padded && shift + bits > 64 {
            packed[index + 1] |= (value as u64) >> (64 - shift);
        }
    }
    packed.into_iter().map(|v| v as i64).collect()
}

fn long_array<'a>(root: &'a Compound, key: &str) -> Result<&'a [i64]> {
    match root.get(key) {
        Some(Nbt::LongArray(v)) => Ok(v),
        _ => Err(invalid("缺少 LongArray 方块数据")),
    }
}

pub(super) fn decode_litematic(bytes: &[u8]) -> Result<StructureData> {
    let root = root(&decompress(bytes)?, Encoding::BigEndian)?;
    let regions = sub(&root, "Regions")?;
    if regions.is_empty() {
        return Err(invalid("Litematic 没有区域"));
    }
    let mut minimum = Pos(i32::MAX, i32::MAX, i32::MAX);
    let mut maximum = Pos(i32::MIN, i32::MIN, i32::MIN);
    let mut region_data = Vec::new();
    for region in regions.values() {
        let region = region
            .as_compound()
            .ok_or_else(|| invalid("Litematic 区域必须为 Compound"))?;
        let position = vector(
            region
                .get("Position")
                .ok_or_else(|| invalid("缺少区域位置"))?,
        )?;
        let size = vector(region.get("Size").ok_or_else(|| invalid("缺少区域尺寸"))?)?;
        let absolute = Size(
            size.0.unsigned_abs(),
            size.1.unsigned_abs(),
            size.2.unsigned_abs(),
        );
        let volume = validate_size(absolute)?;
        // 负尺寸区域的位置标记另一端，存储仍按包围盒从小到大排列。
        let start = Pos(
            position.0 + (size.0 + 1).min(0),
            position.1 + (size.1 + 1).min(0),
            position.2 + (size.2 + 1).min(0),
        );
        let end = add(
            start,
            Pos(absolute.0 as i32, absolute.1 as i32, absolute.2 as i32),
        );
        minimum = Pos(
            minimum.0.min(start.0),
            minimum.1.min(start.1),
            minimum.2.min(start.2),
        );
        maximum = Pos(
            maximum.0.max(end.0),
            maximum.1.max(end.1),
            maximum.2.max(end.2),
        );
        let palette = list(region, "BlockStatePalette")?
            .iter()
            .map(|v| palette_state(v, false))
            .collect::<Result<Vec<_>>>()?;
        let values = unpack(
            long_array(region, "BlockStates")?,
            volume,
            palette.len(),
            false,
        )?;
        region_data.push((region, start, absolute, palette, values));
    }
    let mut data = StructureData::new(FormatId::Litematic);
    data.origin = minimum;
    data.size = Size(
        (maximum.0 - minimum.0) as u32,
        (maximum.1 - minimum.1) as u32,
        (maximum.2 - minimum.2) as u32,
    );
    validate_size(data.size)?;
    data.metadata = root.clone();
    data.metadata.remove("Regions");
    for (region, start, size, palette, values) in region_data {
        let offset = subtract(start, minimum);
        for (i, index) in values.into_iter().enumerate() {
            let state = &palette[index];
            let p = add(yzx_position(i, size), offset);
            if state.is_air() {
                data.blocks.remove(&p);
            } else {
                data.insert(p, Block::new(state.clone()));
            }
        }
        attach_entities(&mut data, region, "TileEntities", offset);
        data.entities.extend(entities(region, "Entities"));
    }
    Ok(data)
}

pub(super) fn encode_litematic(data: &StructureData) -> Result<Vec<u8>> {
    let mut palette = vec![BlockState::air()];
    let mut lookup = BTreeMap::from([(BlockState::air().string(), 0)]);
    let mut values = Vec::with_capacity(data.size.volume() as usize);
    for i in 0..data.size.volume() as usize {
        let state = data
            .blocks
            .get(&yzx_position(i, data.size))
            .map(|b| b.state.clone())
            .unwrap_or_else(BlockState::air);
        values.push(palette_add(&mut palette, &mut lookup, &state));
    }
    let region = compound([
        ("Position", pos_compound(data.origin)),
        (
            "Size",
            pos_compound(Pos(
                data.size.0 as i32,
                data.size.1 as i32,
                data.size.2 as i32,
            )),
        ),
        (
            "BlockStatePalette",
            Nbt::List(palette.iter().map(|v| palette_entry(v, false)).collect()),
        ),
        (
            "BlockStates",
            Nbt::LongArray(pack(&values, palette.len(), false)),
        ),
        ("Entities", entity_list(&data.entities)),
        ("TileEntities", entity_list(&block_entities(data, false))),
        ("PendingBlockTicks", Nbt::List(vec![])),
        ("PendingFluidTicks", Nbt::List(vec![])),
    ]);
    let mut root = data.metadata.clone();
    root.insert("Version".into(), Nbt::Int(6));
    root.insert("SubVersion".into(), Nbt::Int(1));
    root.entry("MinecraftDataVersion".into())
        .or_insert(Nbt::Int(4556));
    let mut metadata = root
        .get("Metadata")
        .and_then(Nbt::as_compound)
        .cloned()
        .unwrap_or_else(|| {
            compound([
                ("Name", Nbt::String("RustWaterStructure".into())),
                ("Author", Nbt::String("".into())),
                ("Description", Nbt::String("".into())),
                ("TimeCreated", Nbt::Long(0)),
                ("TimeModified", Nbt::Long(0)),
            ])
        });
    metadata.insert("RegionCount".into(), Nbt::Int(1));
    metadata.insert("TotalBlocks".into(), Nbt::Int(data.non_air_count() as i32));
    metadata.insert("TotalVolume".into(), Nbt::Long(data.size.volume() as i64));
    metadata.insert(
        "EnclosingSize".into(),
        pos_compound(Pos(
            data.size.0 as i32,
            data.size.1 as i32,
            data.size.2 as i32,
        )),
    );
    root.insert("Metadata".into(), Nbt::Compound(metadata));
    root.insert(
        "Regions".into(),
        Nbt::Compound(compound([("region", Nbt::Compound(region))])),
    );
    gzip_nbt("Litematic", root)
}

pub(super) fn decode_axiom(bytes: &[u8]) -> Result<StructureData> {
    let mut reader = nbt::Reader::new(bytes, Encoding::BigEndian);
    if reader.int()? != 0x0ae5bb36 {
        return Err(invalid("Axiom 魔数无效"));
    }
    let header_len = usize::try_from(reader.int()?).map_err(|_| invalid("Axiom 头部长度无效"))?;
    let header = reader.take(header_len)?.to_vec();
    let thumbnail_len =
        usize::try_from(reader.int()?).map_err(|_| invalid("Axiom 缩略图长度无效"))?;
    let thumbnail = reader.take(thumbnail_len)?.to_vec();
    let length = usize::try_from(reader.int()?).map_err(|_| invalid("Axiom 方块数据长度无效"))?;
    let root = root(&decompress(reader.take(length)?)?, Encoding::BigEndian)?;
    let regions = list(&root, "BlockRegion")?;
    if regions.is_empty() {
        return Err(invalid("Axiom 没有区域"));
    }
    let mut data = StructureData::new(FormatId::AxiomBP);
    let mut minimum = Pos(i32::MAX, i32::MAX, i32::MAX);
    let mut maximum = Pos(i32::MIN, i32::MIN, i32::MIN);
    let mut parsed = Vec::new();
    for region in regions {
        let region = region
            .as_compound()
            .ok_or_else(|| invalid("Axiom 区域必须为 Compound"))?;
        let p = Pos(num(region, "X")?, num(region, "Y")?, num(region, "Z")?);
        minimum = Pos(minimum.0.min(p.0), minimum.1.min(p.1), minimum.2.min(p.2));
        maximum = Pos(maximum.0.max(p.0), maximum.1.max(p.1), maximum.2.max(p.2));
        let states = sub(region, "BlockStates")?;
        let palette = list(states, "palette")?
            .iter()
            .map(|v| palette_state(v, false))
            .collect::<Result<Vec<_>>>()?;
        let indices = match states.get("data") {
            None if palette.len() == 1 => vec![0; 4096],
            _ => unpack(long_array(states, "data")?, 4096, palette.len(), true)?,
        };
        parsed.push((p, palette, indices));
    }
    data.origin = Pos(minimum.0 * 16, minimum.1 * 16, minimum.2 * 16);
    data.size = Size(
        ((maximum.0 - minimum.0 + 1) * 16) as u32,
        ((maximum.1 - minimum.1 + 1) * 16) as u32,
        ((maximum.2 - minimum.2 + 1) * 16) as u32,
    );
    validate_size(data.size)?;
    for (p, palette, indices) in parsed {
        let p = subtract(p, minimum);
        let offset = Pos(p.0 * 16, p.1 * 16, p.2 * 16);
        for (i, index) in indices.into_iter().enumerate() {
            if !palette[index].is_air() {
                data.insert(
                    add(offset, yzx_position(i, Size(16, 16, 16))),
                    Block::new(palette[index].clone()),
                );
            }
        }
    }
    // 原 Go 将区域坐标最小值归零，仅裁去正方向末端的空气。
    data.size = Size(
        data.blocks
            .keys()
            .map(|p| p.0 as u32 + 1)
            .max()
            .unwrap_or(0),
        data.blocks
            .keys()
            .map(|p| p.1 as u32 + 1)
            .max()
            .unwrap_or(0),
        data.blocks
            .keys()
            .map(|p| p.2 as u32 + 1)
            .max()
            .unwrap_or(0),
    );
    data.entities = entities(&root, "Entities");
    let entity_offset = Pos(-data.origin.0, -data.origin.1, -data.origin.2);
    attach_entities(&mut data, &root, "BlockEntities", entity_offset);
    data.metadata = root;
    data.metadata.remove("BlockRegion");
    data.metadata.insert(
        "AxiomHeader".into(),
        Nbt::ByteArray(header.into_iter().map(|v| v as i8).collect()),
    );
    data.metadata.insert(
        "AxiomThumbnail".into(),
        Nbt::ByteArray(thumbnail.into_iter().map(|v| v as i8).collect()),
    );
    Ok(data)
}

pub(super) fn encode_axiom(data: &StructureData) -> Result<Vec<u8>> {
    let mut regions = Vec::new();
    let minimum = Pos(
        data.origin.0.div_euclid(16),
        data.origin.1.div_euclid(16),
        data.origin.2.div_euclid(16),
    );
    let end = add(
        data.origin,
        Pos(
            data.size.0 as i32 - 1,
            data.size.1 as i32 - 1,
            data.size.2 as i32 - 1,
        ),
    );
    let maximum = Pos(
        end.0.div_euclid(16),
        end.1.div_euclid(16),
        end.2.div_euclid(16),
    );
    for sy in minimum.1..=maximum.1 {
        for sz in minimum.2..=maximum.2 {
            for sx in minimum.0..=maximum.0 {
                let mut palette = vec![BlockState::air()];
                let mut lookup = BTreeMap::from([(BlockState::air().string(), 0)]);
                let mut values = Vec::with_capacity(4096);
                for i in 0..4096 {
                    let world = add(
                        Pos(sx * 16, sy * 16, sz * 16),
                        yzx_position(i, Size(16, 16, 16)),
                    );
                    let p = subtract(world, data.origin);
                    let state = data
                        .blocks
                        .get(&p)
                        .map(|b| b.state.clone())
                        .unwrap_or_else(BlockState::air);
                    values.push(palette_add(&mut palette, &mut lookup, &state));
                }
                regions.push(Nbt::Compound(compound([
                    ("X", Nbt::Int(sx)),
                    ("Y", Nbt::Int(sy)),
                    ("Z", Nbt::Int(sz)),
                    (
                        "BlockStates",
                        Nbt::Compound(compound([
                            (
                                "palette",
                                Nbt::List(
                                    palette.iter().map(|v| palette_entry(v, false)).collect(),
                                ),
                            ),
                            ("data", Nbt::LongArray(pack(&values, palette.len(), true))),
                        ])),
                    ),
                ])));
            }
        }
    }
    let mut root = compound([
        ("BlockRegion", Nbt::List(regions)),
        (
            "DataVersion",
            data.metadata
                .get("DataVersion")
                .cloned()
                .unwrap_or(Nbt::Int(4556)),
        ),
    ]);
    root.insert("Entities".into(), entity_list(&data.entities));
    let mut tile_entities = block_entities(data, false);
    for nbt in &mut tile_entities {
        let p = add(position(nbt)?, data.origin);
        for (key, value) in [("x", p.0), ("y", p.1), ("z", p.2)] {
            nbt.insert(key.into(), Nbt::Int(value));
        }
    }
    root.insert("BlockEntities".into(), entity_list(&tile_entities));
    let payload = gzip_nbt("", root)?;
    let header = bytes(&data.metadata, "AxiomHeader").unwrap_or(nbt::encode(
        "",
        &Nbt::Compound(compound([
            ("Version", Nbt::Int(2)),
            ("Name", Nbt::String("RustWaterStructure".into())),
            ("Author", Nbt::String("".into())),
            ("ContainsAir", Nbt::Byte(1)),
            ("BlockCount", Nbt::Int(data.non_air_count() as i32)),
        ])),
        Encoding::BigEndian,
    )?);
    let thumbnail = bytes(&data.metadata, "AxiomThumbnail").unwrap_or_default();
    let mut writer = nbt::Writer::new(Encoding::BigEndian);
    writer.int(0x0ae5bb36);
    for value in [&header, &thumbnail, &payload] {
        writer.int(i32::try_from(value.len()).map_err(|_| invalid("Axiom 数据过大"))?);
        writer.data.extend_from_slice(value);
    }
    Ok(writer.data)
}
