//! Bedrock 方块状态升级，逐条移植 worldupgrader v1.4.0。
//!
//! 原算法采用 MIT 许可，schema 采用 CC0；来源及哈希见 assets/block_upgrade_schemas。

use crate::model::BlockState;
use crate::nbt::{Compound, Nbt};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

type Properties<T> = BTreeMap<String, BTreeMap<String, T>>;

const SOURCES: &[&str] = &[
    include_str!("../../assets/block_upgrade_schemas/0001_1.9.0_to_1.10.0.json"),
    include_str!("../../assets/block_upgrade_schemas/0011_1.10.0_to_1.12.0.json"),
    include_str!("../../assets/block_upgrade_schemas/0021_1.12.0_to_1.13.0.json"),
    include_str!("../../assets/block_upgrade_schemas/0031_1.13.0_to_1.14.0.json"),
    include_str!("../../assets/block_upgrade_schemas/0041_1.14.0_to_1.16.0.57_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0051_1.16.0.57_beta_to_1.16.0.59_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0061_1.16.0.59_beta_to_1.16.0.68_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0071_1.16.0_to_1.16.100.json"),
    include_str!("../../assets/block_upgrade_schemas/0081_1.16.200_to_1.16.210.json"),
    include_str!("../../assets/block_upgrade_schemas/0091_1.17.10_to_1.17.30.json"),
    include_str!("../../assets/block_upgrade_schemas/0101_1.17.30_to_1.17.40.json"),
    include_str!("../../assets/block_upgrade_schemas/0111_1.18.0_to_1.18.10.json"),
    include_str!("../../assets/block_upgrade_schemas/0121_1.18.10_to_1.18.20.27_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0131_1.18.20.27_beta_to_1.18.30.json"),
    include_str!("../../assets/block_upgrade_schemas/0141_1.18.30_to_1.19.0.34_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0151_1.19.0.34_beta_to_1.19.20.json"),
    include_str!("../../assets/block_upgrade_schemas/0161_1.19.50_to_1.19.60.26_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0171_1.19.60_to_1.19.70.26_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0181_1.19.70_to_1.19.80.24_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0191_1.19.80.24_beta_to_1.20.0.23_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0201_1.20.0.23_beta_to_1.20.10.24_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0211_1.20.10.24_beta_to_1.20.20.23_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0221_1.20.20.23_beta_to_1.20.30.22_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0231_1.20.30.22_beta_to_1.20.40.24_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0241_1.20.40.24_beta_to_1.20.50.23_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0251_1.20.50.23_beta_to_1.20.60.26_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0261_1.20.60.26_beta_to_1.20.70.24_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0271_1.20.70.24_beta_to_1.20.80.24_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0281_1.20.80.24_beta_to_1.21.0.25_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0291_1.21.0.3_to_1.21.1.0.json"),
    include_str!("../../assets/block_upgrade_schemas/0301_1.21.0.25_beta_to_1.21.20.24_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0311_1.21.20.24_beta_to_1.21.30.24_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0321_1.21.30.24_beta_to_1.21.40.25_beta.json"),
    include_str!("../../assets/block_upgrade_schemas/0331_1.21.50.29_beta_to_1.21.60.28_beta.json"),
    include_str!(
        "../../assets/block_upgrade_schemas/0341_1.21.100.23_beta_to_1.21.110.26_beta.json"
    ),
];

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SchemaModel {
    max_version_major: i32,
    max_version_minor: i32,
    max_version_patch: i32,
    max_version_revision: i32,
    renamed_ids: BTreeMap<String, String>,
    added_properties: Properties<TagModel>,
    removed_properties: BTreeMap<String, Vec<String>>,
    renamed_properties: Properties<String>,
    remapped_property_values: Properties<String>,
    remapped_property_values_index: BTreeMap<String, Vec<ValueRemapModel>>,
    flattened_properties: BTreeMap<String, FlattenInfo>,
    remapped_states: BTreeMap<String, Vec<BlockRemapModel>>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct TagModel {
    byte: Option<u8>,
    int: Option<i32>,
    string: Option<String>,
}

impl TagModel {
    fn into_nbt(self) -> Nbt {
        if let Some(value) = self.byte {
            Nbt::Byte(value as i8)
        } else if let Some(value) = self.int {
            Nbt::Int(value)
        } else {
            Nbt::String(
                self.string
                    .expect("内嵌 schema 的标签必须包含 byte/int/string"),
            )
        }
    }
}

#[derive(Deserialize)]
struct ValueRemapModel {
    old: TagModel,
    new: TagModel,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct FlattenInfo {
    prefix: String,
    flattened_property: String,
    suffix: String,
    flattened_value_remaps: BTreeMap<String, String>,
    flattened_property_type: String,
}

impl FlattenInfo {
    fn apply(&self, old_name: &str, properties: &mut Compound) -> String {
        let value = properties.get(&self.flattened_property);
        let key = match (self.flattened_property_type.as_str(), value) {
            ("" | "string", Some(Nbt::String(value))) => value.clone(),
            ("int", Some(Nbt::Int(value))) => value.to_string(),
            // Go 的 byte 为无符号数，NBT Byte 保留同一个字节的位模式。
            ("byte", Some(Nbt::Byte(value))) => (*value as u8).to_string(),
            _ => return old_name.to_owned(),
        };
        if key.is_empty() {
            return old_name.to_owned();
        }
        properties.remove(&self.flattened_property);
        let embedded = self.flattened_value_remaps.get(&key).unwrap_or(&key);
        format!("{}{embedded}{}", self.prefix, self.suffix)
    }
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct BlockRemapModel {
    old_state: Option<BTreeMap<String, TagModel>>,
    new_name: String,
    new_flattened_name: FlattenInfo,
    new_state: Option<BTreeMap<String, TagModel>>,
    copied_state: Vec<String>,
}

struct BlockRemap {
    old_properties: Compound,
    new_name: String,
    new_flattened_name: FlattenInfo,
    new_properties: Compound,
    copied_properties: Vec<String>,
}

struct Schema {
    version: i32,
    renamed_ids: BTreeMap<String, String>,
    added_properties: BTreeMap<String, Compound>,
    removed_properties: BTreeMap<String, Vec<String>>,
    renamed_properties: Properties<String>,
    flattened_properties: BTreeMap<String, FlattenInfo>,
    remapped_values: Properties<Vec<(Nbt, Nbt)>>,
    remapped_states: BTreeMap<String, Vec<BlockRemap>>,
}

fn properties(model: BTreeMap<String, TagModel>) -> Compound {
    model
        .into_iter()
        .map(|(key, tag)| (key, tag.into_nbt()))
        .collect()
}

impl From<SchemaModel> for Schema {
    fn from(model: SchemaModel) -> Self {
        let index: BTreeMap<_, Vec<_>> = model
            .remapped_property_values_index
            .into_iter()
            .map(|(key, pairs)| {
                (
                    key,
                    pairs
                        .into_iter()
                        .map(|pair| (pair.old.into_nbt(), pair.new.into_nbt()))
                        .collect(),
                )
            })
            .collect();
        let remapped_values = model
            .remapped_property_values
            .into_iter()
            .map(|(block, fields)| {
                let fields = fields
                    .into_iter()
                    .map(|(field, key)| {
                        (
                            field,
                            index
                                .get(&key)
                                .expect("内嵌 schema 的属性映射引用必须存在")
                                .clone(),
                        )
                    })
                    .collect();
                (block, fields)
            })
            .collect();
        let remapped_states = model
            .remapped_states
            .into_iter()
            .map(|(block, remaps)| {
                let remaps = remaps
                    .into_iter()
                    .map(|remap| BlockRemap {
                        old_properties: properties(remap.old_state.unwrap_or_default()),
                        new_name: remap.new_name,
                        new_flattened_name: remap.new_flattened_name,
                        new_properties: properties(remap.new_state.unwrap_or_default()),
                        copied_properties: remap.copied_state,
                    })
                    .collect();
                (block, remaps)
            })
            .collect();
        Self {
            version: (model.max_version_major << 24)
                | (model.max_version_minor << 16)
                | (model.max_version_patch << 8)
                | model.max_version_revision,
            renamed_ids: model.renamed_ids,
            added_properties: model
                .added_properties
                .into_iter()
                .map(|(name, values)| (name, properties(values)))
                .collect(),
            removed_properties: model.removed_properties,
            renamed_properties: model.renamed_properties,
            flattened_properties: model.flattened_properties,
            remapped_values,
            remapped_states,
        }
    }
}

impl Schema {
    fn remap_value(&self, block: &str, field: &str, value: Nbt) -> Nbt {
        if let Some((_, new)) = self
            .remapped_values
            .get(block)
            .and_then(|fields| fields.get(field))
            .and_then(|pairs| pairs.iter().find(|(old, _)| *old == value))
        {
            return new.clone();
        }
        value
    }

    fn apply_state_remap(&self, state: &mut BlockState) -> bool {
        let Some(remaps) = self.remapped_states.get(&state.name) else {
            return false;
        };
        for remap in remaps {
            if !remap
                .old_properties
                .iter()
                .all(|(key, value)| state.properties.get(key) == Some(value))
            {
                continue;
            }
            let mut new_properties = remap.new_properties.clone();
            for key in &remap.copied_properties {
                if let Some(value) = state.properties.get(key) {
                    new_properties.insert(key.clone(), value.clone());
                }
            }
            // 展平读取旧字段，copiedState 在移除展平字段之前完成。
            state.name = if remap.new_name.is_empty() {
                remap
                    .new_flattened_name
                    .apply(&state.name, &mut state.properties)
            } else {
                remap.new_name.clone()
            };
            state.properties = new_properties;
            return true;
        }
        false
    }

    fn apply(&self, state: &mut BlockState) {
        if self.apply_state_remap(state) {
            return;
        }
        let old_name = state.name.clone();
        if let Some(name) = self.renamed_ids.get(&old_name) {
            assert!(
                !self.flattened_properties.contains_key(&old_name),
                "同一 schema 不得同时重命名和展平方块"
            );
            state.name = name.clone();
        } else if let Some(info) = self.flattened_properties.get(&old_name) {
            state.name = info.apply(&old_name, &mut state.properties);
        }
        if let Some(fields) = self.added_properties.get(&old_name) {
            for (key, value) in fields {
                state
                    .properties
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
            }
        }
        if let Some(fields) = self.removed_properties.get(&old_name) {
            for key in fields {
                state.properties.remove(key);
            }
        }
        if let Some(fields) = self.renamed_properties.get(&old_name) {
            for (old, new) in fields {
                if let Some(value) = state.properties.remove(old) {
                    state
                        .properties
                        .insert(new.clone(), self.remap_value(&old_name, old, value));
                }
            }
        }
        if let Some(fields) = self.remapped_values.get(&old_name) {
            for key in fields.keys() {
                if let Some(value) = state.properties.get(key).cloned() {
                    state
                        .properties
                        .insert(key.clone(), self.remap_value(&old_name, key, value));
                }
            }
        }
    }
}

/// 按原输入版本依次应用所有适用的 schema，保留 NBT 标量类型。
///
/// 不执行目标运行时调色板查找，由调用方决定未知状态的处理方式。
pub fn upgrade(state: &BlockState, version: i32) -> BlockState {
    static SCHEMAS: OnceLock<Vec<Schema>> = OnceLock::new();
    let schemas = SCHEMAS.get_or_init(|| {
        SOURCES
            .iter()
            .map(|source| {
                let model: SchemaModel =
                    serde_json::from_str(source).expect("内嵌的原始升级 schema 必须合法");
                Schema::from(model)
            })
            .collect()
    });
    let mut result = state.clone();
    for schema in schemas {
        // 必须使用原输入版本；后一个 schema 不能因前一次升级而被跳过。
        if version <= schema.version {
            schema.apply(&mut result);
        }
    }
    result
}
