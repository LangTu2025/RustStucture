//! 大型结构的两遍流式读取：只展开紧凑索引，不为每个方块分配树节点。
use super::*;
use crate::world::dense_paged::{IndexLayout, PagedIndices, RawIndices};
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

pub struct DenseStructure {
    pub size: Size,
    pub origin: Pos,
    pub palette: Vec<BlockState>,
    /// Y→Z→X 布局，X 最快。
    pub indices: Vec<u32>,
    pub extra_indices: Option<Vec<u32>>,
    pub nbt: BTreeMap<Pos, Compound>,
    pub loss: DenseLoss,
}

#[derive(Debug, Default)]
pub struct DenseLoss {
    pub entities: u64,
    pub metadata: bool,
}

pub trait DenseSource {
    fn size(&self) -> Size;
    fn origin(&self) -> Pos;
    fn palette(&self) -> &[BlockState];
    fn block_nbt(&self) -> &BTreeMap<Pos, Compound>;
    fn loss(&self) -> &DenseLoss;
    fn has_extra(&self) -> bool;
    /// 返回调色板索引，X→Z→Y布局；缺失位置使用 u32::MAX，原点需非负且16对齐。
    fn fill_subchunk(&self, origin: Pos, layer: u8, output: &mut [u32; 4096]) -> Result<()>;
}

pub struct PagedStructure {
    pub size: Size,
    pub origin: Pos,
    pub palette: Vec<BlockState>,
    pub nbt: BTreeMap<Pos, Compound>,
    pub loss: DenseLoss,
    indices: PagedIndices,
    extras: Option<PagedIndices>,
}

pub enum BoundedStructure {
    Memory(DenseStructure),
    Paged(PagedStructure),
}

impl BoundedStructure {
    pub fn is_paged(&self) -> bool {
        matches!(self, Self::Paged(_))
    }

    pub fn temporary_bytes(&self) -> u64 {
        match self {
            Self::Memory(_) => 0,
            Self::Paged(data) => {
                data.indices.bytes() + data.extras.as_ref().map_or(0, PagedIndices::bytes)
            }
        }
    }
}

impl DenseSource for DenseStructure {
    fn size(&self) -> Size {
        self.size
    }
    fn origin(&self) -> Pos {
        self.origin
    }
    fn palette(&self) -> &[BlockState] {
        &self.palette
    }
    fn block_nbt(&self) -> &BTreeMap<Pos, Compound> {
        &self.nbt
    }
    fn loss(&self) -> &DenseLoss {
        &self.loss
    }
    fn has_extra(&self) -> bool {
        self.extra_indices.is_some()
    }
    fn fill_subchunk(&self, origin: Pos, layer: u8, output: &mut [u32; 4096]) -> Result<()> {
        if origin.0 < 0
            || origin.1 < 0
            || origin.2 < 0
            || origin.0 % 16 != 0
            || origin.1 % 16 != 0
            || origin.2 % 16 != 0
        {
            return Err(invalid("子区块读取要求非负且16对齐的局部坐标"));
        }
        output.fill(u32::MAX);
        let indices = match layer {
            0 => Some(&self.indices),
            1 => self.extra_indices.as_ref(),
            _ => None,
        };
        let Some(indices) = indices else {
            return Ok(());
        };
        for x in 0..16 {
            for z in 0..16 {
                for y in 0..16 {
                    let p = Pos(origin.0 + x, origin.1 + y, origin.2 + z);
                    if p.0 < 0
                        || p.1 < 0
                        || p.2 < 0
                        || p.0 as u32 >= self.size.0
                        || p.1 as u32 >= self.size.1
                        || p.2 as u32 >= self.size.2
                    {
                        continue;
                    }
                    let source = (p.1 as usize * self.size.2 as usize + p.2 as usize)
                        * self.size.0 as usize
                        + p.0 as usize;
                    output[(x * 256 + z * 16 + y) as usize] = indices[source];
                }
            }
        }
        Ok(())
    }
}

