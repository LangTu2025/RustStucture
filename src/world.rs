//! Bedrock 世界目录、子区块和 MCWorld 容器。
pub mod cache;
pub mod dense_paged;
use crate::nbt::{self, Encoding, Nbt, Reader};
use crate::{Block, BlockState, Error, FormatId, Pos, Result, Size, StructureData, blocks};
use bedrock_leveldb::{
    CompressionPolicy, Db, OpenOptions, VisitorControl, WriteBatch, WriteOptions,
};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Cursor, Seek, Write};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const WORLD_WRITE_CACHE_SIZE: usize = 8 * 1024 * 1024;

#[derive(Default)]
pub struct Control {
    pub cancelled: AtomicBool,
    pub progress: Option<Arc<dyn Fn(u64, u64) + Send + Sync>>,
}

impl Control {
    pub fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn report(&self, completed: u64, total: u64) {
        if let Some(callback) = &self.progress {
            callback(completed, total);
        }
    }
}

fn db_error(error: impl std::fmt::Display) -> Error {
    Error::Other(format!("LevelDB: {error}"))
}

fn prepare_empty_directory(path: &Path) -> Result<()> {
    if path.exists() && fs::read_dir(path)?.next().is_some() {
        return Err(Error::invalid(
            "完整世界导出要求空目录，以免混入既有世界数据",
        ));
    }
    fs::create_dir_all(path)?;
    Ok(())
}

fn validate_world_extent(size: Size, start: Pos) -> Result<()> {
    for (origin, length) in [(start.0, size.0), (start.1, size.1), (start.2, size.2)] {
        if length == 0 || length > i32::MAX as u32 {
            return Err(Error::invalid("世界选区尺寸必须为有效的正32位整数"));
        }
        i32::try_from(i64::from(origin) + i64::from(length) - 1)
            .map_err(|_| Error::invalid("世界选区坐标溢出"))?;
    }
    if start.1 < -64 || i64::from(start.1) + i64::from(size.1) > 320 {
        return Err(Error::invalid("结构超出主世界高度 [-64,319]"));
    }
    Ok(())
}

fn chunk_key(x: i32, z: i32, tag: u8, sub_y: Option<i8>) -> Vec<u8> {
    let mut key = Vec::from(x.to_le_bytes());
    key.extend(z.to_le_bytes());
    key.push(tag);
    if let Some(y) = sub_y {
        key.push(y as u8);
    }
    key
}

fn palette_bits(count: usize) -> usize {
    let needed = usize::BITS as usize - count.saturating_sub(1).leading_zeros() as usize;
    [0, 1, 2, 3, 4, 5, 6, 8, 16]
        .into_iter()
        .find(|bits| *bits >= needed)
        .unwrap_or(16)
}

fn state_nbt(state: &BlockState) -> Nbt {
    Nbt::Compound(BTreeMap::from([
        ("name".into(), Nbt::String(state.name.clone())),
        ("states".into(), Nbt::Compound(state.properties.clone())),
        ("version".into(), Nbt::Int(18168865)),
    ]))
}

fn encode_storage(states: &[BlockState]) -> Result<Vec<u8>> {
    let mut palette = Vec::new();
    let mut lookup = BTreeMap::new();
    let mut indices = Vec::with_capacity(4096);
    for state in states {
        // 常见子区块只有少量状态，直接比较避免为每个体素序列化 NBT。
        // 大调色板仍使用哈希索引，限制线性扫描的最坏开销。
        let index = if palette.len() < 64 {
            palette
                .iter()
                .position(|candidate| candidate == state)
                .unwrap_or_else(|| {
                    let index = palette.len();
                    palette.push(state.clone());
                    index
                })
        } else {
            if lookup.is_empty() {
                for (index, state) in palette.iter().enumerate() {
                    lookup.insert(blocks::runtime_id(state), index);
                }
            }
            let hash = blocks::runtime_id(state);
            *lookup.entry(hash).or_insert_with(|| {
                let index = palette.len();
                palette.push(state.clone());
                index
            })
        };
        indices.push(index as u32);
    }
    let bits = palette_bits(palette.len());
    let mut bytes = vec![(bits << 1) as u8];
    if let Some(per_word) = 32usize.checked_div(bits) {
        for group in indices.chunks(per_word) {
            let word = group.iter().enumerate().fold(0u32, |word, (index, value)| {
                word | (value << (index * bits))
            });
            bytes.extend(word.to_le_bytes());
        }
        bytes.extend((palette.len() as u32).to_le_bytes());
    }
    for state in palette {
        bytes.extend(nbt::encode("", &state_nbt(&state), Encoding::LittleEndian)?);
    }
    Ok(bytes)
}

