//! 稳定的公共读取、转换及损失报告接口。
use crate::nbt::{Compound, Nbt};
use crate::{
    Block, BlockState, Error, FormatId, Pos, Result, Size, StructureData, blocks, formats, world,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LossPolicy {
    Compatible,
    Strict,
}

#[derive(Debug, Default, Serialize)]
pub struct LossReport {
    pub extra_layers: u64,
    pub block_nbt: u64,
    pub entities: u64,
    pub block_states: u64,
    pub metadata: bool,
    pub bounds_or_origin: bool,
    pub details: Vec<String>,
}

impl LossReport {
    pub fn is_empty(&self) -> bool {
        self.extra_layers == 0
            && self.block_nbt == 0
            && self.entities == 0
            && self.block_states == 0
            && !self.metadata
            && !self.bounds_or_origin
    }
}

pub struct Encoded {
    pub bytes: Vec<u8>,
    pub report: LossReport,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Capabilities {
    pub read: bool,
    pub write: bool,
    pub extra_layers: bool,
    pub block_nbt: &'static str,
    pub entities: bool,
    pub declared_size: bool,
    pub origin: bool,
}

pub fn capabilities(format: FormatId) -> Capabilities {
    use FormatId::*;
    let block_nbt = match format {
        MCStructure | MCWorld | BDX | Construction | Schematic | SchemV1 | SchemV2 | Litematic
        | AxiomBP | MianYangV1 | MianYangV2 | MianYangV3 | MianYangV4 | GangBanV3 | GangBanV4
        | FuHongV2 => "all",
        GangBanV1 | GangBanV2 | IBImport => "command",
        GangBanV5 | GangBanV6 | GangBanV7 => "command-container",
        FuHongV3 | FuHongV4 | FuHongV5 | KBDX => "command-container-sign",
        _ => "none",
    };
    Capabilities {
        read: format != SIBI,
        write: format != SIBI,
        extra_layers: matches!(format, MCStructure | MCWorld | Construction),
        block_nbt,
        entities: matches!(
            format,
            Schematic | SchemV1 | SchemV2 | MCStructure | Litematic | Construction | AxiomBP
        ),
        declared_size: matches!(
            format,
            Schematic
                | SchemV1
                | SchemV2
                | Litematic
                | MCStructure
                | MCWorld
                | AxiomBP
                | Construction
                | BDX
                | MianYangV4
                | GangBanV1
                | GangBanV2
                | GangBanV3
                | CovStructure
        ),
        origin: !matches!(format, BDX | CovStructure | MCWorld | SIBI),
    }
}

pub fn decode(format: FormatId, bytes: &[u8]) -> Result<StructureData> {
    use FormatId::*;
    match format {
        SIBI => Err(Error::Unsupported("SIBI 缺少规范与样例，按约定跳过".into())),
        MCWorld => world::decode_archive(bytes, None),
        Schematic | SchemV1 | SchemV2 | Litematic | MCStructure | AxiomBP | Construction => {
            formats::java::decode(format, bytes)
        }
        BDX | MCFunction | KBDX | IBImport | BDS | BCF | TIBI | NexusNP => {
            formats::binary::decode(format, bytes)
        }
        _ => formats::json::decode(format, bytes),
    }
}

fn candidates(name: &str, bytes: &[u8]) -> Vec<FormatId> {
    use FormatId::*;
    let extension = Path::new(name)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut result = match extension.as_str() {
        "mcworld" | "zip" => vec![MCWorld],
        "kbdx" => vec![KBDX],
        "tibi" => vec![TIBI],
        "mcfunction" | "txt" => vec![MCFunction],
        "covstructure" => vec![CovStructure],
        "np" => vec![NexusNP],
        _ => Vec::new(),
    };
    match bytes {
        [0x1f, 0x8b, ..] => result.extend([Litematic, SchemV1, SchemV2, Schematic, MianYangV4]),
        [0x0a, 0xe5, ..] => result.push(AxiomBP),
        [0x0a, ..] => result.extend([MCStructure, Schematic, SchemV1, SchemV2, Litematic]),
        [b'B', b'C', ..] => result.push(BCF),
        [b'P', b'K', ..] => result.push(MCWorld),
        [b'B', b'D', ..] => result.push(BDX),
        [b'I', b'B', ..] => result.push(IBImport),
        [b'c', b'o', ..] => result.push(Construction),
        [b'H', b'4', ..] => result.push(SIBI),
        [b's', b'e', ..] | [b'f', b'i', ..] | [b't', b'i', ..] => result.push(MCFunction),
        [b'#' | b'/', ..] => result.push(MCFunction),
        [b'{', ..] => result.extend([
            FuHongV4,
            FuHongV3,
            FuHongV2,
            TimeBuilderV1,
            MianYangV1,
            QingXuV1,
            CovStructure,
        ]),
        [b'[', b'[', ..] => result.push(GangBanV6),
        [b'[', b'{', ..] => result.extend([
            RunAway, FuHongV1, GangBanV5, GangBanV4, GangBanV3, GangBanV1,
        ]),
        [b'[', ..] => result.extend([
            FuHongV1, GangBanV5, GangBanV4, GangBanV3, GangBanV1, RunAway,
        ]),
        [0x78, ..] => result.extend([FuHongV5, MianYangV3, GangBanV7]),
        [0x91 | 0x92, ..] => result.extend([BDS, NexusNP]),
        _ => {}
    }
    result.push(KBDX);
    result.dedup();
    result
}

pub fn detect(name: &str, bytes: &[u8]) -> Result<StructureData> {
    let mut errors = Vec::new();
    // 二进制优先使用完整原始字节。0x0A 是 NBT Compound 标签，不能当作换行删除。
    for format in candidates(name, bytes) {
        match decode(format, bytes) {
            Ok(value) => return Ok(value),
            Err(error) => errors.push(format!("{}: {error}", format.name())),
        }
    }
    let without_bom = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let start = without_bom
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(without_bom.len());
    let text = &without_bom[start..];
    if text.len() != bytes.len()
        && matches!(
            text.first(),
            Some(b'{' | b'[' | b'#' | b'/' | b's' | b'f' | b't')
        )
    {
        for format in candidates(name, text)
            .into_iter()
            .filter(|format| text_format(*format))
        {
            match decode(format, text) {
                Ok(value) => return Ok(value),
                Err(error) => errors.push(format!("{}: {error}", format.name())),
            }
        }
    }
    Err(Error::invalid(format!(
        "无法识别结构文件；{}",
        errors.join("；")
    )))
}

fn text_format(format: FormatId) -> bool {
    use FormatId::*;
    matches!(
        format,
        MCFunction
            | RunAway
            | QingXuV1
            | TimeBuilderV1
            | CovStructure
            | MianYangV1
            | MianYangV2
            | GangBanV1
            | GangBanV2
            | GangBanV3
            | GangBanV4
            | GangBanV5
            | GangBanV6
            | FuHongV1
            | FuHongV2
            | FuHongV3
            | FuHongV4
    )
}

#[derive(Debug)]
pub struct Reader {
    data: StructureData,
    chunk_index: HashMap<(i32, i32), Vec<Pos>>,
    closed: bool,
}

impl Reader {
    pub fn open(
        path: impl AsRef<Path>,
        format: Option<FormatId>,
        selection: Option<(Pos, Pos)>,
    ) -> Result<Self> {
        let path = path.as_ref();
        if path.is_dir() {
            return Self::from_data(world::read_directory(path, selection)?);
        }
        let bytes = std::fs::read(path)?;
        let name = path.to_string_lossy();
        if format == Some(FormatId::MCWorld) || (format.is_none() && bytes.starts_with(b"PK")) {
            let selection = selection.or_else(|| world::selection_from_name(&name));
            return Self::from_data(world::decode_archive(&bytes, selection)?);
        }
        Self::from_bytes(&name, &bytes, format)
    }

    pub fn from_bytes(name: &str, bytes: &[u8], format: Option<FormatId>) -> Result<Self> {
        let data = match format {
            Some(format) => decode(format, bytes)?,
            None => detect(name, bytes)?,
        };
        Self::from_data(data)
    }

    pub fn from_data(data: StructureData) -> Result<Self> {
        let mut reader = Self {
            data,
            chunk_index: HashMap::new(),
            closed: false,
        };
        reader.index_chunks()?;
        Ok(reader)
    }

    fn index_chunks(&mut self) -> Result<()> {
        let mut index = HashMap::<_, Vec<_>>::new();
        for position in self.data.blocks.keys() {
            let x = position
                .0
                .checked_add(self.data.offset.0)
                .ok_or_else(|| Error::invalid("X 偏移溢出"))?;
            let z = position
                .2
                .checked_add(self.data.offset.2)
                .ok_or_else(|| Error::invalid("Z 偏移溢出"))?;
            position
                .1
                .checked_add(self.data.offset.1)
                .and_then(|value| value.checked_sub(64))
                .ok_or_else(|| Error::invalid("Y 偏移溢出"))?;
            index
                .entry((x.div_euclid(16), z.div_euclid(16)))
                .or_default()
                .push(*position);
        }
        self.chunk_index = index;
        Ok(())
    }

    pub fn data(&self) -> &StructureData {
        &self.data
    }

    pub fn into_data(self) -> StructureData {
        self.data
    }

    pub fn set_offset(&mut self, offset: Pos) -> Result<()> {
        self.check_open()?;
        let previous = self.data.offset;
        self.data.offset = offset;
        if let Err(error) = self.index_chunks() {
            self.data.offset = previous;
            return Err(error);
        }
        Ok(())
    }

    fn check_open(&self) -> Result<()> {
        if self.closed {
            Err(Error::invalid("读取器已关闭"))
        } else {
            Ok(())
        }
    }

    pub fn count_non_air_blocks(&self) -> Result<u64> {
        self.check_open()?;
        Ok(self.data.non_air_count())
    }

    pub fn get_chunks(
        &self,
        positions: &[(i32, i32)],
    ) -> Result<BTreeMap<(i32, i32), BTreeMap<Pos, Block>>> {
        self.check_open()?;
        let mut chunks = BTreeMap::new();
        for chunk in positions {
            let mut values = BTreeMap::new();
            if let Some(keys) = self.chunk_index.get(chunk) {
                for key in keys {
                    let position = Pos(
                        (key.0 + self.data.offset.0).rem_euclid(16),
                        key.1 + self.data.offset.1 - 64,
                        (key.2 + self.data.offset.2).rem_euclid(16),
                    );
                    values.insert(position, self.data.blocks[key].clone());
                }
            }
            chunks.insert(*chunk, values);
        }
        Ok(chunks)
    }

    pub fn get_chunks_nbt(
        &self,
        positions: &[(i32, i32)],
    ) -> Result<BTreeMap<(i32, i32), BTreeMap<Pos, Compound>>> {
        Ok(self
            .get_chunks(positions)?
            .into_iter()
            .map(|(position, chunk)| {
                (
                    position,
                    chunk
                        .into_iter()
                        .filter_map(|(position, block)| block.nbt.map(|nbt| (position, nbt)))
                        .collect(),
                )
            })
            .collect())
    }

    pub fn close(&mut self) -> Result<()> {
        self.data.blocks.clear();
        self.chunk_index.clear();
        self.closed = true;
        Ok(())
    }
}

fn is_container(state: &BlockState) -> bool {
    state.name.contains("chest")
        || state.name.contains("shulker_box")
        || matches!(
            state.name.trim_start_matches("minecraft:"),
            "barrel"
                | "hopper"
                | "dispenser"
                | "dropper"
                | "furnace"
                | "lit_furnace"
                | "blast_furnace"
                | "lit_blast_furnace"
                | "smoker"
                | "lit_smoker"
                | "crafter"
        )
}

fn checked_add(value: i32, offset: i32) -> Result<i32> {
    value
        .checked_add(offset)
        .ok_or_else(|| Error::invalid("导出坐标偏移溢出"))
}

fn shift_number(value: &mut Nbt, offset: i32) -> Result<()> {
    if offset == 0 {
        return Ok(());
    }
    let overflow = || Error::invalid("实体坐标偏移溢出");
    match value {
        Nbt::Byte(value) => {
            *value = i8::try_from(i64::from(*value) + i64::from(offset)).map_err(|_| overflow())?
        }
        Nbt::Short(value) => {
            *value = i16::try_from(i64::from(*value) + i64::from(offset)).map_err(|_| overflow())?
        }
        Nbt::Int(value) => *value = checked_add(*value, offset)?,
        Nbt::Long(value) => *value = value.checked_add(i64::from(offset)).ok_or_else(overflow)?,
        Nbt::Float(value) => {
            *value += offset as f32;
            if !value.is_finite() {
                return Err(overflow());
            }
        }
        Nbt::Double(value) => {
            *value += f64::from(offset);
            if !value.is_finite() {
                return Err(overflow());
            }
        }
        _ => return Err(Error::invalid("实体坐标不是数值")),
    }
    Ok(())
}

fn shift_positions(value: &mut Compound, offset: Pos) -> Result<()> {
    for (key, shift) in [("x", offset.0), ("y", offset.1), ("z", offset.2)] {
        if let Some(value) = value.get_mut(key) {
            shift_number(value, shift)?;
        }
    }
    for key in ["Pos", "pos", "blockPos"] {
        match value.get_mut(key) {
            Some(Nbt::List(values)) if values.len() == 3 => {
                for (value, shift) in values.iter_mut().zip([offset.0, offset.1, offset.2]) {
                    shift_number(value, shift)?;
                }
            }
            Some(Nbt::IntArray(values)) if values.len() == 3 => {
                for (value, shift) in values.iter_mut().zip([offset.0, offset.1, offset.2]) {
                    *value = checked_add(*value, shift)?;
                }
            }
            Some(Nbt::Compound(value)) => shift_positions(value, offset)?,
            _ => {}
        }
    }
    if let Some(Nbt::Compound(nbt)) = value.get_mut("nbt") {
        shift_positions(nbt, offset)?;
    }
    Ok(())
}

fn materialize_offset(data: &StructureData) -> Result<StructureData> {
    let dimension = |size: u32, offset: i32| {
        size.checked_add(offset.unsigned_abs())
            .filter(|size| *size > 0 && *size <= i32::MAX as u32)
            .ok_or_else(|| Error::invalid("导出尺寸为空或偏移后超过有符号32位范围"))
    };
    let size = Size(
        dimension(data.size.0, data.offset.0)?,
        dimension(data.size.1, data.offset.1)?,
        dimension(data.size.2, data.offset.2)?,
    );
    u64::from(size.0)
        .checked_mul(u64::from(size.1))
        .and_then(|value| value.checked_mul(u64::from(size.2)))
        .ok_or_else(|| Error::invalid("导出结构体积溢出"))?;
    let origin = Pos(
        checked_add(data.origin.0, data.offset.0.min(0))?,
        checked_add(data.origin.1, data.offset.1.min(0))?,
        checked_add(data.origin.2, data.offset.2.min(0))?,
    );
    for (origin, length) in [(origin.0, size.0), (origin.1, size.1), (origin.2, size.2)] {
        i32::try_from(i64::from(origin) + i64::from(length) - 1)
            .map_err(|_| Error::invalid("导出结构边界超出坐标范围"))?;
    }
    let shift = Pos(
        data.offset.0.max(0),
        data.offset.1.max(0),
        data.offset.2.max(0),
    );
    let mut result = StructureData {
        format: data.format,
        size,
        origin,
        offset: Pos::default(),
        blocks: BTreeMap::new(),
        entities: data.entities.clone(),
        metadata: data.metadata.clone(),
    };
    for (pos, block) in &data.blocks {
        if pos.0 < 0
            || pos.1 < 0
            || pos.2 < 0
            || pos.0 as u32 >= data.size.0
            || pos.1 as u32 >= data.size.1
            || pos.2 as u32 >= data.size.2
        {
            return Err(Error::invalid(format!(
                "方块 {pos:?} 超出声明尺寸 {:?}",
                data.size
            )));
        }
        let mut block = block.clone();
        if shift != Pos::default()
            && let Some(nbt) = &mut block.nbt
        {
            shift_positions(nbt, shift)?;
        }
        result.blocks.insert(
            Pos(
                checked_add(pos.0, shift.0)?,
                checked_add(pos.1, shift.1)?,
                checked_add(pos.2, shift.2)?,
            ),
            block,
        );
    }
    if shift != Pos::default() {
        for entity in &mut result.entities {
            shift_positions(entity, shift)?;
        }
    }
    result.size = size;
    result.origin = origin;
    result.offset = Pos::default();
    Ok(result)
}

fn permits_nbt(capabilities: Capabilities, state: &crate::BlockState) -> bool {
    match capabilities.block_nbt {
        "all" => true,
        "command" => state.name.ends_with("command_block"),
        "command-container" => state.name.ends_with("command_block") || is_container(state),
        "command-container-sign" => {
            state.name.ends_with("command_block")
                || is_container(state)
                || state.name.contains("sign")
        }
        _ => false,
    }
}

fn state_is_preserved(source: &BlockState, output: &BlockState) -> bool {
    source.name == output.name
        && source.properties.iter().all(|(key, value)| {
            output.properties.get(key).is_some_and(|after| {
                value == after
                    || value
                        .as_i64()
                        .zip(after.as_i64())
                        .is_some_and(|(a, b)| a == b)
            })
        })
}

fn loses_state(format: FormatId, state: &BlockState) -> bool {
    use FormatId::*;
    let output = match format {
        Schematic => {
            let (id, aux) = blocks::to_schematic(state);
            blocks::from_schematic(id, aux)
        }
        SchemV1 | SchemV2 | Litematic | AxiomBP => blocks::java(&blocks::to_java(state)),
        KBDX | NexusNP | RunAway | TimeBuilderV1 | CovStructure | FuHongV1 | FuHongV3
        | FuHongV4 | FuHongV5 | MianYangV1 | MianYangV2 | MianYangV3 | MianYangV4 | GangBanV1
        | GangBanV2 | GangBanV3 | GangBanV4 | GangBanV5 | GangBanV6 | GangBanV7 => {
            let (name, aux) = blocks::to_legacy(state);
            blocks::legacy(&name, aux)
        }
        _ => return false,
    };
    !state_is_preserved(state, &output)
}

fn preserves_nbt(source: &Nbt, output: &Nbt) -> bool {
    match (source, output) {
        (Nbt::Compound(source), Nbt::Compound(output)) => source.iter().all(|(key, value)| {
            output
                .get(key)
                .is_some_and(|after| preserves_nbt(value, after))
        }),
        (Nbt::List(source), Nbt::List(output)) => {
            source.len() == output.len()
                && source
                    .iter()
                    .zip(output)
                    .all(|(before, after)| preserves_nbt(before, after))
        }
        _ => source == output,
    }
}

fn has_raw_string(value: &Nbt) -> bool {
    match value {
        Nbt::RawString(_) => true,
        Nbt::Compound(values) => values.values().any(has_raw_string),
        Nbt::List(values) => values.iter().any(has_raw_string),
        _ => false,
    }
}

fn native_nbt_format(format: FormatId) -> bool {
    use FormatId::*;
    matches!(
        format,
        Schematic
            | SchemV1
            | SchemV2
            | Litematic
            | MCStructure
            | MCWorld
            | BDX
            | Construction
            | AxiomBP
    )
}

fn kbdx_field_loss(block: &Block, nbt: &Compound) -> bool {
    if block.state.name.ends_with("command_block") {
        return false;
    }
    if nbt.iter().any(|(key, value)| {
        !matches!(
            key.as_str(),
            "id" | "Items" | "CustomName" | "Lock" | "x" | "y" | "z" | "Pos"
        ) || (matches!(key.as_str(), "CustomName" | "Lock") && value.as_str() == Some(""))
    }) {
        return true;
    }
    nbt.get("Items")
        .and_then(Nbt::as_list)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.as_compound().is_none_or(|item| {
                    item.keys()
                        .any(|key| !matches!(key.as_str(), "Name" | "Damage" | "Count" | "Slot"))
                })
            })
        })
}

