use crate::{Error, Result};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum FormatId {
    Schematic,
    SchemV1,
    SchemV2,
    Litematic,
    MCStructure,
    MCWorld,
    BDX,
    Construction,
    AxiomBP,
    MCFunction,
    KBDX,
    IBImport,
    MianYangV1,
    MianYangV2,
    MianYangV3,
    MianYangV4,
    GangBanV1,
    GangBanV2,
    GangBanV3,
    GangBanV4,
    GangBanV5,
    GangBanV6,
    GangBanV7,
    RunAway,
    QingXuV1,
    TimeBuilderV1,
    FuHongV1,
    FuHongV2,
    FuHongV3,
    FuHongV4,
    FuHongV5,
    BDS,
    SIBI,
    BCF,
    TIBI,
    CovStructure,
    NexusNP,
}
impl FormatId {
    pub const ALL: [Self; 37] = [
        Self::Schematic,
        Self::SchemV1,
        Self::SchemV2,
        Self::Litematic,
        Self::MCStructure,
        Self::MCWorld,
        Self::BDX,
        Self::Construction,
        Self::AxiomBP,
        Self::MCFunction,
        Self::KBDX,
        Self::IBImport,
        Self::MianYangV1,
        Self::MianYangV2,
        Self::MianYangV3,
        Self::MianYangV4,
        Self::GangBanV1,
        Self::GangBanV2,
        Self::GangBanV3,
        Self::GangBanV4,
        Self::GangBanV5,
        Self::GangBanV6,
        Self::GangBanV7,
        Self::RunAway,
        Self::QingXuV1,
        Self::TimeBuilderV1,
        Self::FuHongV1,
        Self::FuHongV2,
        Self::FuHongV3,
        Self::FuHongV4,
        Self::FuHongV5,
        Self::BDS,
        Self::SIBI,
        Self::BCF,
        Self::TIBI,
        Self::CovStructure,
        Self::NexusNP,
    ];
    pub fn name(self) -> &'static str {
        const NAMES: [&str; 37] = [
            "Schematic",
            "SchemV1",
            "SchemV2",
            "Litematic",
            "MCStructure",
            "MCWorld",
            "BDX",
            "Construction",
            "AxiomBP",
            "MCFunction",
            "KBDX",
            "IBImport",
            "MianYangV1",
            "MianYangV2",
            "MianYangV3",
            "MianYangV4",
            "GangBanV1",
            "GangBanV2",
            "GangBanV3",
            "GangBanV4",
            "GangBanV5",
            "GangBanV6",
            "GangBanV7",
            "RunAway",
            "QingXuV1",
            "TimeBuilderV1",
            "FuHongV1",
            "FuHongV2",
            "FuHongV3",
            "FuHongV4",
            "FuHongV5",
            "BDS",
            "SIBI",
            "BCF",
            "TIBI",
            "CovStructure",
            "NexusNP",
        ];
        NAMES[self as usize]
    }
    pub fn from_id(id: u8) -> Result<Self> {
        Self::ALL
            .get(id as usize)
            .copied()
            .ok_or_else(|| Error::invalid(format!("无效格式 ID: {id}")))
    }
    pub fn parse(name: &str) -> Result<Self> {
        if let Ok(id) = name.parse::<u8>() {
            return Self::from_id(id);
        }
        Self::ALL
            .into_iter()
            .find(|f| f.name().eq_ignore_ascii_case(name))
            .ok_or_else(|| Error::invalid(format!("未知格式: {name}")))
    }
}