fn decode_storage(reader: &mut Reader<'_>) -> Result<Vec<BlockState>> {
    let header = reader.byte()?;
    let bits = (header >> 1) as usize;
    if ![0, 1, 2, 3, 4, 5, 6, 8, 16].contains(&bits) {
        return Err(Error::invalid(format!("不支持的子区块位宽 {bits}")));
    }
    let per_word = 32usize.checked_div(bits).unwrap_or(4096);
    let word_count = if bits == 0 {
        0
    } else {
        4096usize.div_ceil(per_word)
    };
    let words = reader
        .take(word_count * 4)?
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| u32::from_le_bytes(*bytes))
        .collect::<Vec<_>>();
    let count = if bits == 0 { 1 } else { reader.int()? };
    if count <= 0 || count > 65536 {
        return Err(Error::invalid("子区块调色板长度无效"));
    }
    let mut palette = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let (_, value) = reader.named()?;
        let value = value
            .as_compound()
            .ok_or_else(|| Error::invalid("子区块调色板不是 Compound"))?;
        let name = value
            .get("name")
            .and_then(Nbt::as_str)
            .ok_or_else(|| Error::invalid("缺少方块名称"))?;
        let properties = value
            .get("states")
            .and_then(Nbt::as_compound)
            .cloned()
            .unwrap_or_default();
        let version = value.get("version").and_then(Nbt::as_i64).unwrap_or(0);
        let version =
            i32::try_from(version).map_err(|_| Error::invalid("方块状态版本超出32位范围"))?;
        palette.push(blocks::bedrock_versioned(name, properties, version));
    }
    let mask = if bits == 0 { 0 } else { (1u32 << bits) - 1 };
    (0..4096)
        .map(|index| {
            let palette_index = if bits == 0 {
                0
            } else {
                (words[index / per_word] >> ((index % per_word) * bits)) & mask
            };
            palette
                .get(palette_index as usize)
                .cloned()
                .ok_or_else(|| Error::invalid("子区块调色板索引越界"))
        })
        .collect()
}

fn decode_subchunk(bytes: &[u8], key_y: i8) -> Result<(i32, Vec<Vec<BlockState>>)> {
    let mut reader = Reader::new(bytes, Encoding::LittleEndian);
    let version = reader.byte()?;
    let layers = match version {
        1 => 1,
        8 | 9 => reader.byte()?,
        _ => return Err(Error::Unsupported(format!("子区块版本 {version}"))),
    };
    let y = if version == 9 {
        reader.byte()? as i8
    } else {
        key_y
    };
    let mut result = Vec::with_capacity(layers as usize);
    for _ in 0..layers {
        result.push(decode_storage(&mut reader)?);
    }
    Ok((i32::from(y), result))
}

pub fn selection_from_name(text: &str) -> Option<(Pos, Pos)> {
    let mut selection = None;
    for part in text.split("@[").skip(1) {
        let Some((first, rest)) = part.split_once("]~[") else {
            continue;
        };
        let Some((second, _)) = rest.split_once(']') else {
            continue;
        };
        let parse = |text: &str| {
            let values = text
                .split(',')
                .map(str::parse::<i32>)
                .collect::<std::result::Result<Vec<_>, _>>()
                .ok()?;
            (values.len() == 3).then(|| Pos(values[0], values[1], values[2]))
        };
        if let (Some(a), Some(b)) = (parse(first), parse(second)) {
            selection = Some((a, b));
        }
    }
    selection
}

fn ordered_selection(selection: (Pos, Pos)) -> (Pos, Pos) {
    let (a, b) = selection;
    (
        Pos(a.0.min(b.0), a.1.min(b.1), a.2.min(b.2)),
        Pos(a.0.max(b.0), a.1.max(b.1), a.2.max(b.2)),
    )
}