impl DenseSource for PagedStructure {
    fn size(&self) -> Size {
        self.size
    }
    fn origin(&self) -> Pos {
        self.origin
    }
    fn palette(&self) -> &[BlockState] {
        &self.palette
    }
    fn block_nbt(&self) -> &BTreeMap<Pos, Compound> {
        &self.nbt
    }
    fn loss(&self) -> &DenseLoss {
        &self.loss
    }
    fn has_extra(&self) -> bool {
        self.extras.is_some()
    }
    fn fill_subchunk(&self, origin: Pos, layer: u8, output: &mut [u32; 4096]) -> Result<()> {
        match layer {
            0 => self.indices.fill_subchunk(origin, output),
            1 => match &self.extras {
                Some(indices) => indices.fill_subchunk(origin, output),
                None => {
                    output.fill(u32::MAX);
                    Ok(())
                }
            },
            _ => {
                output.fill(u32::MAX);
                Ok(())
            }
        }
    }
}

impl DenseSource for BoundedStructure {
    fn size(&self) -> Size {
        match self {
            Self::Memory(data) => data.size(),
            Self::Paged(data) => data.size(),
        }
    }
    fn origin(&self) -> Pos {
        match self {
            Self::Memory(data) => data.origin(),
            Self::Paged(data) => data.origin(),
        }
    }
    fn palette(&self) -> &[BlockState] {
        match self {
            Self::Memory(data) => data.palette(),
            Self::Paged(data) => data.palette(),
        }
    }
    fn block_nbt(&self) -> &BTreeMap<Pos, Compound> {
        match self {
            Self::Memory(data) => data.block_nbt(),
            Self::Paged(data) => data.block_nbt(),
        }
    }
    fn loss(&self) -> &DenseLoss {
        match self {
            Self::Memory(data) => data.loss(),
            Self::Paged(data) => data.loss(),
        }
    }
    fn has_extra(&self) -> bool {
        match self {
            Self::Memory(data) => data.has_extra(),
            Self::Paged(data) => data.has_extra(),
        }
    }
    fn fill_subchunk(&self, origin: Pos, layer: u8, output: &mut [u32; 4096]) -> Result<()> {
        match self {
            Self::Memory(data) => data.fill_subchunk(origin, layer, output),
            Self::Paged(data) => data.fill_subchunk(origin, layer, output),
        }
    }
}

impl DenseStructure {
    pub fn from_file(path: impl AsRef<Path>, format: FormatId) -> Result<Self> {
        from_file(path, format)
    }

    pub fn state(&self, x: u32, y: u32, z: u32, layer: u8) -> Option<&BlockState> {
        if x >= self.size.0 || y >= self.size.1 || z >= self.size.2 {
            return None;
        }
        let index =
            (y as usize * self.size.2 as usize + z as usize) * self.size.0 as usize + x as usize;
        let id = match layer {
            0 => *self.indices.get(index)?,
            1 => *self.extra_indices.as_ref()?.get(index)?,
            _ => return None,
        };
        self.palette.get(id as usize)
    }
}

struct StreamNbt {
    input: Box<dyn Read>,
    little_endian: bool,
}

