use crate::nbt::{Compound, Nbt};
use crate::{Error, FormatId, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Pos(pub i32, pub i32, pub i32);
impl Pos {
    pub fn chunk(self) -> (i32, i32) {
        (self.0.div_euclid(16), self.2.div_euclid(16))
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size(pub u32, pub u32, pub u32);
impl Size {
    pub fn volume(self) -> u64 {
        u64::from(self.0)
            .saturating_mul(u64::from(self.1))
            .saturating_mul(u64::from(self.2))
    }
    pub fn chunk_count(self) -> u64 {
        u64::from(self.0.div_ceil(16)) * u64::from(self.2.div_ceil(16))
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockState {
    pub name: String,
    pub properties: Compound,
}
impl BlockState {
    pub fn new(name: impl Into<String>) -> Self {
        let mut name = name.into();
        if !name.contains(':') {
            name = format!("minecraft:{name}");
        }
        Self {
            name,
            properties: BTreeMap::new(),
        }
    }
    pub fn air() -> Self {
        Self::new("air")
    }
    pub fn is_air(&self) -> bool {
        self.name == "minecraft:air"
    }
    pub fn string(&self) -> String {
        let mut s = self.name.clone();
        if !self.properties.is_empty() {
            s.push('[');
            s.push_str(
                &self
                    .properties
                    .iter()
                    .map(|(k, v)| format!("{k}={}", v.scalar_string()))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            s.push(']');
        }
        s
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub state: BlockState,
    pub extra: Option<BlockState>,
    pub nbt: Option<Compound>,
}
impl Block {
    pub fn new(state: BlockState) -> Self {
        Self {
            state,
            extra: None,
            nbt: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructureData {
    pub format: FormatId,
    pub size: Size,
    pub origin: Pos,
    pub offset: Pos,
    #[serde(skip)]
    pub blocks: BTreeMap<Pos, Block>,
    pub entities: Vec<Compound>,
    pub metadata: Compound,
}
impl StructureData {
    pub fn new(format: FormatId) -> Self {
        Self {
            format,
            size: Size::default(),
            origin: Pos::default(),
            offset: Pos::default(),
            blocks: BTreeMap::new(),
            entities: vec![],
            metadata: BTreeMap::new(),
        }
    }
    pub fn insert(&mut self, pos: Pos, block: Block) {
        self.blocks.insert(pos, block);
    }
    pub fn normalize(&mut self) -> Result<()> {
        if self.blocks.is_empty() {
            return Ok(());
        }
        let min = Pos(
            self.blocks.keys().map(|p| p.0).min().unwrap(),
            self.blocks.keys().map(|p| p.1).min().unwrap(),
            self.blocks.keys().map(|p| p.2).min().unwrap(),
        );
        let max = Pos(
            self.blocks.keys().map(|p| p.0).max().unwrap(),
            self.blocks.keys().map(|p| p.1).max().unwrap(),
            self.blocks.keys().map(|p| p.2).max().unwrap(),
        );
        let width = |lo: i32, hi: i32| {
            u32::try_from(i64::from(hi) - i64::from(lo) + 1)
                .ok()
                .filter(|width| *width <= i32::MAX as u32)
                .ok_or_else(|| Error::invalid("坐标范围超过32位局部坐标容量"))
        };
        self.size = Size(
            width(min.0, max.0)?,
            width(min.1, max.1)?,
            width(min.2, max.2)?,
        );
        self.origin = min;
        self.blocks = std::mem::take(&mut self.blocks)
            .into_iter()
            .map(|(p, b)| (Pos(p.0 - min.0, p.1 - min.1, p.2 - min.2), b))
            .collect();
        Ok(())
    }
    pub fn non_air_count(&self) -> u64 {
        self.blocks.values().filter(|b| !b.state.is_air()).count() as u64
    }
    pub fn get_size(&self) -> Size {
        Size(
            self.size.0.saturating_add(self.offset.0.unsigned_abs()),
            self.size.1.saturating_add(self.offset.1.unsigned_abs()),
            self.size.2.saturating_add(self.offset.2.unsigned_abs()),
        )
    }
    pub fn get_chunks(
        &self,
        positions: &[(i32, i32)],
    ) -> BTreeMap<(i32, i32), BTreeMap<Pos, Block>> {
        let mut result: BTreeMap<_, BTreeMap<_, _>> = positions
            .iter()
            .copied()
            .map(|p| (p, BTreeMap::new()))
            .collect();
        for (p, b) in &self.blocks {
            let p = Pos(
                p.0 + self.offset.0,
                p.1 + self.offset.1,
                p.2 + self.offset.2,
            );
            if let Some(c) = result.get_mut(&p.chunk()) {
                c.insert(
                    Pos(p.0.rem_euclid(16), p.1 - 64, p.2.rem_euclid(16)),
                    b.clone(),
                );
            }
        }
        result
    }
    pub fn get_chunks_nbt(
        &self,
        positions: &[(i32, i32)],
    ) -> BTreeMap<(i32, i32), BTreeMap<Pos, Compound>> {
        self.get_chunks(positions)
            .into_iter()
            .map(|(p, c)| {
                (
                    p,
                    c.into_iter()
                        .filter_map(|(p, b)| b.nbt.map(|n| (p, n)))
                        .collect(),
                )
            })
            .collect()
    }
}
pub fn compound(entries: impl IntoIterator<Item = (String, Nbt)>) -> Compound {
    entries.into_iter().collect()
}