pub fn read_directory(path: &Path, selection: Option<(Pos, Pos)>) -> Result<StructureData> {
    let options = OpenOptions {
        read_only: true,
        create_if_missing: false,
        ..OpenOptions::default()
    };
    let db = Db::open(path.join("db"), options).map_err(db_error)?;
    let mut data = StructureData::new(FormatId::MCWorld);
    let level_dat = fs::read(path.join("level.dat"))?;
    if level_dat.len() < 8 {
        return Err(Error::invalid("level.dat 截断"));
    }
    let (_, root) = nbt::decode(&level_dat[8..], Encoding::LittleEndian)?;
    data.metadata = root
        .as_compound()
        .cloned()
        .ok_or_else(|| Error::invalid("level.dat 根标签无效"))?;
    let selection = selection
        .or_else(|| {
            data.metadata
                .get("LevelName")
                .and_then(Nbt::as_str)
                .and_then(selection_from_name)
        })
        .ok_or_else(|| Error::invalid("无法从文件名或世界名称中解析坐标信息；请指定选区"))?;
    let (min, max) = ordered_selection(selection);
    data.origin = min;
    data.size = Size(
        (i64::from(max.0) - i64::from(min.0) + 1)
            .try_into()
            .map_err(|_| Error::invalid("选区过宽"))?,
        (i64::from(max.1) - i64::from(min.1) + 1)
            .try_into()
            .map_err(|_| Error::invalid("选区过高"))?,
        (i64::from(max.2) - i64::from(min.2) + 1)
            .try_into()
            .map_err(|_| Error::invalid("选区过长"))?,
    );
    db.for_each_entry(Default::default(), |key, bytes| {
        if key.len() != 9 && key.len() != 10 {
            return Ok(VisitorControl::Continue);
        }
        let x = i32::from_le_bytes(key[..4].try_into().unwrap());
        let z = i32::from_le_bytes(key[4..8].try_into().unwrap());
        if x < min.0.div_euclid(16)
            || x > max.0.div_euclid(16)
            || z < min.2.div_euclid(16)
            || z > max.2.div_euclid(16)
        {
            return Ok(VisitorControl::Continue);
        }
        let convert_error = |error: Error| {
            bedrock_leveldb::LevelDbError::from(std::io::Error::other(error.to_string()))
        };
        if key[8] == b'/' && key.len() == 10 {
            let (sub_y, layers) = decode_subchunk(bytes, key[9] as i8).map_err(convert_error)?;
            for index in 0..4096 {
                let world_pos = Pos(
                    x * 16 + (index >> 8) as i32,
                    sub_y * 16 + (index & 15) as i32,
                    z * 16 + ((index >> 4) & 15) as i32,
                );
                if world_pos.0 < min.0
                    || world_pos.0 > max.0
                    || world_pos.1 < min.1
                    || world_pos.1 > max.1
                    || world_pos.2 < min.2
                    || world_pos.2 > max.2
                {
                    continue;
                }
                let state = layers
                    .first()
                    .map_or_else(BlockState::air, |layer| layer[index].clone());
                let extra = layers
                    .get(1)
                    .map(|layer| layer[index].clone())
                    .filter(|state| !state.is_air());
                if state.is_air() && extra.is_none() {
                    continue;
                }
                let position = Pos(
                    world_pos.0 - min.0,
                    world_pos.1 - min.1,
                    world_pos.2 - min.2,
                );
                let entry = data
                    .blocks
                    .entry(position)
                    .or_insert_with(|| Block::new(BlockState::air()));
                entry.state = state;
                entry.extra = extra;
            }
        } else if key[8] == b'1' {
            let mut reader = Reader::new(bytes, Encoding::LittleEndian);
            while reader.pos < reader.data.len() {
                let (_, value) = reader.named().map_err(convert_error)?;
                let Some(mut nbt) = value.as_compound().cloned() else {
                    continue;
                };
                let coordinate = |name: &str| {
                    nbt.get(name)
                        .and_then(Nbt::as_i64)
                        .and_then(|value| i32::try_from(value).ok())
                };
                let (Some(x), Some(y), Some(z)) =
                    (coordinate("x"), coordinate("y"), coordinate("z"))
                else {
                    continue;
                };
                if x < min.0 || x > max.0 || y < min.1 || y > max.1 || z < min.2 || z > max.2 {
                    continue;
                }
                let position = Pos(x - min.0, y - min.1, z - min.2);
                nbt.insert("x".into(), Nbt::Int(position.0));
                nbt.insert("y".into(), Nbt::Int(position.1));
                nbt.insert("z".into(), Nbt::Int(position.2));
                data.blocks
                    .entry(position)
                    .or_insert_with(|| Block::new(BlockState::air()))
                    .nbt = Some(nbt);
            }
        }
        Ok(VisitorControl::Continue)
    })
    .map_err(db_error)?;
    Ok(data)
}