impl StreamNbt {
    fn open(path: &Path, format: FormatId) -> Result<Self> {
        let mut input = BufReader::new(File::open(path)?);
        let gzip = input.fill_buf()?.starts_with(&[0x1f, 0x8b]);
        let input: Box<dyn Read> = if gzip {
            Box::new(BufReader::new(GzDecoder::new(input)))
        } else {
            Box::new(input)
        };
        Ok(Self {
            input,
            little_endian: format == FormatId::MCStructure,
        })
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut bytes = [0; N];
        self.input.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.take::<1>()?[0])
    }

    fn short(&mut self) -> Result<i16> {
        let bytes = self.take()?;
        Ok(if self.little_endian {
            i16::from_le_bytes(bytes)
        } else {
            i16::from_be_bytes(bytes)
        })
    }

    fn int(&mut self) -> Result<i32> {
        let bytes = self.take()?;
        Ok(if self.little_endian {
            i32::from_le_bytes(bytes)
        } else {
            i32::from_be_bytes(bytes)
        })
    }

    fn long(&mut self) -> Result<i64> {
        let bytes = self.take()?;
        Ok(if self.little_endian {
            i64::from_le_bytes(bytes)
        } else {
            i64::from_be_bytes(bytes)
        })
    }

    fn length(&mut self) -> Result<usize> {
        usize::try_from(self.int()?).map_err(|_| invalid("NBT 长度为负数"))
    }

    fn string_bytes(&mut self) -> Result<Vec<u8>> {
        let length = self.short()? as u16 as usize;
        let mut bytes = vec![0; length];
        self.input.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    fn string(&mut self) -> Result<String> {
        String::from_utf8(self.string_bytes()?).map_err(|_| invalid("NBT 标签名称不是 UTF-8"))
    }

    fn discard(&mut self, length: usize) -> Result<()> {
        let consumed = std::io::copy(
            &mut self.input.by_ref().take(length as u64),
            &mut std::io::sink(),
        )?;
        if consumed != length as u64 {
            return Err(invalid("NBT 数据截断"));
        }
        Ok(())
    }

    fn skip(&mut self, tag: u8, depth: usize) -> Result<()> {
        if depth > 512 {
            return Err(invalid("NBT 嵌套过深"));
        }
        match tag {
            1..=6 => self.discard([0, 1, 2, 4, 8, 4, 8][tag as usize]),
            7 | 11 | 12 => {
                let length = self.length()?;
                let width = match tag {
                    7 => 1,
                    11 => 4,
                    _ => 8,
                };
                self.discard(
                    length
                        .checked_mul(width)
                        .ok_or_else(|| invalid("NBT 数组长度溢出"))?,
                )
            }
            8 => {
                let length = self.short()? as u16 as usize;
                self.discard(length)
            }
            9 => {
                let item = self.byte()?;
                let length = self.length()?;
                if (1..=6).contains(&item) {
                    let width = [0, 1, 2, 4, 8, 4, 8][item as usize];
                    return self.discard(
                        length
                            .checked_mul(width)
                            .ok_or_else(|| invalid("NBT 列表长度溢出"))?,
                    );
                }
                for _ in 0..length {
                    self.skip(item, depth + 1)?;
                }
                Ok(())
            }
            10 => {
                loop {
                    let tag = self.byte()?;
                    if tag == 0 {
                        break;
                    }
                    self.string_bytes()?;
                    self.skip(tag, depth + 1)?;
                }
                Ok(())
            }
            _ => Err(invalid("NBT 标签类型无效")),
        }
    }

    fn value(&mut self, tag: u8, depth: usize) -> Result<Nbt> {
        if depth > 512 {
            return Err(invalid("NBT 嵌套过深"));
        }
        Ok(match tag {
            1 => Nbt::Byte(self.byte()? as i8),
            2 => Nbt::Short(self.short()?),
            3 => Nbt::Int(self.int()?),
            4 => Nbt::Long(self.long()?),
            5 => Nbt::Float(f32::from_bits(self.int()? as u32)),
            6 => Nbt::Double(f64::from_bits(self.long()? as u64)),
            7 => {
                let length = self.length()?;
                let mut data = vec![0u8; length];
                self.input.read_exact(&mut data)?;
                Nbt::ByteArray(data.into_iter().map(|v| v as i8).collect())
            }
            8 => match String::from_utf8(self.string_bytes()?) {
                Ok(value) => Nbt::String(value),
                Err(error) => Nbt::RawString(error.into_bytes()),
            },
            9 => {
                let item = self.byte()?;
                let length = self.length()?;
                let mut values = Vec::with_capacity(length.min(4096));
                for _ in 0..length {
                    values.push(self.value(item, depth + 1)?);
                }
                Nbt::List(values)
            }
            10 => {
                let mut values = Compound::new();
                loop {
                    let tag = self.byte()?;
                    if tag == 0 {
                        break;
                    }
                    let name = self.string()?;
                    values.insert(name, self.value(tag, depth + 1)?);
                }
                Nbt::Compound(values)
            }
            11 => {
                let length = self.length()?;
                let mut values = Vec::with_capacity(length.min(4096));
                for _ in 0..length {
                    values.push(self.int()?);
                }
                Nbt::IntArray(values)
            }
            12 => {
                let length = self.length()?;
                let mut values = Vec::with_capacity(length.min(4096));
                for _ in 0..length {
                    values.push(self.long()?);
                }
                Nbt::LongArray(values)
            }
            _ => return Err(invalid("NBT 标签类型无效")),
        })
    }
}

