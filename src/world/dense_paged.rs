//! 有界索引文件与 16³ 分页；文件句柄释放时自动删除临时文件。
use crate::{Error, Pos, Result, Size};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    sync::Mutex,
};

const TILE_VALUES: usize = 4096;
const TILE_BYTES: u64 = (TILE_VALUES * 4) as u64;
const IO_CHUNK_VALUES: usize = 4096;

#[derive(Clone, Copy)]
pub(crate) enum IndexLayout {
    Yzx,
    Xyz,
}

pub(crate) struct RawIndices {
    file: File,
    len: usize,
    layout: IndexLayout,
}

pub struct PagedIndices {
    file: Mutex<File>,
    size: Size,
    bytes: u64,
}

impl RawIndices {
    pub(crate) fn new(len: usize, layout: IndexLayout) -> Result<Self> {
        let bytes = (len as u64)
            .checked_mul(4)
            .ok_or_else(|| Error::invalid("临时索引长度溢出"))?;
        let file = tempfile::tempfile()?;
        file.set_len(bytes)?;
        Ok(Self { file, len, layout })
    }

    pub(crate) fn read_values(&mut self, offset: usize, output: &mut [u32]) -> Result<()> {
        self.check_range(offset, output.len())?;
        self.file.seek(SeekFrom::Start(offset as u64 * 4))?;
        let mut bytes = [0u8; IO_CHUNK_VALUES * 4];
        for values in output.chunks_mut(IO_CHUNK_VALUES) {
            let length = values.len() * 4;
            self.file.read_exact(&mut bytes[..length])?;
            for (value, bytes) in values.iter_mut().zip(bytes[..length].as_chunks::<4>().0) {
                *value = u32::from_le_bytes(*bytes);
            }
        }
        Ok(())
    }

    pub(crate) fn write_values(&mut self, offset: usize, values: &[u32]) -> Result<()> {
        self.check_range(offset, values.len())?;
        self.file.seek(SeekFrom::Start(offset as u64 * 4))?;
        let mut bytes = [0u8; IO_CHUNK_VALUES * 4];
        for values in values.chunks(IO_CHUNK_VALUES) {
            let length = values.len() * 4;
            for (output, value) in bytes[..length]
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(values)
            {
                *output = value.to_le_bytes();
            }
            self.file.write_all(&bytes[..length])?;
        }
        Ok(())
    }

    fn check_range(&self, offset: usize, count: usize) -> Result<()> {
        if offset.checked_add(count).is_none_or(|end| end > self.len) {
            return Err(Error::invalid("临时索引访问越界"));
        }
        Ok(())
    }

    pub(crate) fn fill_prefix(&mut self, end: usize, value: u32) -> Result<()> {
        let buffer = vec![value; 16384];
        for offset in (0..end).step_by(buffer.len()) {
            self.write_values(offset, &buffer[..(end - offset).min(buffer.len())])?;
        }
        Ok(())
    }

