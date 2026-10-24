use super::*;
use crate::bdump::{BlockReference, Command, CommandReader};
use std::collections::BTreeMap;
use std::sync::OnceLock;

fn decompress(bytes: &[u8]) -> Result<Vec<u8>> {
    if !bytes.starts_with(b"BD@") {
        return Err(Error::invalid("无效 BDX 压缩头"));
    }
    let mut decompressed = Vec::new();
    brotli::Decompressor::new(&bytes[3..], 4096).read_to_end(&mut decompressed)?;
    Ok(decompressed)
}

fn metadata(bytes: &[u8]) -> Result<(String, usize)> {
    let mut reader = Cursor::new(bytes);
    if reader.take(4)? != b"BDX\0" {
        return Err(Error::invalid("无效 BDX 数据头"));
    }
    let author = reader.zstring()?;
    Ok((author, reader.pos))
}

fn advance(position: &mut [i32; 3], axis: usize, delta: i32) -> Result<()> {
    position[axis] = position[axis]
        .checked_add(delta)
        .ok_or_else(|| Error::invalid("BDX 笔刷坐标溢出"))?;
    Ok(())
}

fn prepare_nbt(nbt: &mut Compound, state: Option<&BlockState>, preserve_command_mode: bool) {
    // 专用命令方块指令的模式只用于解析方块类型；opcode41中的同名字段是原始NBT。
    if !preserve_command_mode && nbt.get("id").and_then(Nbt::as_str) == Some("CommandBlock") {
        nbt.remove("LPCommandMode");
    }
    if nbt.contains_key("Items")
        && !nbt.contains_key("id")
        && let Some(id) = state.and_then(container_id)
    {
        nbt.insert("id".into(), Nbt::String(id.into()));
    }
}

fn runtime_state(pool: u8, index: u32) -> Result<BlockState> {
    if pool != 117 {
        return Err(Error::invalid(format!("未知 BDX runtime pool: {pool}")));
    }
    static POOL: OnceLock<Vec<(String, u16)>> = OnceLock::new();
    let entries = POOL.get_or_init(|| {
        serde_json::from_str(include_str!("runtime_ids_117.json")).expect("内置117调色板正确")
    });
    let (name, aux) = entries
        .get(index as usize)
        .ok_or_else(|| Error::invalid("BDX 117 runtime 索引越界"))?;
    Ok(crate::blocks::legacy(name, *aux))
}

fn resolve(
    reference: BlockReference,
    constants: &[String],
    pool: u8,
    nbt: Option<&Compound>,
) -> Result<BlockState> {
    let constant = |index: u16| {
        constants
            .get(usize::from(index))
            .map(String::as_str)
            .ok_or_else(|| Error::invalid("BDX 常量索引越界"))
    };
    match reference {
        BlockReference::Legacy { name, data } => Ok(crate::blocks::legacy(constant(name)?, data)),
        BlockReference::States { name, states } => {
            Ok(super::text::state(constant(name)?, constant(states)?))
        }
        BlockReference::InlineStates { name, states } => {
            Ok(super::text::state(constant(name)?, &states))
        }
        BlockReference::Runtime(index) => runtime_state(pool, index),
        BlockReference::CommandBlock { data } => {
            let mode = nbt
                .and_then(|n| n.get("LPCommandMode"))
                .and_then(Nbt::as_i64)
                .unwrap_or(0);
            let name = match mode {
                0 => "command_block",
                1 => "repeating_command_block",
                2 => "chain_command_block",
                _ => return Err(Error::invalid("BDX 命令方块模式错误")),
            };
            Ok(crate::blocks::legacy(name, data))
        }
    }
}

fn container_id(state: &BlockState) -> Option<&'static str> {
    match state.name.trim_start_matches("minecraft:") {
        "chest" | "trapped_chest" => Some("Chest"),
        "hopper" => Some("Hopper"),
        "barrel" => Some("Barrel"),
        "dispenser" => Some("Dispenser"),
        "dropper" => Some("Dropper"),
        "furnace" | "lit_furnace" => Some("Furnace"),
        "blast_furnace" | "lit_blast_furnace" => Some("BlastFurnace"),
        "smoker" | "lit_smoker" => Some("Smoker"),
        "crafter" => Some("Crafter"),
        name if name.contains("shulker_box") => Some("ShulkerBox"),
        _ => None,
    }
}