#[derive(Clone, Copy)]
enum Scope {
    Root,
    JavaBlocks,
    Structure,
    Palette,
    DefaultPalette,
}

struct Loader {
    format: FormatId,
    detected_format: Option<FormatId>,
    size: Size,
    origin: Pos,
    palette: Vec<BlockState>,
    remap: Vec<u32>,
    indices: Vec<u32>,
    extras: Option<Vec<u32>>,
    tile_entities: Vec<Compound>,
    mc_nbt: BTreeMap<usize, Compound>,
    arrays_read: u8,
    loss: DenseLoss,
    volume: usize,
    disk: Option<RawIndices>,
    disk_extra: Option<RawIndices>,
}

impl Loader {
    fn new(format: FormatId) -> Self {
        Self {
            format,
            detected_format: None,
            size: Size::default(),
            origin: Pos::default(),
            palette: vec![BlockState::air()],
            remap: Vec::new(),
            indices: Vec::new(),
            extras: None,
            tile_entities: Vec::new(),
            mc_nbt: BTreeMap::new(),
            arrays_read: 0,
            loss: DenseLoss::default(),
            volume: 0,
            disk: None,
            disk_extra: None,
        }
    }

    fn walk(
        &mut self,
        reader: &mut StreamNbt,
        scope: Scope,
        load_arrays: bool,
        depth: usize,
    ) -> Result<()> {
        if depth > 512 {
            return Err(invalid("NBT 嵌套过深"));
        }
        loop {
            let tag = reader.byte()?;
            if tag == 0 {
                return Ok(());
            }
            let name = reader.string()?;
            if !load_arrays {
                let detected = match (scope, name.as_str(), tag) {
                    (Scope::Root, "Blocks", 7) => Some(FormatId::Schematic),
                    (Scope::Root, "Blocks", 10) => Some(FormatId::SchemV2),
                    (Scope::Root, "BlockData", 7) => Some(FormatId::SchemV1),
                    (Scope::Structure, "block_indices", 9) => Some(FormatId::MCStructure),
                    _ => None,
                };
                if detected.is_some() {
                    self.detected_format = detected;
                }
            }
            let child = match (scope, name.as_str(), tag) {
                (Scope::Root, "Schematic", 10) => Some(Scope::Root),
                (Scope::Root, "Blocks", 10) => Some(Scope::JavaBlocks),
                (Scope::Root, "structure", 10) => Some(Scope::Structure),
                (Scope::Structure, "palette", 10) => Some(Scope::Palette),
                (Scope::Palette, "default", 10) => Some(Scope::DefaultPalette),
                _ => None,
            };
            if let Some(child) = child {
                self.walk(reader, child, load_arrays, depth + 1)?;
                continue;
            }
            if load_arrays {
                match (scope, name.as_str(), tag) {
                    (Scope::Root, "Blocks", 7) if self.format == FormatId::Schematic => {
                        self.schematic_array(reader, true)?
                    }
                    (Scope::Root, "Data", 7) if self.format == FormatId::Schematic => {
                        self.schematic_array(reader, false)?
                    }
                    (Scope::Root, "BlockData", 7) if self.format == FormatId::SchemV1 => {
                        self.varints(reader)?
                    }
                    (Scope::JavaBlocks, "Data", 7) if self.format == FormatId::SchemV2 => {
                        self.varints(reader)?
                    }
                    (Scope::Structure, "block_indices", 9)
                        if self.format == FormatId::MCStructure =>
                    {
                        self.mc_indices(reader)?
                    }
                    _ => reader.skip(tag, depth)?,
                }
            } else {
                self.header(reader, scope, &name, tag, depth)?;
            }
        }
    }