pub fn write_directory(
    path: &Path,
    data: &StructureData,
    start: Pos,
    control: Option<&Control>,
) -> Result<()> {
    validate_world_extent(data.size, start)?;
    prepare_empty_directory(path)?;
    let db = Db::open(
        path.join("db"),
        OpenOptions {
            compression_policy: CompressionPolicy::Snappy,
            cache_size: WORLD_WRITE_CACHE_SIZE,
            ..OpenOptions::default()
        },
    )
    .map_err(db_error)?;
    type SubchunkEntries<'a> = BTreeMap<i32, Vec<(Pos, &'a Block)>>;
    let mut chunks: BTreeMap<(i32, i32), SubchunkEntries<'_>> = BTreeMap::new();
    for (position, block) in &data.blocks {
        let position = Pos(
            position
                .0
                .checked_add(start.0)
                .ok_or_else(|| Error::invalid("X 坐标溢出"))?,
            position
                .1
                .checked_add(start.1)
                .ok_or_else(|| Error::invalid("Y 坐标溢出"))?,
            position
                .2
                .checked_add(start.2)
                .ok_or_else(|| Error::invalid("Z 坐标溢出"))?,
        );
        if !(-64..=319).contains(&position.1) {
            return Err(Error::invalid("方块超出主世界高度 [-64,319]"));
        }
        chunks
            .entry(position.chunk())
            .or_default()
            .entry(position.1.div_euclid(16))
            .or_default()
            .push((position, block));
    }
    let total = chunks.len() as u64;
    if let Some(control) = control {
        control.check()?;
        control.report(0, total);
    }
    for (completed, ((cx, cz), subs)) in chunks.into_iter().enumerate() {
        if let Some(control) = control {
            control.check()?;
        }
        let mut batch = WriteBatch::default();
        batch.put(chunk_key(cx, cz, b',', None), vec![41]);
        batch.put(chunk_key(cx, cz, b'6', None), 2i32.to_le_bytes().to_vec());
        let mut block_entities = Vec::new();
        for (sy, entries) in subs {
            let has_extra = entries.iter().any(|(_, block)| block.extra.is_some());
            let mut layers = vec![vec![BlockState::air(); 4096]; if has_extra { 2 } else { 1 }];
            for (position, block) in entries {
                let index = ((position.0.rem_euclid(16) << 8)
                    | (position.2.rem_euclid(16) << 4)
                    | position.1.rem_euclid(16)) as usize;
                layers[0][index] = block.state.clone();
                if let Some(extra) = &block.extra {
                    layers[1][index] = extra.clone();
                }
                if let Some(nbt) = &block.nbt {
                    block_entities.extend(nbt::encode_compound_with_position(
                        "",
                        nbt,
                        (position.0, position.1, position.2),
                        Encoding::LittleEndian,
                    )?);
                }
            }
            let mut encoded = vec![9, layers.len() as u8, sy as i8 as u8];
            for layer in layers {
                encoded.extend(encode_storage(&layer)?);
            }
            batch.put(chunk_key(cx, cz, b'/', Some(sy as i8)), encoded);
        }
        if !block_entities.is_empty() {
            batch.put(chunk_key(cx, cz, b'1', None), block_entities);
        }
        db.write(batch, WriteOptions::default()).map_err(db_error)?;
        if let Some(control) = control {
            control.report(completed as u64 + 1, total);
        }
    }
    if total > 0 {
        db.flush().map_err(db_error)?;
    }
    write_metadata(path, data.size, start, &data.metadata)
}