pub(super) fn decode(bytes: &[u8]) -> Result<StructureData> {
    let decompressed = decompress(bytes)?;
    let (author, command_offset) = metadata(&decompressed)?;
    let mut result = StructureData::new(FormatId::BDX);
    result.metadata.insert("author".into(), Nbt::String(author));
    let signature = crate::bdump::verify_stream(&decompressed)?;
    result
        .metadata
        .insert("signed".into(), Nbt::Byte(signature.signed as i8));
    result.metadata.insert(
        "signature_corrupted".into(),
        Nbt::Byte(signature.corrupted as i8),
    );
    if let Some(username) = signature.username {
        result
            .metadata
            .insert("signer".into(), Nbt::String(username));
    }
    let mut reader = CommandReader::new(&decompressed[command_offset..]);
    let mut constants = Vec::new();
    let mut position = [0i32; 3];
    let mut minimum = [0i32; 3];
    let mut maximum = [0i32; 3];
    let mut pool = 117;
    let mut terminated = false;
    while let Some(command) = reader.read()? {
        let pos = Pos(position[0], position[1], position[2]);
        match command {
            Command::Constant(value) => {
                if constants.len() > usize::from(u16::MAX) {
                    return Err(Error::invalid("BDX 常量超过65536项"));
                }
                constants.push(value);
            }
            Command::Move { axis, delta } => {
                advance(&mut position, axis, delta)?;
                minimum[axis] = minimum[axis].min(position[axis]);
                maximum[axis] = maximum[axis].max(position[axis]);
            }
            Command::RuntimePool(id) => pool = id,
            Command::Place {
                block: reference,
                mut nbt,
            } => {
                let raw_nbt = matches!(&reference, BlockReference::States { .. });
                let state = resolve(reference, &constants, pool, nbt.as_ref())?;
                if let Some(nbt) = &mut nbt {
                    prepare_nbt(nbt, Some(&state), raw_nbt);
                }
                // Go 将方块和方块实体分别写入，普通放置不会清除先前的 NBT。
                let nbt =
                    nbt.or_else(|| result.blocks.get(&pos).and_then(|block| block.nbt.clone()));
                result.blocks.insert(
                    pos,
                    Block {
                        state,
                        extra: None,
                        nbt,
                    },
                );
            }
            Command::SetCommandBlock(mut nbt) => {
                prepare_nbt(&mut nbt, None, false);
                result
                    .blocks
                    .entry(pos)
                    .or_insert_with(|| Block::new(BlockState::air()))
                    .nbt = Some(nbt);
            }
            Command::Debug(bytes) => {
                result.metadata.insert(
                    "debug".into(),
                    Nbt::ByteArray(bytes.into_iter().map(|b| b as i8).collect()),
                );
            }
            Command::NoOperation => {}
            Command::Terminate => {
                terminated = true;
                break;
            }
        }
    }
    if !terminated {
        return Err(Error::invalid("BDX 缺少结束指令"));
    }
    result
        .metadata
        .insert("runtime_pool".into(), Nbt::Int(i32::from(pool)));
    // BDX 的范围包含指令走过的笔刷位置，即使这些位置没有放置方块。
    // 原 Go 读取器保留这种空气边界，不能只从非空方块重新计算尺寸。
    let mut normalized = BTreeMap::new();
    for (pos, block) in result.blocks {
        let relative = |coordinate: i32, origin: i32| {
            coordinate
                .checked_sub(origin)
                .ok_or_else(|| Error::invalid("BDX 结构坐标溢出"))
        };
        normalized.insert(
            Pos(
                relative(pos.0, minimum[0])?,
                relative(pos.1, minimum[1])?,
                relative(pos.2, minimum[2])?,
            ),
            block,
        );
    }
    let dimension = |axis: usize| {
        u32::try_from(i64::from(maximum[axis]) - i64::from(minimum[axis]) + 1)
            .map_err(|_| Error::invalid("BDX 尺寸溢出"))
    };
    result.blocks = normalized;
    result.origin = Pos(minimum[0], minimum[1], minimum[2]);
    result.size = crate::Size(dimension(0)?, dimension(1)?, dimension(2)?);
    Ok(result)
}