    fn header(
        &mut self,
        reader: &mut StreamNbt,
        scope: Scope,
        name: &str,
        tag: u8,
        depth: usize,
    ) -> Result<()> {
        match (scope, name) {
            (Scope::Root, "Width" | "Height" | "Length") => {
                let number = reader
                    .value(tag, depth)?
                    .as_i64()
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or_else(|| invalid("结构尺寸无效"))?;
                match name {
                    "Width" => self.size.0 = number,
                    "Height" => self.size.1 = number,
                    _ => self.size.2 = number,
                }
            }
            (Scope::Root, "WEOriginX" | "WEOriginY" | "WEOriginZ") => {
                let number = reader
                    .value(tag, depth)?
                    .as_i64()
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or_else(|| invalid("结构原点无效"))?;
                match name {
                    "WEOriginX" => self.origin.0 = number,
                    "WEOriginY" => self.origin.1 = number,
                    _ => self.origin.2 = number,
                }
            }
            (Scope::Root, "Offset" | "structure_world_origin") => {
                self.origin = vector(&reader.value(tag, depth)?)?
            }
            (Scope::Root, "size") => {
                let size = vector(&reader.value(tag, depth)?)?;
                self.size = Size(size.0 as u32, size.1 as u32, size.2 as u32);
            }
            (Scope::Root | Scope::JavaBlocks, "Palette") if tag == 10 => {
                let value = reader.value(tag, depth)?;
                let palette = value.as_compound().ok_or_else(|| invalid("调色板无效"))?;
                let maximum = palette.values().filter_map(Nbt::as_i64).max().unwrap_or(-1);
                if !(0..=1_000_000).contains(&maximum) {
                    return Err(invalid("调色板索引范围无效"));
                }
                self.remap = vec![u32::MAX; maximum as usize + 1];
                for (state, id) in palette {
                    let id = id
                        .as_i64()
                        .and_then(|v| usize::try_from(v).ok())
                        .ok_or_else(|| invalid("调色板索引无效"))?;
                    self.remap[id] = self.palette.len() as u32;
                    self.palette.push(blocks::java(state));
                }
            }
            (Scope::DefaultPalette, "block_palette") if tag == 9 => {
                let value = reader.value(tag, depth)?;
                for state in value.as_list().ok_or_else(|| invalid("调色板列表无效"))? {
                    self.palette.push(palette_state(state, true)?);
                }
            }
            (Scope::Root | Scope::JavaBlocks, "TileEntities" | "BlockEntities") if tag == 9 => {
                let value = reader.value(tag, depth)?;
                self.tile_entities.extend(
                    value
                        .as_list()
                        .unwrap_or(&[])
                        .iter()
                        .filter_map(|v| v.as_compound().cloned()),
                );
            }
            (Scope::DefaultPalette, "block_position_data") if tag == 10 => {
                let value = reader.value(tag, depth)?;
                for (index, nbt) in value
                    .as_compound()
                    .ok_or_else(|| invalid("方块位置NBT无效"))?
                {
                    if let Some(nbt) = nbt
                        .as_compound()
                        .and_then(|v| v.get("block_entity_data"))
                        .and_then(Nbt::as_compound)
                    {
                        self.mc_nbt.insert(
                            index.parse().map_err(|_| invalid("方块NBT索引无效"))?,
                            nbt.clone(),
                        );
                    }
                }
            }
            (Scope::Root | Scope::Structure, "Entities" | "entities") if tag == 9 => {
                let item_tag = reader.byte()?;
                let count = reader.length()?;
                self.loss.entities += count as u64;
                for _ in 0..count {
                    reader.skip(item_tag, depth + 1)?;
                }
            }
            _ => {
                if matches!(scope, Scope::Root) && !matches!(name, "Blocks" | "Data" | "BlockData")
                {
                    self.loss.metadata = true;
                }
                reader.skip(tag, depth)?;
            }
        }
        Ok(())
    }