fn loses_nbt_fields(format: FormatId, block: &Block) -> Result<bool> {
    let Some(source) = &block.nbt else {
        return Ok(false);
    };
    if format == FormatId::KBDX && kbdx_field_loss(block, source) {
        return Ok(true);
    }
    let mut probe = StructureData::new(format);
    probe.size = Size(1, 1, 1);
    let mut block = block.clone();
    block.extra = None;
    probe.blocks.insert(Pos::default(), block);
    let bytes = encode_codec(format, &probe)?;
    let restored = decode(format, &bytes)?;
    let Some(output) = restored
        .blocks
        .values()
        .find_map(|block| block.nbt.as_ref())
    else {
        return Ok(!source.is_empty());
    };
    Ok(source
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "x" | "y" | "z" | "Pos"))
        .any(|(key, before)| {
            !output
                .get(key)
                .is_some_and(|after| preserves_nbt(before, after))
        }))
}

fn report_bounds(
    format: FormatId,
    data: &StructureData,
    world_archive: bool,
    report: &mut LossReport,
) {
    let capabilities = capabilities(format);
    if format == FormatId::MCWorld {
        if world_archive && data.origin != Pos(0, -64, 0) {
            report.bounds_or_origin = true;
            report
                .details
                .push("MCWorld 归档写入坐标固定从 (0,-64,0) 开始，来源 origin 将改变".into());
        }
    } else if !capabilities.origin && data.origin != Pos::default() {
        report.bounds_or_origin = true;
        report
            .details
            .push(format!("{} 不保存来源 origin", format.name()));
    }
    if format == FormatId::AxiomBP
        && [data.origin.0, data.origin.1, data.origin.2]
            .iter()
            .any(|v| v.rem_euclid(16) != 0)
        || format == FormatId::AxiomBP
            && [data.size.0, data.size.1, data.size.2]
                .iter()
                .any(|v| !v.is_multiple_of(16))
    {
        report.bounds_or_origin = true;
        report
            .details
            .push("AxiomBP 将 origin 与空气边界扩展到16方块网格".into());
    }
    if !capabilities.declared_size {
        let positions: Vec<_> = data
            .blocks
            .iter()
            .filter(|(_, block)| {
                !matches!(format, FormatId::BDS | FormatId::NexusNP | FormatId::BCF)
                    || !block.state.is_air()
            })
            .map(|(position, _)| position)
            .collect();
        let minimum = Pos(
            positions.iter().map(|p| p.0).min().unwrap_or(0),
            positions.iter().map(|p| p.1).min().unwrap_or(0),
            positions.iter().map(|p| p.2).min().unwrap_or(0),
        );
        let maximum = Pos(
            positions.iter().map(|p| p.0).max().unwrap_or(-1),
            positions.iter().map(|p| p.1).max().unwrap_or(-1),
            positions.iter().map(|p| p.2).max().unwrap_or(-1),
        );
        if minimum != Pos::default()
            || i64::from(maximum.0) + 1 != i64::from(data.size.0)
            || i64::from(maximum.1) + 1 != i64::from(data.size.1)
            || i64::from(maximum.2) + 1 != i64::from(data.size.2)
        {
            report.bounds_or_origin = true;
            report.details.push(format!(
                "{} 从方块坐标重建边界，不能保留全部空气边缘",
                format.name()
            ));
        }
    }
}

