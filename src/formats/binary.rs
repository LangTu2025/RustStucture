use crate::nbt::{Compound, Nbt};
use crate::{Block, BlockState, Error, FormatId, Pos, Result, StructureData};
use std::io::{Read, Write};

#[path = "binary/bcf.rs"]
mod bcf;
#[path = "binary/bdx.rs"]
mod bdx;
#[path = "binary/packed.rs"]
mod packed;
#[path = "binary/text.rs"]
mod text;
#[path = "binary/tibi.rs"]
mod tibi;

pub use bdx::{FastBDXResult, import_bdx_to_world};

pub fn decode(format: FormatId, data: &[u8]) -> Result<StructureData> {
    match format {
        FormatId::BDX => bdx::decode(data),
        FormatId::MCFunction => text::decode(data),
        FormatId::KBDX => packed::decode_kbdx(data),
        FormatId::IBImport => text::decode_ibi(data),
        FormatId::BDS | FormatId::NexusNP => packed::decode_msgpack(format, data),
        FormatId::BCF => bcf::decode(data),
        FormatId::TIBI => tibi::decode(data),
        _ => Err(Error::Unsupported(format.name().into())),
    }
}

pub fn encode(format: FormatId, data: &StructureData) -> Result<Vec<u8>> {
    if data.blocks.is_empty() && format != FormatId::BDX {
        return Err(Error::invalid(format!(
            "{} 不能表示空方块列表",
            format.name()
        )));
    }
    if data
        .blocks
        .values()
        .any(|block| block.extra.as_ref().is_some_and(|state| !state.is_air()))
    {
        return Err(Error::Unsupported(format!(
            "{} 不支持第二层方块",
            format.name()
        )));
    }
    match format {
        FormatId::BDX => bdx::encode(data),
        FormatId::MCFunction => Ok(text::encode(data).into_bytes()),
        FormatId::KBDX => packed::encode_kbdx(data),
        FormatId::IBImport => text::encode_ibi(data),
        FormatId::BDS | FormatId::NexusNP => packed::encode_msgpack(format, data),
        FormatId::BCF => bcf::encode(data),
        FormatId::TIBI => tibi::encode(data),
        _ => Err(Error::Unsupported(format.name().into())),
    }
}

fn world_pos(data: &StructureData, pos: Pos) -> Result<Pos> {
    let sum = |a: i32, b: i32| a.checked_add(b).ok_or_else(|| Error::invalid("坐标溢出"));
    Ok(Pos(
        sum(pos.0, data.origin.0)?,
        sum(pos.1, data.origin.1)?,
        sum(pos.2, data.origin.2)?,
    ))
}

fn fill(data: &mut StructureData, start: Pos, end: Pos, block: Block) -> Result<()> {
    let volume = (i64::from(start.0) - i64::from(end.0))
        .unsigned_abs()
        .saturating_add(1)
        .saturating_mul(
            (i64::from(start.1) - i64::from(end.1))
                .unsigned_abs()
                .saturating_add(1),
        )
        .saturating_mul(
            (i64::from(start.2) - i64::from(end.2))
                .unsigned_abs()
                .saturating_add(1),
        );
    if volume > 100_000_000 {
        return Err(Error::invalid("区域方块数量过大"));
    }
    for x in start.0.min(end.0)..=start.0.max(end.0) {
        for y in start.1.min(end.1)..=start.1.max(end.1) {
            for z in start.2.min(end.2)..=start.2.max(end.2) {
                data.blocks.insert(Pos(x, y, z), block.clone());
            }
        }
    }
    Ok(())
}

pub(super) struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(count)
            .ok_or_else(|| Error::invalid("长度溢出"))?;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| Error::invalid("二进制数据截断"))?;
        self.pos = end;
        Ok(bytes)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn string(&mut self, count: usize) -> Result<String> {
        String::from_utf8(self.take(count)?.to_vec()).map_err(|e| Error::invalid(e.to_string()))
    }
    fn zstring(&mut self) -> Result<String> {
        let count = self.data[self.pos..]
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| Error::invalid("字符串缺少终止符"))?;
        let text = self.string(count)?;
        self.take(1)?;
        Ok(text)
    }
    fn varint(&mut self) -> Result<u64> {
        let mut value = 0u64;
        for shift in (0..70).step_by(7) {
            let byte = self.u8()?;
            if shift == 63 && byte > 1 {
                return Err(Error::invalid("变长整数溢出"));
            }
            value |= u64::from(byte & 127) << shift;
            if byte & 128 == 0 {
                return Ok(value);
            }
        }
        Err(Error::invalid("无效变长整数"))
    }
    fn varstring(&mut self) -> Result<String> {
        let count = usize::try_from(self.varint()?).map_err(|_| Error::invalid("字符串过长"))?;
        self.string(count)
    }
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 128 {
        out.push(value as u8 | 128);
        value >>= 7;
    }
    out.push(value as u8);
}

fn put_varstring(out: &mut Vec<u8>, value: &str) {
    put_varint(out, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
}