    fn schematic_array(&mut self, reader: &mut StreamNbt, blocks: bool) -> Result<()> {
        let length = reader.length()?;
        if length != self.volume {
            return Err(invalid("Schematic 数组长度与尺寸不符"));
        }
        if let Some(disk) = &mut self.disk {
            let mut bytes = vec![0u8; length.min(16384)];
            let mut values = vec![0u32; bytes.len()];
            for offset in (0..length).step_by(bytes.len()) {
                let count = bytes.len().min(length - offset);
                reader.input.read_exact(&mut bytes[..count])?;
                disk.read_values(offset, &mut values[..count])?;
                for (value, byte) in values[..count].iter_mut().zip(&bytes[..count]) {
                    *value |= if blocks {
                        u32::from(*byte) << 8
                    } else {
                        u32::from(*byte)
                    };
                }
                disk.write_values(offset, &values[..count])?;
            }
            self.arrays_read |= if blocks { 1 } else { 2 };
            return Ok(());
        }
        let mut buffer = [0u8; 65536];
        for chunk in self.indices.chunks_mut(buffer.len()) {
            reader.input.read_exact(&mut buffer[..chunk.len()])?;
            for (index, value) in chunk.iter_mut().zip(&buffer) {
                *index |= if blocks {
                    u32::from(*value) << 8
                } else {
                    u32::from(*value)
                };
            }
        }
        self.arrays_read |= if blocks { 1 } else { 2 };
        Ok(())
    }

    fn varints(&mut self, reader: &mut StreamNbt) -> Result<()> {
        let mut remaining = reader.length()?;
        if let Some(disk) = &mut self.disk {
            let mut buffer = vec![0u32; self.volume.min(16384)];
            for offset in (0..self.volume).step_by(buffer.len()) {
                let count = buffer.len().min(self.volume - offset);
                for value in &mut buffer[..count] {
                    *value = read_palette_varint(reader, &mut remaining, &self.remap)?;
                }
                disk.write_values(offset, &buffer[..count])?;
            }
            if remaining != 0 {
                return Err(invalid("方块 Varint 数量超过尺寸"));
            }
            self.arrays_read = 3;
            return Ok(());
        }
        for index in &mut self.indices {
            *index = read_palette_varint(reader, &mut remaining, &self.remap)?;
        }
        if remaining != 0 {
            return Err(invalid("方块 Varint 数量超过尺寸"));
        }
        self.arrays_read = 3;
        Ok(())
    }

    fn mc_indices(&mut self, reader: &mut StreamNbt) -> Result<()> {
        if reader.byte()? != 9 || reader.length()? != 2 {
            return Err(invalid("MCStructure 必须有两层索引列表"));
        }
        if self.disk.is_some() {
            return self.mc_paged_indices(reader);
        }
        for layer in 0..2 {
            if reader.byte()? != 3 || reader.length()? != self.indices.len() {
                return Err(invalid("MCStructure 索引列表长度无效"));
            }
            for i in 0..self.indices.len() {
                let value = reader.int()?;
                let id = if value == -1 {
                    if layer == 0 { 0 } else { u32::MAX }
                } else {
                    let id = u32::try_from(value)
                        .map_err(|_| invalid("MCStructure 索引无效"))?
                        .checked_add(1)
                        .ok_or_else(|| invalid("MCStructure 索引溢出"))?;
                    if id as usize >= self.palette.len() {
                        return Err(invalid("MCStructure 调色板索引越界"));
                    }
                    id
                };
                let p = xyz_position(i, self.size);
                let target = (p.1 as usize * self.size.2 as usize + p.2 as usize)
                    * self.size.0 as usize
                    + p.0 as usize;
                if layer == 0 {
                    self.indices[target] = id;
                } else if id != u32::MAX && !self.palette[id as usize].is_air() {
                    self.extras
                        .get_or_insert_with(|| vec![u32::MAX; self.indices.len()])[target] = id;
                }
            }
        }
        self.arrays_read = 3;
        Ok(())
    }