pub fn write_metadata(
    path: &Path,
    size: Size,
    start: Pos,
    source_metadata: &crate::nbt::Compound,
) -> Result<()> {
    validate_world_extent(size, start)?;
    let end = Pos(
        start
            .0
            .checked_add(size.0.saturating_sub(1) as i32)
            .ok_or_else(|| Error::invalid("X 范围溢出"))?,
        start
            .1
            .checked_add(size.1.saturating_sub(1) as i32)
            .ok_or_else(|| Error::invalid("Y 范围溢出"))?,
        start
            .2
            .checked_add(size.2.saturating_sub(1) as i32)
            .ok_or_else(|| Error::invalid("Z 范围溢出"))?,
    );
    let name = format!(
        "RustWaterStructure@[{}, {}, {}]~[{}, {}, {}]",
        start.0, start.1, start.2, end.0, end.1, end.2
    )
    .replace(", ", ",");
    let mut metadata = source_metadata.clone();
    metadata.insert("LevelName".into(), Nbt::String(name.clone()));
    metadata
        .entry("StorageVersion".into())
        .or_insert(Nbt::Int(10));
    metadata
        .entry("NetworkVersion".into())
        .or_insert(Nbt::Int(786));
    metadata.entry("Generator".into()).or_insert(Nbt::Int(2));
    metadata.entry("GameType".into()).or_insert(Nbt::Int(1));
    metadata.entry("SpawnX".into()).or_insert(Nbt::Int(0));
    metadata.entry("SpawnY".into()).or_insert(Nbt::Int(64));
    metadata.entry("SpawnZ".into()).or_insert(Nbt::Int(0));
    metadata.entry("RandomSeed".into()).or_insert(Nbt::Long(0));
    let payload = nbt::encode("", &Nbt::Compound(metadata), Encoding::LittleEndian)?;
    let mut level_dat = Vec::from(10u32.to_le_bytes());
    level_dat.extend((payload.len() as u32).to_le_bytes());
    level_dat.extend(payload);
    fs::write(path.join("level.dat"), level_dat)?;
    fs::write(path.join("levelname.txt"), name)?;
    Ok(())
}

pub fn decode_archive(bytes: &[u8], selection: Option<(Pos, Pos)>) -> Result<StructureData> {
    let directory = tempfile::tempdir()?;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| Error::invalid(error.to_string()))?;
    archive
        .extract(directory.path())
        .map_err(|error| Error::invalid(error.to_string()))?;
    read_directory(directory.path(), selection)
}

pub fn encode_archive(data: &StructureData) -> Result<Vec<u8>> {
    let directory = tempfile::tempdir()?;
    write_directory(directory.path(), data, Pos(0, -64, 0), None)?;
    encode_directory(directory.path())
}

pub fn encode_directory(directory: &Path) -> Result<Vec<u8>> {
    Ok(write_directory_archive(directory, Cursor::new(Vec::new()))?.into_inner())
}

pub fn write_directory_archive<W: Write + Seek>(directory: &Path, writer: W) -> Result<W> {
    let mut archive = zip::ZipWriter::new(writer);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    fn visit<W: Write + Seek>(
        path: &Path,
        root: &Path,
        archive: &mut zip::ZipWriter<W>,
        options: zip::write::SimpleFileOptions,
    ) -> Result<()> {
        let mut entries = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            if entry.file_type()?.is_dir() {
                visit(&entry.path(), root, archive, options)?;
            } else {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .map_err(|error| Error::invalid(error.to_string()))?
                    .to_string_lossy()
                    .replace('\\', "/");
                archive
                    .start_file(relative, options)
                    .map_err(|error| Error::Other(error.to_string()))?;
                let mut file = File::open(entry.path())?;
                std::io::copy(&mut file, archive)?;
            }
        }
        Ok(())
    }
    visit(directory, directory, &mut archive, options)?;
    archive
        .finish()
        .map_err(|error| Error::Other(error.to_string()))
}

/// 从紧凑索引直接写世界，临时工作集限制为一个子区块及 LevelDB 写缓冲。
pub fn write_dense_directory(
    path: &Path,
    data: &crate::formats::java::dense::DenseStructure,
    start: Pos,
    control: Option<&Control>,
) -> Result<()> {
    write_dense_source_directory(path, data, start, control)
}