fn prepare_export_with_destination(
    format: FormatId,
    data: &StructureData,
    policy: LossPolicy,
    world_archive: bool,
) -> Result<(StructureData, LossReport)> {
    let capabilities = capabilities(format);
    if !capabilities.write {
        return Err(Error::Unsupported(format.name().into()));
    }
    let mut report = LossReport::default();
    let mut sanitized = materialize_offset(data)?;
    report.metadata = match format {
        FormatId::Schematic
        | FormatId::SchemV1
        | FormatId::SchemV2
        | FormatId::MCStructure
        | FormatId::Litematic
        | FormatId::Construction
        | FormatId::MCWorld => false,
        FormatId::AxiomBP => sanitized.metadata.keys().any(|key| {
            !matches!(
                key.as_str(),
                "DataVersion" | "AxiomHeader" | "AxiomThumbnail"
            )
        }),
        _ => !sanitized.metadata.is_empty(),
    };
    if report.metadata {
        report.details.push("目标格式无法保存全部来源元数据".into());
    }
    let mut state_cache = HashMap::new();
    let mut nbt_cache = HashMap::new();
    for block in sanitized.blocks.values_mut() {
        if !capabilities.extra_layers && block.extra.as_ref().is_some_and(|state| !state.is_air()) {
            report.extra_layers += 1;
            block.extra = None;
        }
        if block.nbt.is_some() {
            if !permits_nbt(capabilities, &block.state) {
                report.block_nbt += 1;
                block.nbt = None;
            } else if !native_nbt_format(format)
                && block
                    .nbt
                    .as_ref()
                    .is_some_and(|values| values.values().any(has_raw_string))
            {
                report.block_nbt += 1;
            } else if capabilities.block_nbt != "all" || format == FormatId::FuHongV2 {
                let key = (block.state.string(), serde_json::to_string(&block.nbt)?);
                let loss = if let Some(loss) = nbt_cache.get(&key) {
                    *loss
                } else {
                    let loss = loses_nbt_fields(format, block)?;
                    nbt_cache.insert(key, loss);
                    loss
                };
                if loss {
                    report.block_nbt += 1;
                }
            }
        }
        let key = block.state.string();
        let loss = *state_cache
            .entry(key)
            .or_insert_with(|| loses_state(format, &block.state));
        if loss {
            report.block_states += 1;
        }
        if let Some(extra) = &block.extra {
            let loss = *state_cache
                .entry(extra.string())
                .or_insert_with(|| loses_state(format, extra));
            if loss {
                report.block_states += 1;
            }
        }
    }
    if !capabilities.entities {
        report.entities = sanitized.entities.len() as u64;
        sanitized.entities.clear();
    }
    report_bounds(format, &sanitized, world_archive, &mut report);
    if report.block_nbt != 0 {
        report.details.push(format!(
            "{} 个方块实体无法保留全部原始NBT字段或值",
            report.block_nbt
        ));
    }
    if report.block_states != 0 {
        report.details.push(format!(
            "{} 个方块状态将被目标方块表降级",
            report.block_states
        ));
    }
    if policy == LossPolicy::Strict && !report.is_empty() {
        return Err(Error::Unsupported(format!(
            "{} 无法无损表示数据: {}",
            format.name(),
            serde_json::to_string(&report)?
        )));
    }
    Ok((sanitized, report))
}