    fn mc_paged_indices(&mut self, reader: &mut StreamNbt) -> Result<()> {
        let mut buffer = vec![0u32; self.volume.min(16384)];
        for layer in 0..2 {
            if reader.byte()? != 3 || reader.length()? != self.volume {
                return Err(invalid("MCStructure 索引列表长度无效"));
            }
            for offset in (0..self.volume).step_by(buffer.len()) {
                let count = buffer.len().min(self.volume - offset);
                for value in &mut buffer[..count] {
                    let source = reader.int()?;
                    *value = if source == -1 {
                        if layer == 0 { 0 } else { u32::MAX }
                    } else {
                        let id = u32::try_from(source)
                            .map_err(|_| invalid("MCStructure 索引无效"))?
                            .checked_add(1)
                            .ok_or_else(|| invalid("MCStructure 索引溢出"))?;
                        let state = self
                            .palette
                            .get(id as usize)
                            .ok_or_else(|| invalid("MCStructure 调色板索引越界"))?;
                        if layer == 1 && state.is_air() {
                            u32::MAX
                        } else {
                            id
                        }
                    };
                }
                if layer == 0 {
                    self.disk
                        .as_mut()
                        .unwrap()
                        .write_values(offset, &buffer[..count])?;
                } else {
                    if self.disk_extra.is_none()
                        && buffer[..count].iter().any(|value| *value != u32::MAX)
                    {
                        let mut file = RawIndices::new(self.volume, IndexLayout::Xyz)?;
                        file.fill_prefix(offset, u32::MAX)?;
                        self.disk_extra = Some(file);
                    }
                    if let Some(file) = &mut self.disk_extra {
                        file.write_values(offset, &buffer[..count])?;
                    }
                }
            }
        }
        self.arrays_read = 3;
        Ok(())
    }
}

fn read_palette_varint(
    reader: &mut StreamNbt,
    remaining: &mut usize,
    remap: &[u32],
) -> Result<u32> {
    let mut value = 0u32;
    for shift in (0..35).step_by(7) {
        if *remaining == 0 {
            return Err(invalid("方块 Varint 数据截断"));
        }
        let byte = reader.byte()?;
        *remaining -= 1;
        if shift == 28 && byte > 15 {
            return Err(invalid("方块 Varint 溢出"));
        }
        value |= u32::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return remap
                .get(value as usize)
                .copied()
                .filter(|id| *id != u32::MAX)
                .ok_or_else(|| invalid("方块调色板索引越界"));
        }
    }
    Err(invalid("方块 Varint 无效"))
}

fn prepare_loader(path: &Path, format: FormatId) -> Result<Loader> {
    if !matches!(
        format,
        FormatId::Schematic | FormatId::SchemV1 | FormatId::SchemV2 | FormatId::MCStructure
    ) {
        return Err(Error::Unsupported(format!(
            "{} 暂无紧凑读取器",
            format.name()
        )));
    }
    let mut loader = Loader::new(format);
    let mut reader = StreamNbt::open(path, format)?;
    if reader.byte()? != 10 {
        return Err(invalid("根标签必须为 Compound"));
    }
    reader.string()?;
    loader.walk(&mut reader, Scope::Root, false, 0)?;
    if loader.detected_format != Some(format) {
        return Err(invalid("NBT 方块数组布局与请求的结构格式不符"));
    }
    if loader.size.0 == 0 || loader.size.1 == 0 || loader.size.2 == 0 {
        return Err(invalid("结构尺寸必须为正数"));
    }
    loader.volume =
        usize::try_from(loader.size.volume()).map_err(|_| invalid("结构体积超过平台索引范围"))?;
    Ok(loader)
}

fn load_arrays(path: &Path, loader: &mut Loader) -> Result<()> {
    let mut reader = StreamNbt::open(path, loader.format)?;
    if reader.byte()? != 10 {
        return Err(invalid("根标签必须为 Compound"));
    }
    reader.string()?;
    loader.walk(&mut reader, Scope::Root, true, 0)?;
    if loader.arrays_read != 3 {
        return Err(invalid("缺少完整方块数组"));
    }
    if loader.format == FormatId::Schematic {
        remap_schematic(loader)?;
    }
    Ok(())
}