pub fn write_dense_source_directory(
    path: &Path,
    data: &impl crate::formats::java::dense::DenseSource,
    start: Pos,
    control: Option<&Control>,
) -> Result<()> {
    if start.0.rem_euclid(16) != 0 || start.1.rem_euclid(16) != 0 || start.2.rem_euclid(16) != 0 {
        return Err(Error::invalid("紧凑导入起点必须按 16 格对齐"));
    }
    let size = data.size();
    validate_world_extent(size, start)?;
    let top = i64::from(start.1) + i64::from(size.1);
    if start.1 < -64 || top > 320 {
        return Err(Error::invalid("结构超出主世界高度 [-64,319]"));
    }
    prepare_empty_directory(path)?;
    let db = Db::open(
        path.join("db"),
        OpenOptions {
            compression_policy: CompressionPolicy::Snappy,
            cache_size: WORLD_WRITE_CACHE_SIZE,
            ..OpenOptions::default()
        },
    )
    .map_err(db_error)?;
    let x_chunks = size.0.div_ceil(16);
    let z_chunks = size.2.div_ceil(16);
    let total = u64::from(x_chunks) * u64::from(z_chunks);
    let mut local_palette_indices = vec![u32::MAX; data.palette().len()];
    let nbt_chunks = data.block_nbt().iter().fold(
        BTreeMap::<_, Vec<_>>::new(),
        |mut chunks, (position, nbt)| {
            chunks
                .entry(position.chunk())
                .or_default()
                .push((*position, nbt));
            chunks
        },
    );
    if let Some(control) = control {
        control.check()?;
        control.report(0, total);
    }
    for cx in 0..x_chunks {
        for cz in 0..z_chunks {
            if let Some(control) = control {
                control.check()?;
            }
            let world_cx = start.0 / 16 + cx as i32;
            let world_cz = start.2 / 16 + cz as i32;
            let mut batch = WriteBatch::default();
            batch.put(chunk_key(world_cx, world_cz, b',', None), vec![41]);
            batch.put(
                chunk_key(world_cx, world_cz, b'6', None),
                2i32.to_le_bytes().to_vec(),
            );
            for sy in 0..size.1.div_ceil(16) {
                let world_sy = start.1 / 16 + sy as i32;
                let mut layers = Vec::new();
                for layer in 0..=u8::from(data.has_extra()) {
                    let mut palette = vec![BlockState::air()];
                    let mut local_indices = Vec::with_capacity(4096);
                    let mut has_non_air = false;
                    let mut mapped_palette_indices = Vec::new();
                    let mut source_indices = [u32::MAX; 4096];
                    data.fill_subchunk(
                        Pos((cx * 16) as i32, (sy * 16) as i32, (cz * 16) as i32),
                        layer,
                        &mut source_indices,
                    )?;
                    for source_index in source_indices {
                        let Some(state) = data
                            .palette()
                            .get(source_index as usize)
                            .filter(|state| !state.is_air())
                        else {
                            local_indices.push(0);
                            continue;
                        };
                        has_non_air = true;
                        let source_index = source_index as usize;
                        let index = if local_palette_indices[source_index] == u32::MAX {
                            let index = palette.len() as u32;
                            palette.push(state.clone());
                            local_palette_indices[source_index] = index;
                            mapped_palette_indices.push(source_index);
                            index
                        } else {
                            local_palette_indices[source_index]
                        };
                        local_indices.push(index);
                    }
                    for source_index in mapped_palette_indices {
                        local_palette_indices[source_index] = u32::MAX;
                    }
                    layers.push((palette, local_indices, has_non_air));
                }
                if !layers.iter().any(|(_, _, has_blocks)| *has_blocks) {
                    continue;
                }
                let mut bytes = vec![9, layers.len() as u8, world_sy as i8 as u8];
                for (palette, indices, _) in layers {
                    bytes.extend(encode_palette_indices(&palette, &indices)?);
                }
                batch.put(
                    chunk_key(world_cx, world_cz, b'/', Some(world_sy as i8)),
                    bytes,
                );
            }
            if let Some(entries) = nbt_chunks.get(&(cx as i32, cz as i32)) {
                let mut bytes = Vec::new();
                for (position, nbt) in entries {
                    bytes.extend(nbt::encode_compound_with_position(
                        "",
                        nbt,
                        (
                            start.0 + position.0,
                            start.1 + position.1,
                            start.2 + position.2,
                        ),
                        Encoding::LittleEndian,
                    )?);
                }
                batch.put(chunk_key(world_cx, world_cz, b'1', None), bytes);
            }
            db.write(batch, WriteOptions::default()).map_err(db_error)?;
            if let Some(control) = control {
                control.report(
                    u64::from(cx) * u64::from(z_chunks) + u64::from(cz) + 1,
                    total,
                );
            }
        }
    }
    db.flush().map_err(db_error)?;
    write_metadata(path, size, start, &BTreeMap::new())
}

fn encode_palette_indices(palette: &[BlockState], indices: &[u32]) -> Result<Vec<u8>> {
    let bits = palette_bits(palette.len());
    let mut bytes = vec![(bits << 1) as u8];
    if let Some(per_word) = 32usize.checked_div(bits) {
        for entries in indices.chunks(per_word) {
            let word = entries
                .iter()
                .enumerate()
                .fold(0u32, |word, (index, value)| {
                    word | (value << (index * bits))
                });
            bytes.extend(word.to_le_bytes());
        }
        bytes.extend((palette.len() as u32).to_le_bytes());
    }
    for state in palette {
        bytes.extend(nbt::encode("", &state_nbt(state), Encoding::LittleEndian)?);
    }
    Ok(bytes)
}