/// 应用偏移、检查目标能力并生成损失报告；返回值可直接交给相应编码器。
pub fn prepare_export(
    format: FormatId,
    data: &StructureData,
    policy: LossPolicy,
) -> Result<(StructureData, LossReport)> {
    prepare_export_with_destination(format, data, policy, true)
}

pub fn encode(format: FormatId, data: &StructureData, policy: LossPolicy) -> Result<Encoded> {
    let (prepared, report) = prepare_export(format, data, policy)?;
    let bytes = match format {
        FormatId::MCWorld => world::encode_archive(&prepared)?,
        _ => encode_codec(format, &prepared)?,
    };
    Ok(Encoded { bytes, report })
}

/// 将世界写入真实目录，使用结构 origin 定位，复用归档导出的其余能力与损失检查。
pub fn encode_world_directory(
    path: impl AsRef<Path>,
    data: &StructureData,
    policy: LossPolicy,
) -> Result<LossReport> {
    let (prepared, report) =
        prepare_export_with_destination(FormatId::MCWorld, data, policy, false)?;
    world::write_directory(path.as_ref(), &prepared, prepared.origin, None)?;
    Ok(report)
}

fn encode_codec(format: FormatId, data: &StructureData) -> Result<Vec<u8>> {
    use FormatId::*;
    match format {
        Schematic | SchemV1 | SchemV2 | Litematic | MCStructure | AxiomBP | Construction => {
            formats::java::encode(format, data)
        }
        BDX | MCFunction | KBDX | IBImport | BDS | BCF | TIBI | NexusNP => {
            formats::binary::encode(format, data)
        }
        SIBI | MCWorld => Err(Error::Unsupported(format.name().into())),
        _ => formats::json::encode(format, data),
    }
}