    pub(crate) fn into_paged(
        mut self,
        size: Size,
        fill: u32,
        budget: usize,
    ) -> Result<PagedIndices> {
        let nx = size.0.div_ceil(16) as usize;
        let ny = size.1.div_ceil(16) as usize;
        let nz = size.2.div_ceil(16) as usize;
        let bytes = (nx as u64)
            .checked_mul(ny as u64)
            .and_then(|n| n.checked_mul(nz as u64))
            .and_then(|n| n.checked_mul(TILE_BYTES))
            .ok_or_else(|| Error::invalid("分页索引长度溢出"))?;
        let mut target = tempfile::tempfile()?;
        target.set_len(bytes)?;
        let groups = (budget / TILE_BYTES as usize).max(1);
        match self.layout {
            IndexLayout::Yzx => {
                for cy in 0..ny {
                    for cz in 0..nz {
                        for cx in (0..nx).step_by(groups) {
                            let count = groups.min(nx - cx);
                            let fill = fill.to_le_bytes();
                            let mut tiles = vec![0; count * TILE_BYTES as usize];
                            for value in tiles.as_chunks_mut::<4>().0 {
                                value.copy_from_slice(&fill);
                            }
                            let length = (count * 16).min(size.0 as usize - cx * 16);
                            let mut row = vec![0; length * 4];
                            for y in 0..16.min(size.1 as usize - cy * 16) {
                                for z in 0..16.min(size.2 as usize - cz * 16) {
                                    let offset = ((cy * 16 + y) * size.2 as usize + cz * 16 + z)
                                        * size.0 as usize
                                        + cx * 16;
                                    self.file.seek(SeekFrom::Start(offset as u64 * 4))?;
                                    self.file.read_exact(&mut row)?;
                                    for (x, value) in row.as_chunks::<4>().0.iter().enumerate() {
                                        let target =
                                            (x / 16 * TILE_VALUES + ((x % 16) << 8) + (z << 4) + y)
                                                * 4;
                                        tiles[target..target + 4].copy_from_slice(value);
                                    }
                                }
                            }
                            let tile = (cy * nz + cz) * nx + cx;
                            target.seek(SeekFrom::Start(tile as u64 * TILE_BYTES))?;
                            target.write_all(&tiles)?;
                        }
                    }
                }
            }
            IndexLayout::Xyz => {
                for cy in 0..ny {
                    for cx in 0..nx {
                        for cz in (0..nz).step_by(groups) {
                            let count = groups.min(nz - cz);
                            let mut tiles = vec![fill; count * TILE_VALUES];
                            let length = (count * 16).min(size.2 as usize - cz * 16);
                            let mut row = vec![fill; length];
                            for x in 0..16.min(size.0 as usize - cx * 16) {
                                for y in 0..16.min(size.1 as usize - cy * 16) {
                                    let offset = ((cx * 16 + x) * size.1 as usize + cy * 16 + y)
                                        * size.2 as usize
                                        + cz * 16;
                                    self.read_values(offset, &mut row)?;
                                    for (z, value) in row.iter().enumerate() {
                                        tiles[z / 16 * TILE_VALUES
                                            + (x << 8)
                                            + ((z % 16) << 4)
                                            + y] = *value;
                                    }
                                }
                            }
                            for tile in 0..count {
                                write_tile(
                                    &mut target,
                                    ((cy * nz + cz + tile) * nx + cx) as u64,
                                    &tiles[tile * TILE_VALUES..(tile + 1) * TILE_VALUES],
                                )?;
                            }
                        }
                    }
                }
            }
        }
        target.flush()?;
        Ok(PagedIndices {
            file: Mutex::new(target),
            size,
            bytes,
        })
    }
}

fn write_tile(file: &mut File, tile: u64, values: &[u32]) -> Result<()> {
    let mut bytes = [0u8; TILE_VALUES * 4];
    for (output, value) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(values) {
        *output = value.to_le_bytes();
    }
    file.seek(SeekFrom::Start(tile * TILE_BYTES))?;
    file.write_all(&bytes)?;
    Ok(())
}

impl PagedIndices {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn fill_subchunk(&self, origin: Pos, output: &mut [u32; TILE_VALUES]) -> Result<()> {
        if origin.0 < 0
            || origin.1 < 0
            || origin.2 < 0
            || origin.0 % 16 != 0
            || origin.1 % 16 != 0
            || origin.2 % 16 != 0
        {
            return Err(Error::invalid("分页读取要求非负且16对齐的局部坐标"));
        }
        if origin.0 as u32 >= self.size.0
            || origin.1 as u32 >= self.size.1
            || origin.2 as u32 >= self.size.2
        {
            output.fill(u32::MAX);
            return Ok(());
        }
        let nx = u64::from(self.size.0.div_ceil(16));
        let nz = u64::from(self.size.2.div_ceil(16));
        let tile = ((origin.1 as u64 / 16 * nz + origin.2 as u64 / 16) * nx) + origin.0 as u64 / 16;
        let mut file = self
            .file
            .lock()
            .map_err(|_| Error::Other("分页索引文件锁已损坏".into()))?;
        file.seek(SeekFrom::Start(tile * TILE_BYTES))?;
        let mut bytes = [0u8; TILE_VALUES * 4];
        file.read_exact(&mut bytes)?;
        for (value, bytes) in output.iter_mut().zip(bytes.as_chunks::<4>().0) {
            *value = u32::from_le_bytes(*bytes);
        }
        Ok(())
    }
}