fn remap_schematic(loader: &mut Loader) -> Result<()> {
    let mut mapping = vec![u32::MAX; 65536];
    let mut remap = |indices: &mut [u32]| {
        for index in indices {
            let key = *index as usize;
            if mapping[key] == u32::MAX {
                mapping[key] = loader.palette.len() as u32;
                loader
                    .palette
                    .push(blocks::from_schematic((key >> 8) as u8, key as u8));
            }
            *index = mapping[key];
        }
    };
    if let Some(disk) = &mut loader.disk {
        let mut buffer = vec![0; loader.volume.min(16384)];
        for offset in (0..loader.volume).step_by(buffer.len()) {
            let count = buffer.len().min(loader.volume - offset);
            disk.read_values(offset, &mut buffer[..count])?;
            remap(&mut buffer[..count]);
            disk.write_values(offset, &buffer[..count])?;
        }
    } else {
        remap(&mut loader.indices);
    }
    Ok(())
}

fn collect_nbt(loader: &mut Loader) -> Result<BTreeMap<Pos, Compound>> {
    let mut nbt = BTreeMap::new();
    for entity in loader.tile_entities.drain(..) {
        if let Ok(p) = position(&entity) {
            nbt.insert(p, entity);
        }
    }
    for (index, entity) in std::mem::take(&mut loader.mc_nbt) {
        if index >= loader.volume {
            return Err(invalid("方块实体索引越界"));
        }
        nbt.insert(xyz_position(index, loader.size), entity);
    }
    Ok(nbt)
}

fn memory_structure(path: &Path, mut loader: Loader) -> Result<DenseStructure> {
    loader
        .indices
        .try_reserve_exact(loader.volume)
        .map_err(|error| Error::Other(format!("无法分配紧凑方块索引: {error}")))?;
    loader.indices.resize(loader.volume, 0);
    load_arrays(path, &mut loader)?;
    let nbt = collect_nbt(&mut loader)?;
    Ok(DenseStructure {
        size: loader.size,
        origin: loader.origin,
        palette: loader.palette,
        indices: loader.indices,
        extra_indices: loader.extras,
        nbt,
        loss: loader.loss,
    })
}

pub fn from_file(path: impl AsRef<Path>, format: FormatId) -> Result<DenseStructure> {
    let path = path.as_ref();
    let loader = prepare_loader(path, format)?;
    memory_structure(path, loader)
}

pub fn from_file_bounded(
    path: impl AsRef<Path>,
    format: FormatId,
    limit_bytes: usize,
) -> Result<BoundedStructure> {
    let path = path.as_ref();
    let mut loader = prepare_loader(path, format)?;
    let bytes_per_value = if format == FormatId::MCStructure {
        8
    } else {
        4
    };
    let estimate = loader
        .volume
        .checked_mul(bytes_per_value)
        .ok_or_else(|| invalid("结构索引长度溢出"))?;
    if estimate <= limit_bytes {
        return memory_structure(path, loader).map(BoundedStructure::Memory);
    }
    let layout = if format == FormatId::MCStructure {
        IndexLayout::Xyz
    } else {
        IndexLayout::Yzx
    };
    loader.disk = Some(RawIndices::new(loader.volume, layout)?);
    load_arrays(path, &mut loader)?;
    let indices = loader
        .disk
        .take()
        .ok_or_else(|| invalid("缺少分页主层索引"))?
        .into_paged(loader.size, u32::MAX, limit_bytes)?;
    let extras = loader
        .disk_extra
        .take()
        .map(|indices| indices.into_paged(loader.size, u32::MAX, limit_bytes))
        .transpose()?;
    let nbt = collect_nbt(&mut loader)?;
    Ok(BoundedStructure::Paged(PagedStructure {
        size: loader.size,
        origin: loader.origin,
        palette: loader.palette,
        nbt,
        loss: loader.loss,
        indices,
        extras,
    }))
}