fn constant_id(constants: &mut BTreeMap<String, u16>, value: String) -> Result<u16> {
    if let Some(index) = constants.get(&value) {
        return Ok(*index);
    }
    let index =
        u16::try_from(constants.len()).map_err(|_| Error::invalid("BDX 常量超过65536项"))?;
    if value.contains('\0') {
        return Err(Error::invalid("BDX 字符串不能包含NUL"));
    }
    constants.insert(value, index);
    Ok(index)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FastBDXResult {
    pub min: Pos,
    pub max: Pos,
    pub blocks: u64,
    pub commands: u64,
    pub empty: bool,
}

impl FastBDXResult {
    pub fn size(&self) -> Result<crate::Size> {
        let dimension = |minimum: i32, maximum: i32| {
            u32::try_from(i64::from(maximum) - i64::from(minimum) + 1)
                .map_err(|_| Error::invalid("BDX 轨迹尺寸溢出"))
        };
        Ok(crate::Size(
            dimension(self.min.0, self.max.0)?,
            dimension(self.min.1, self.max.1)?,
            dimension(self.min.2, self.max.2)?,
        ))
    }
}

fn scan_commands(bytes: &[u8], control: Option<&crate::world::Control>) -> Result<FastBDXResult> {
    let mut result = FastBDXResult {
        min: Pos::default(),
        max: Pos::default(),
        blocks: 0,
        commands: 0,
        empty: true,
    };
    let mut position = [0i32; 3];
    let mut reader = CommandReader::new(bytes);
    while let Some(command) = reader.read()? {
        result.commands += 1;
        if result.commands.is_multiple_of(1024)
            && let Some(control) = control
        {
            control.check()?;
        }
        match command {
            Command::Move { axis, delta } => {
                advance(&mut position, axis, delta)?;
                result.min = Pos(
                    result.min.0.min(position[0]),
                    result.min.1.min(position[1]),
                    result.min.2.min(position[2]),
                );
                result.max = Pos(
                    result.max.0.max(position[0]),
                    result.max.1.max(position[1]),
                    result.max.2.max(position[2]),
                );
            }
            Command::Place { .. } | Command::SetCommandBlock(_) => {
                result.blocks += 1;
                result.empty = false;
            }
            Command::Terminate => return Ok(result),
            _ => {}
        }
    }
    Err(Error::invalid("BDX 缺少结束指令"))
}

struct DirectWriter<'a> {
    world: &'a crate::world::cache::CachedWorld,
    start: Pos,
    minimum: Pos,
    position: [i32; 3],
    constants: Vec<String>,
    pool: u8,
    resolved: BTreeMap<(u8, BlockReference, i64), BlockState>,
}

impl DirectWriter<'_> {
    fn world_position(&self) -> Result<Pos> {
        let axis = |coordinate: i32, minimum: i32, start: i32| {
            let value = i64::from(start) + i64::from(coordinate) - i64::from(minimum);
            i32::try_from(value).map_err(|_| Error::invalid("BDX 导入世界坐标溢出"))
        };
        let pos = Pos(
            axis(self.position[0], self.minimum.0, self.start.0)?,
            axis(self.position[1], self.minimum.1, self.start.1)?,
            axis(self.position[2], self.minimum.2, self.start.2)?,
        );
        if !(-64..=319).contains(&pos.1) {
            return Err(Error::invalid(format!(
                "BDX 方块 Y 坐标 {} 超出主世界范围 [-64,319]",
                pos.1
            )));
        }
        Ok(pos)
    }

    fn placement(&mut self, reference: BlockReference, mut nbt: Option<Compound>) -> Result<()> {
        let pos = self.world_position()?;
        let raw_nbt = matches!(&reference, BlockReference::States { .. });
        let mode = nbt
            .as_ref()
            .and_then(|value| value.get("LPCommandMode"))
            .and_then(Nbt::as_i64)
            .unwrap_or(0);
        let key = (self.pool, reference, mode);
        if !self.resolved.contains_key(&key) {
            let state = resolve(key.1.clone(), &self.constants, self.pool, nbt.as_ref())?;
            if self.resolved.len() >= 65536 {
                self.resolved.clear();
            }
            self.resolved.insert(key.clone(), state);
        }
        let state = self
            .resolved
            .get(&key)
            .ok_or_else(|| Error::invalid("BDX 方块映射缓存缺失"))?;
        // Go 快路径将映射失败的 RuntimeID=0 视为不修改方块，NBT仍可单独更新。
        if state.name != "minecraft:unknown" {
            self.world.set_resolved_block(pos, 0, state.clone())?;
        }
        if let Some(nbt) = &mut nbt {
            prepare_nbt(nbt, Some(state), raw_nbt);
            self.world.set_block_nbt(pos, std::mem::take(nbt))?;
        }
        Ok(())
    }

    fn apply(&mut self, command: Command) -> Result<bool> {
        match command {
            Command::Constant(value) => {
                if self.constants.len() > usize::from(u16::MAX) {
                    return Err(Error::invalid("BDX 常量超过65536项"));
                }
                self.constants.push(value);
            }
            Command::Move { axis, delta } => advance(&mut self.position, axis, delta)?,
            Command::RuntimePool(pool) => self.pool = pool,
            Command::Place { block, nbt } => self.placement(block, nbt)?,
            Command::SetCommandBlock(mut nbt) => {
                let pos = self.world_position()?;
                prepare_nbt(&mut nbt, None, false);
                self.world.set_block_nbt(pos, nbt)?;
            }
            Command::Terminate => return Ok(false),
            Command::Debug(_) | Command::NoOperation => {}
        }
        Ok(true)
    }
}

