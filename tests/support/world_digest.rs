use bedrock_leveldb::{Db, OpenOptions, VisitorControl};
use rust_water_structure::{
    BlockState, Error, Pos, Result, StructureData, blocks,
    nbt::{self, Compound, Encoding, Nbt, Reader},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

const BLOCK_DOMAIN: &[u8] = b"RustWaterStructure.blocks.v1\0";
const SUBCHUNK_DOMAIN: &[u8] = b"RustWaterStructure.subchunk.v1\0";
const NBT_DOMAIN: &[u8] = b"RustWaterStructure.block-nbt.v1\0";

/// 子区块排序采用 (维度, X, Y, Z)，内部采用 X→Z→Y，Y 最快。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SubPos(i32, i32, i32, i32);

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct WorldDigest {
    pub non_air: u64,
    pub extra_non_air: u64,
    pub subchunks: u64,
    pub blocks_sha256: String,
    pub nbt_sha256: String,
    pub nbt_count: u64,
}

struct SubDigest {
    primary: u64,
    extra: u64,
    hash: [u8; 32],
}

#[derive(Default)]
struct Collector {
    subchunks: BTreeMap<SubPos, SubDigest>,
    nbt: BTreeMap<SubPos, [u8; 32]>,
}

fn error(message: impl Into<String>) -> Error {
    Error::invalid(message)
}

fn db_error(message: impl std::fmt::Display) -> Error {
    Error::Other(format!("LevelDB: {message}"))
}

fn coordinate_bytes(position: SubPos) -> [u8; 16] {
    let mut bytes = [0; 16];
    for (axis, value) in [position.0, position.1, position.2, position.3]
        .into_iter()
        .enumerate()
    {
        bytes[axis * 4..axis * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

impl Collector {
    fn subchunk(&mut self, position: SubPos, layers: &[Vec<u32>], air: u32) -> Result<()> {
        if layers.len() > 2
            && layers[2..]
                .iter()
                .any(|layer| layer.iter().any(|id| *id != air))
        {
            return Err(Error::Unsupported(
                "摘要暂不支持包含第三层非空气方块的世界".into(),
            ));
        }
        let primary = layers
            .first()
            .map_or(0, |layer| layer.iter().filter(|id| **id != air).count())
            as u64;
        let extra = layers
            .get(1)
            .map_or(0, |layer| layer.iter().filter(|id| **id != air).count())
            as u64;
        if primary == 0 && extra == 0 {
            return Ok(());
        }
        let mut hash = Sha256::new();
        hash.update(SUBCHUNK_DOMAIN);
        let mut payload = [0u8; 4096 * 8];
        for (index, bytes) in payload.as_chunks_mut::<8>().0.iter_mut().enumerate() {
            for layer in 0..2 {
                let id = layers.get(layer).map_or(air, |values| values[index]);
                bytes[layer * 4..layer * 4 + 4].copy_from_slice(&id.to_le_bytes());
            }
        }
        hash.update(payload);
        let value = SubDigest {
            primary,
            extra,
            hash: hash.finalize().into(),
        };
        if let Some(previous) = self.subchunks.get(&position) {
            if previous.hash != value.hash {
                return Err(error("重复子区块键对应不同方块内容"));
            }
        } else {
            self.subchunks.insert(position, value);
        }
        Ok(())
    }

    fn block_nbt(&mut self, dimension: i32, position: Pos, nbt: Compound) -> Result<()> {
        let position = SubPos(dimension, position.0, position.1, position.2);
        // Compound 在所有嵌套层级均为 BTreeMap；数组/列表顺序与数值标签类型保持原样。
        let bytes = nbt::encode("", &Nbt::Compound(nbt), Encoding::LittleEndian)?;
        let hash: [u8; 32] = Sha256::digest(bytes).into();
        if let Some(previous) = self.nbt.get(&position) {
            if *previous != hash {
                return Err(error("同一坐标存在不同的方块 NBT"));
            }
        } else {
            self.nbt.insert(position, hash);
        }
        Ok(())
    }

    fn finish(self) -> WorldDigest {
        let mut blocks = Sha256::new();
        blocks.update(BLOCK_DOMAIN);
        let mut primary = 0;
        let mut extra = 0;
        for (position, value) in &self.subchunks {
            blocks.update(coordinate_bytes(*position));
            blocks.update(value.hash);
            primary += value.primary;
            extra += value.extra;
        }
        let mut nbt = Sha256::new();
        nbt.update(NBT_DOMAIN);
        for (position, hash) in &self.nbt {
            nbt.update(coordinate_bytes(*position));
            nbt.update(hash);
        }
        WorldDigest {
            non_air: primary,
            extra_non_air: extra,
            subchunks: self.subchunks.len() as u64,
            blocks_sha256: hex::encode(blocks.finalize()),
            nbt_sha256: hex::encode(nbt.finalize()),
            nbt_count: self.nbt.len() as u64,
        }
    }
}

fn storage(reader: &mut Reader<'_>) -> Result<Vec<u32>> {
    let header = reader.byte()?;
    if header & 1 != 0 {
        return Err(Error::Unsupported(
            "LevelDB 中发现网络 RuntimeID 调色板，不能与持久化 NBT 调色板直接等价比较".into(),
        ));
    }
    let bits = (header >> 1) as usize;
    if ![0, 1, 2, 3, 4, 5, 6, 8, 16].contains(&bits) {
        return Err(error(format!("无效子区块位宽 {bits}")));
    }
    let per_word = 32usize.checked_div(bits).unwrap_or(4096);
    let words = if bits == 0 {
        &[][..]
    } else {
        reader.take(4096usize.div_ceil(per_word) * 4)?
    };
    let count = if bits == 0 { 1 } else { reader.int()? };
    if !(1..=65536).contains(&count) {
        return Err(error("无效子区块调色板长度"));
    }
    let mut palette = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let (_, state) = reader.named()?;
        let state = state
            .as_compound()
            .ok_or_else(|| error("方块调色板条目不是 Compound"))?;
        let name = state
            .get("name")
            .and_then(Nbt::as_str)
            .ok_or_else(|| error("方块调色板缺少 name"))?;
        let properties = state
            .get("states")
            .and_then(Nbt::as_compound)
            .cloned()
            .ok_or_else(|| error("方块调色板缺少 states"))?;
        let state = BlockState {
            name: BlockState::new(name).name,
            properties,
        };
        palette.push(blocks::runtime_id(&state));
    }
    let mask = (1u32 << bits) - 1;
    let mut values = Vec::with_capacity(4096);
    for index in 0..4096 {
        let id = if bits == 0 {
            0
        } else {
            let offset = index / per_word * 4;
            let word = u32::from_le_bytes(words[offset..offset + 4].try_into().unwrap());
            ((word >> (index % per_word * bits)) & mask) as usize
        };
        values.push(
            *palette
                .get(id)
                .ok_or_else(|| error("子区块调色板索引越界"))?,
        );
    }
    Ok(values)
}

fn decode_subchunk(bytes: &[u8], key_y: i8) -> Result<(i32, Vec<Vec<u32>>)> {
    let mut reader = Reader::new(bytes, Encoding::LittleEndian);
    let version = reader.byte()?;
    let layer_count = match version {
        1 => 1,
        8 | 9 => reader.byte()?,
        _ => return Err(Error::Unsupported(format!("子区块版本 {version}"))),
    };
    let y = if version == 9 {
        reader.byte()? as i8
    } else {
        key_y
    };
    if version == 9 && y != key_y {
        return Err(error("子区块键与负载中的 Y 不一致"));
    }
    let mut layers = Vec::with_capacity(layer_count as usize);
    for _ in 0..layer_count {
        layers.push(storage(&mut reader)?);
    }
    Ok((i32::from(y), layers))
}

fn chunk_record(key: &[u8]) -> Option<(i32, i32, i32, u8, Option<i8>)> {
    let (dimension, tag_offset) = match key.len() {
        9 | 10 => (0, 8),
        13 | 14 => (i32::from_le_bytes(key[8..12].try_into().ok()?), 12),
        _ => return None,
    };
    let x = i32::from_le_bytes(key[0..4].try_into().ok()?);
    let z = i32::from_le_bytes(key[4..8].try_into().ok()?);
    Some((
        dimension,
        x,
        z,
        key[tag_offset],
        key.get(tag_offset + 1).copied().map(|value| value as i8),
    ))
}

fn scan_directory(path: &Path) -> Result<WorldDigest> {
    let options = OpenOptions {
        read_only: true,
        create_if_missing: false,
        ..OpenOptions::default()
    };
    let db = Db::open(path.join("db"), options).map_err(db_error)?;
    let air = blocks::runtime_id(&BlockState::air());
    let mut collector = Collector::default();
    db.for_each_entry(Default::default(), |key, bytes| {
        let mut operation = || -> Result<()> {
            let Some((dimension, x, z, tag, key_y)) = chunk_record(key) else {
                return Ok(());
            };
            if tag == b'/' {
                let key_y = key_y.ok_or_else(|| error("子区块键缺少 Y"))?;
                let (y, layers) = decode_subchunk(bytes, key_y)?;
                collector.subchunk(SubPos(dimension, x, y, z), &layers, air)?;
            } else if tag == b'1' && key_y.is_none() {
                let mut reader = Reader::new(bytes, Encoding::LittleEndian);
                while reader.pos < reader.data.len() {
                    let (_, value) = reader.named()?;
                    let nbt = value
                        .as_compound()
                        .cloned()
                        .ok_or_else(|| error("方块 NBT 根标签不是 Compound"))?;
                    let coordinate = |key| {
                        nbt.get(key)
                            .and_then(Nbt::as_i64)
                            .and_then(|value| i32::try_from(value).ok())
                            .ok_or_else(|| error("方块 NBT 缺少有效坐标"))
                    };
                    let position = Pos(coordinate("x")?, coordinate("y")?, coordinate("z")?);
                    if position.0.div_euclid(16) != x || position.2.div_euclid(16) != z {
                        return Err(error("方块 NBT 坐标不属于其 LevelDB 区块键"));
                    }
                    collector.block_nbt(dimension, position, nbt)?;
                }
            }
            Ok(())
        };
        operation().map_err(|error| {
            bedrock_leveldb::LevelDbError::from(std::io::Error::other(error.to_string()))
        })?;
        Ok(VisitorControl::Continue)
    })
    .map_err(db_error)?;
    Ok(collector.finish())
}

fn extract_archive(path: &Path, target: &Path) -> Result<PathBuf> {
    let mut archive =
        zip::ZipArchive::new(File::open(path)?).map_err(|error| Error::Other(error.to_string()))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| Error::Other(error.to_string()))?;
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| error("ZIP 条目路径越界"))?;
        let destination = target.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(destination)?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = File::create(destination)?;
        std::io::copy(&mut entry, &mut output)?;
        output.flush()?;
    }
    fn find(path: &Path, depth: usize) -> Result<Option<PathBuf>> {
        if path.join("db").is_dir() {
            return Ok(Some(path.to_path_buf()));
        }
        if depth > 3 {
            return Ok(None);
        }
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_dir()
                && let Some(found) = find(&entry.path(), depth + 1)?
            {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }
    find(target, 0)?.ok_or_else(|| error("压缩包中未找到世界 db 目录"))
}

pub fn digest_world(path: &Path) -> Result<WorldDigest> {
    if path.is_dir() {
        return scan_directory(path);
    }
    let directory = tempfile::tempdir()?;
    let path = extract_archive(path, directory.path())?;
    scan_directory(&path)
}

pub fn digest_structure(data: &StructureData, start: Pos) -> Result<WorldDigest> {
    let air = blocks::runtime_id(&BlockState::air());
    let mut subchunks: BTreeMap<SubPos, Vec<Vec<u32>>> = BTreeMap::new();
    let mut collector = Collector::default();
    for (p, block) in &data.blocks {
        let world = Pos(p.0 + start.0, p.1 + start.1, p.2 + start.2);
        let position = SubPos(
            0,
            world.0.div_euclid(16),
            world.1.div_euclid(16),
            world.2.div_euclid(16),
        );
        let index = ((world.0.rem_euclid(16) << 8)
            | (world.2.rem_euclid(16) << 4)
            | world.1.rem_euclid(16)) as usize;
        let primary = blocks::runtime_id(&block.state);
        let extra = block.extra.as_ref().map_or(air, blocks::runtime_id);
        if primary != air || extra != air {
            let layers = subchunks
                .entry(position)
                .or_insert_with(|| vec![vec![air; 4096]; 2]);
            layers[0][index] = primary;
            layers[1][index] = extra;
        }
        if let Some(nbt) = &block.nbt {
            let mut nbt = nbt.clone();
            for (key, value) in [("x", world.0), ("y", world.1), ("z", world.2)] {
                nbt.insert(key.into(), Nbt::Int(value));
            }
            collector.block_nbt(0, world, nbt)?;
        }
    }
    for (position, layers) in subchunks {
        collector.subchunk(position, &layers, air)?;
    }
    Ok(collector.finish())
}