/// 解压一次、扫描两遍，将 BDX 指令直接应用到有界世界缓存。
/// start 是结构最小轨迹点对应的真实世界坐标；不会构造逐方块 BTreeMap。
pub fn import_bdx_to_world(
    bytes: &[u8],
    cached: &crate::world::cache::CachedWorld,
    start: Pos,
    control: Option<&crate::world::Control>,
) -> Result<FastBDXResult> {
    if let Some(control) = control {
        control.check()?;
    }
    let decompressed = decompress(bytes)?;
    let (_, offset) = metadata(&decompressed)?;
    let commands = &decompressed[offset..];
    let result = scan_commands(commands, control)?;
    if let Some(control) = control {
        control.check()?;
        control.report(0, result.commands);
    }
    let mut writer = DirectWriter {
        world: cached,
        start,
        minimum: result.min,
        position: [0; 3],
        constants: Vec::new(),
        pool: 117,
        resolved: BTreeMap::new(),
    };
    let mut reader = CommandReader::new(commands);
    let mut completed = 0u64;
    while let Some(command) = reader.read()? {
        if completed.is_multiple_of(1024)
            && let Some(control) = control
        {
            control.check()?;
        }
        completed += 1;
        if !writer.apply(command)? {
            break;
        }
        if completed.is_multiple_of(4096)
            && let Some(control) = control
        {
            control.report(completed, result.commands);
        }
    }
    if let Some(control) = control {
        control.check()?;
    }
    cached.flush()?;
    if let Some(control) = control {
        control.report(result.commands, result.commands);
    }
    Ok(result)
}

pub(super) fn encode(data: &StructureData) -> Result<Vec<u8>> {
    let author = data
        .metadata
        .get("author")
        .and_then(Nbt::as_str)
        .unwrap_or("");
    if author.contains('\0') {
        return Err(Error::invalid("BDX 作者包含NUL"));
    }
    let mut body = b"BDX\0".to_vec();
    body.extend_from_slice(author.as_bytes());
    body.push(0);
    let mut constants = BTreeMap::new();
    let mut placements = Vec::new();
    for (pos, block) in &data.blocks {
        let name = constant_id(&mut constants, block.state.name.clone())?;
        let states = constant_id(&mut constants, super::text::bedrock_states(&block.state))?;
        // 原 Go BDump 导出器使用归零后的结构坐标。
        placements.push((*pos, name, states, block.nbt.as_ref()));
    }
    let mut ordered: Vec<_> = constants.into_iter().collect();
    ordered.sort_by_key(|(_, index)| *index);
    for (text, _) in ordered {
        body.push(1);
        body.extend_from_slice(text.as_bytes());
        body.push(0);
    }
    let mut brush = [0i32; 3];
    for (pos, name, states, nbt) in placements {
        for (axis, target) in [pos.0, pos.1, pos.2].into_iter().enumerate() {
            let delta = target
                .checked_sub(brush[axis])
                .ok_or_else(|| Error::invalid("BDX 坐标差值溢出"))?;
            if delta != 0 {
                body.push([21, 23, 25][axis]);
                body.extend_from_slice(&delta.to_be_bytes());
                brush[axis] = target;
            }
        }
        body.push(if nbt.is_some() { 41 } else { 5 });
        body.extend_from_slice(&name.to_be_bytes());
        body.extend_from_slice(&states.to_be_bytes());
        if let Some(nbt) = nbt {
            body.extend_from_slice(&states.to_be_bytes());
            body.extend(crate::nbt::encode(
                "",
                &Nbt::Compound(nbt.clone()),
                crate::nbt::Encoding::LittleEndian,
            )?);
        }
    }
    // 在末尾移动笔刷至声明尺寸的边界，保留没有方块的空气边缘。
    for (axis, size) in [data.size.0, data.size.1, data.size.2]
        .into_iter()
        .enumerate()
    {
        if size == 0 {
            continue;
        }
        let target = i32::try_from(size - 1).map_err(|_| Error::invalid("BDX 尺寸超出坐标范围"))?;
        let delta = target
            .checked_sub(brush[axis])
            .ok_or_else(|| Error::invalid("BDX 坐标差值溢出"))?;
        if delta != 0 {
            body.push([21, 23, 25][axis]);
            body.extend_from_slice(&delta.to_be_bytes());
            brush[axis] = target;
        }
    }
    body.extend_from_slice(b"XE");
    let mut output = b"BD@".to_vec();
    {
        let mut writer = brotli::CompressorWriter::new(&mut output, 4096, 6, 22);
        writer.write_all(&body)?;
    }
    Ok(output)
}
