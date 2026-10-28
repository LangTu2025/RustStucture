//! 方块表与旧版、Java、Bedrock 状态间的兼容映射。
pub mod upgrade;
use crate::model::BlockState;
use crate::nbt::{self, Compound, Encoding, Nbt, Reader};
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::sync::OnceLock;

#[derive(Default)]
struct Group {
    legacy: HashMap<u16, usize>,
    states: Vec<(Compound, usize)>,
}

struct Registry {
    native: HashMap<u32, BlockState>,
    defaults: HashMap<String, BlockState>,
    nemc: Vec<BlockState>,
    groups: HashMap<String, Group>,
    java: HashMap<String, Vec<(Compound, String)>>,
    schematic: Vec<String>,
}

fn unpack(bytes: &[u8]) -> String {
    let mut value = String::new();
    brotli::Decompressor::new(bytes, 65536)
        .read_to_string(&mut value)
        .expect("内置方块数据损坏");
    value
}

fn base_name(name: &str) -> &str {
    name.trim().split_once(':').map_or(name.trim(), |(_, n)| n)
}

pub fn parse_properties(text: &str) -> Compound {
    let text = text.trim().trim_matches(['[', ']', '{', '}']);
    let has_unquoted_namespace = text
        .split(',')
        .any(|entry| entry.trim().starts_with("minecraft:"));
    if !text.contains('=')
        && !has_unquoted_namespace
        && let Ok(Nbt::Compound(properties)) = nbt::parse_snbt(&format!("{{{text}}}"))
    {
        return properties;
    }
    let mut result = Compound::new();
    for entry in text.split(',').filter(|s| !s.trim().is_empty()) {
        let entry = entry.trim();
        let pair = entry.split_once('=').or_else(|| {
            let mut quote = None;
            let mut escaped = false;
            let mut separators = Vec::new();
            for (index, character) in entry.char_indices() {
                if escaped {
                    escaped = false;
                } else if character == '\\' && quote.is_some() {
                    escaped = true;
                } else if quote == Some(character) {
                    quote = None;
                } else if quote.is_none() && matches!(character, '\'' | '"') {
                    quote = Some(character);
                } else if quote.is_none() && character == ':' {
                    separators.push(index);
                }
            }
            let index = if entry.starts_with("minecraft:") && separators.len() > 1 {
                separators[1]
            } else {
                *separators.first()?
            };
            Some((&entry[..index], &entry[index + 1..]))
        });
        if let Some((key, value)) = pair {
            let key = key.trim().trim_matches(['\'', '"']).to_owned();
            let value = value.trim();
            let value = nbt::parse_snbt(value)
                .unwrap_or_else(|_| Nbt::String(value.trim_matches(['\'', '"']).into()));
            result.insert(key, value);
        }
    }
    result
}

fn load_native() -> (HashMap<u32, BlockState>, HashMap<String, BlockState>) {
    let mut reader = Reader::new(
        include_bytes!("../assets/block_states.bin"),
        Encoding::NetworkLittleEndian,
    );
    let string_count = reader.short().expect("方块字符串表长度") as u16;
    let strings: Vec<_> = (0..string_count)
        .map(|_| reader.string().expect("方块字符串表"))
        .collect();
    let version_count = reader.short().expect("方块版本表长度") as u16;
    for _ in 0..version_count {
        reader.int().expect("方块版本");
    }
    let key_count = reader.short().expect("方块属性表长度") as u16;
    let keys: Vec<_> = (0..key_count)
        .map(|_| {
            let index = reader.varuint(5).expect("属性名称索引") as usize;
            let kind = reader.byte().expect("属性类型");
            (strings[index].clone(), kind)
        })
        .collect();
    let mut native = HashMap::new();
    let mut defaults = HashMap::new();
    while reader.pos < reader.data.len() {
        let name_index = reader.varuint(5).expect("方块名称索引") as usize;
        reader.varuint(5).expect("方块版本索引");
        let count = reader.short().expect("方块属性数量") as u16;
        let mut properties = Compound::new();
        for _ in 0..count {
            let key = reader.varuint(5).expect("方块属性索引") as usize;
            let size = reader.varuint(5).expect("方块属性长度") as usize;
            let bytes = reader.take(size).expect("方块属性内容");
            let mut property = Reader::new(bytes, Encoding::NetworkLittleEndian);
            let value = match keys[key].1 {
                0 => {
                    Nbt::String(strings[property.varuint(5).expect("属性字符串") as usize].clone())
                }
                1 => Nbt::Int(property.int().expect("属性整数")),
                _ => Nbt::Byte(property.byte().expect("属性字节") as i8),
            };
            properties.insert(keys[key].0.clone(), value);
        }
        let state = BlockState {
            name: strings[name_index].clone(),
            properties,
        };
        defaults
            .entry(state.name.clone())
            .or_insert_with(|| state.clone());
        native.insert(runtime_id(&state), state);
    }
    (native, defaults)
}

fn add_record(
    groups: &mut HashMap<String, Group>,
    name: &str,
    value: &str,
    id: usize,
    overwrite: bool,
) {
    let group = groups.entry(base_name(name).into()).or_default();
    if let Ok(data) = value.parse::<u16>() {
        if overwrite {
            group.legacy.insert(data, id);
        } else {
            group.legacy.entry(data).or_insert(id);
        }
    } else {
        let properties = parse_properties(value);
        if let Some(entry) = group.states.iter_mut().find(|(p, _)| *p == properties) {
            if overwrite {
                entry.1 = id;
            }
        } else {
            group.states.push((properties, id));
        }
    }
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let (native, defaults) = load_native();
        let text = unpack(include_bytes!("../assets/nemc.br"));
        let mut nemc = Vec::new();
        let mut groups = HashMap::new();
        for line in text.lines().skip(2) {
            let parts: Vec<_> = line.splitn(3, ' ').collect();
            if parts.len() != 3 {
                continue;
            }
            let state = BlockState {
                name: format!("minecraft:{}", parts[0]),
                properties: parse_properties(parts[2]),
            };
            let id = nemc.len();
            add_record(&mut groups, parts[0], parts[1], id, false);
            add_record(&mut groups, parts[0], parts[2], id, false);
            nemc.push(state);
        }
        for (data, overwrite) in [
            (
                include_bytes!("../assets/bedrock_java_to_translate.br").as_slice(),
                false,
            ),
            (
                include_bytes!("../assets/specific_legacy_value_to_translate.br").as_slice(),
                true,
            ),
        ] {
            let records = unpack(data);
            let lines: Vec<_> = records.lines().collect();
            for entry in lines.as_chunks::<3>().0 {
                if let Ok(id) = entry[2].trim().parse::<usize>() {
                    add_record(&mut groups, entry[0], entry[1], id, overwrite);
                }
            }
        }
        let mut java: HashMap<String, Vec<(Compound, String)>> = HashMap::new();
        let java_text = unpack(include_bytes!("../assets/bedrock_to_java_translate.br"));
        let lines: Vec<_> = java_text.lines().collect();
        for entry in lines.as_chunks::<4>().0 {
            if entry[1].parse::<u16>().is_ok() {
                continue;
            }
            let properties = parse_properties(entry[1]);
            let java_properties = parse_properties(entry[3]);
            let state = BlockState {
                name: BlockState::new(entry[2]).name,
                properties: java_properties,
            };
            java.entry(base_name(entry[0]).into())
                .or_default()
                .push((properties, state.string()));
        }
        let schematic = serde_json::from_str(
            include_str!("../assets/schematic_names.json").trim_start_matches('\u{feff}'),
        )
        .expect("Schematic 名称表损坏");
        Registry {
            native,
            defaults,
            nemc,
            groups,
            java,
            schematic,
        }
    })
}

fn equivalent(a: &Nbt, b: &Nbt) -> bool {
    a == b
        || a.as_i64().zip(b.as_i64()).is_some_and(|(a, b)| a == b)
        || a.scalar_string().trim_matches('"') == b.scalar_string().trim_matches('"')
}

#[derive(Clone, Debug)]
enum SearchValue {
    Bool(bool),
    Int(i32),
    String { value: Vec<u8>, number: Option<i32> },
}

impl SearchValue {
    fn has_bool(&self) -> bool {
        match self {
            Self::Bool(_) => true,
            Self::Int(value) => matches!(value, 0 | 1),
            Self::String { number, .. } => number.is_some_and(|value| matches!(value, 0 | 1)),
        }
    }

    fn bool_value(&self) -> bool {
        match self {
            Self::Bool(value) => *value,
            Self::Int(value) => *value == 1,
            Self::String { number, .. } => number == &Some(1),
        }
    }

    fn has_int(&self) -> bool {
        match self {
            Self::Bool(_) | Self::Int(_) => true,
            Self::String { number, .. } => number.is_some(),
        }
    }

    fn int_value(&self) -> i32 {
        match self {
            Self::Bool(value) => i32::from(*value),
            Self::Int(value) => *value,
            Self::String { number, .. } => number.unwrap(),
        }
    }

    fn string_value(&self) -> Vec<u8> {
        match self {
            Self::Bool(value) => {
                if *value {
                    b"true".to_vec()
                } else {
                    b"false".to_vec()
                }
            }
            Self::Int(value) => value.to_string().into_bytes(),
            Self::String { value, .. } => value.clone(),
        }
    }

    fn precise_value(&self) -> Vec<u8> {
        if self.has_bool() {
            let value = self.bool_value();
            match self {
                Self::Bool(_) => {
                    return if value { b"1b" } else { b"0b" }.to_vec();
                }
                Self::String { value: text, .. }
                    if text == b"true" || text == b"1b" || text == b"false" || text == b"0b" =>
                {
                    return if value { b"1b" } else { b"0b" }.to_vec();
                }
                _ => {}
            }
        }
        self.string_value()
    }

    fn fuzzy_equal(&self, other: &Self) -> bool {
        match self {
            Self::Bool(value) => {
                if other.has_bool() {
                    *value == other.bool_value()
                } else if other.has_int() {
                    if other.int_value() == 0 {
                        !value
                    } else {
                        *value
                    }
                } else {
                    let text = other.string_value();
                    if *value {
                        matches!(text.as_slice(), b"true" | b"1b" | b"1")
                    } else {
                        matches!(text.as_slice(), b"false" | b"0b" | b"0")
                    }
                }
            }
            Self::Int(value) => {
                if other.has_bool() {
                    if self.has_bool() {
                        self.bool_value() == other.bool_value()
                    } else {
                        other.fuzzy_equal(self)
                    }
                } else if other.has_int() {
                    *value == other.int_value()
                } else {
                    let text = other.string_value();
                    if *value == 1 {
                        matches!(text.as_slice(), b"true" | b"1b" | b"1")
                    } else if *value == 0 {
                        matches!(text.as_slice(), b"false" | b"0b" | b"0")
                    } else {
                        value.to_string().as_bytes() == text
                    }
                }
            }
            Self::String { value, .. } => {
                if other.has_bool() {
                    if self.has_bool() {
                        self.bool_value() == other.bool_value()
                    } else {
                        other.fuzzy_equal(self)
                    }
                } else if other.has_int() {
                    if self.has_int() {
                        self.int_value() == other.int_value()
                    } else {
                        other.fuzzy_equal(self)
                    }
                } else {
                    *value == other.string_value()
                }
            }
        }
    }
}

fn search_string(mut value: Vec<u8>, remove_all_quotes: bool) -> SearchValue {
    if remove_all_quotes {
        value.retain(|byte| *byte != b'"');
    }
    if value.first() == Some(&b'"') {
        value.remove(0);
    }
    if value.last() == Some(&b'"') {
        value.pop();
    }
    let number = if matches!(value.as_slice(), b"true" | b"1b" | b"1") {
        Some(1)
    } else if matches!(value.as_slice(), b"false" | b"0b" | b"0") {
        Some(0)
    } else {
        std::str::from_utf8(&value)
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .map(|value| value as i32)
            .filter(|value| *value != -1)
    };
    SearchValue::String { value, number }
}

fn go_format_nbt(value: &Nbt) -> String {
    match value {
        Nbt::End => "<nil>".into(),
        Nbt::Byte(value) => value.to_string(),
        Nbt::Short(value) => value.to_string(),
        Nbt::Int(value) => value.to_string(),
        Nbt::Long(value) => value.to_string(),
        Nbt::Float(value) => value.to_string(),
        Nbt::Double(value) => value.to_string(),
        Nbt::ByteArray(values) => format!(
            "[{}]",
            values
                .iter()
                .map(|value| (*value as u8).to_string())
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Nbt::String(value) => value.clone(),
        Nbt::RawString(value) => String::from_utf8_lossy(value).into_owned(),
        Nbt::List(values) => format!(
            "[{}]",
            values
                .iter()
                .map(go_format_nbt)
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Nbt::Compound(values) => format!(
            "map[{}]",
            values
                .iter()
                .map(|(key, value)| format!("{key}:{}", go_format_nbt(value)))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Nbt::RawCompound(values) => format!(
            "map[{}]",
            values
                .iter()
                .map(|(key, value)| format!(
                    "{}:{}",
                    String::from_utf8_lossy(key),
                    go_format_nbt(value)
                ))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Nbt::IntArray(values) => format!(
            "[{}]",
            values
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Nbt::LongArray(values) => format!(
            "[{}]",
            values
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        ),
    }
}

fn query_value(value: &Nbt) -> std::result::Result<SearchValue, String> {
    match value {
        Nbt::Byte(value) if *value == 0 || *value == 1 => Ok(SearchValue::Bool(*value != 0)),
        Nbt::Byte(value) => Err(format!(
            "as an uint8 in prop, value should be 0 or 1, get {}",
            *value as u8
        )),
        Nbt::Int(value) => Ok(SearchValue::Int(*value)),
        Nbt::Double(value) => Ok(SearchValue::Int(go_float64_to_i32(*value))),
        Nbt::String(value) => Ok(search_string(value.as_bytes().to_vec(), true)),
        Nbt::RawString(value) => Ok(search_string(value.clone(), true)),
        value => Err(format!(
            "as a prop value from nbt, value can only be uint8/int32/string, get {}",
            go_format_nbt(value)
        )),
    }
}

fn go_float64_to_i32(value: f64) -> i32 {
    if !value.is_finite() || value >= 2_147_483_648.0 || value < -2_147_483_648.0 {
        i32::MIN
    } else {
        value.trunc() as i32
    }
}

fn normalized_properties(properties: &Compound) -> BTreeMap<String, SearchValue> {
    properties
        .iter()
        .map(|(key, value)| {
            (
                key.replace('"', "").replace("minecraft:", ""),
                match value {
                    Nbt::Byte(value) if *value == 0 || *value == 1 => {
                        SearchValue::Bool(*value != 0)
                    }
                    Nbt::Byte(value) => search_string(format!("{}b", value).into_bytes(), false),
                    Nbt::Int(value) => SearchValue::Int(*value),
                    Nbt::String(value) => search_string(value.as_bytes().to_vec(), false),
                    Nbt::RawString(value) => search_string(value.clone(), false),
                    Nbt::Short(value) => search_string(format!("{value}s").into_bytes(), false),
                    Nbt::Long(value) => search_string(format!("{value}l").into_bytes(), false),
                    Nbt::Float(value) => search_string(format!("{value}f").into_bytes(), false),
                    Nbt::Double(value) => search_string(value.to_string().into_bytes(), false),
                    other => search_string(nbt::to_snbt(other).into_bytes(), false),
                },
            )
        })
        .collect()
}

fn precise_properties(properties: &BTreeMap<String, SearchValue>) -> Vec<u8> {
    let mut output = Vec::from(b"{".as_slice());
    for (index, (key, value)) in properties.iter().enumerate() {
        if index != 0 {
            output.push(b',');
        }
        output.extend_from_slice(key.as_bytes());
        output.push(b':');
        output.extend_from_slice(&value.precise_value());
    }
    output.push(b'}');
    output
}

fn property_score(candidate: &Compound, query: &BTreeMap<String, SearchValue>) -> (u8, u8) {
    if query.is_empty() {
        return (0, 0);
    }
    let candidate = normalized_properties(candidate);
    let mut same = 0u8;
    let mut different = 0u8;
    let mut redundant = 0u8;
    let mut missing = 0u8;
    for (key, query_value) in query {
        match candidate.get(key) {
            Some(candidate_value) if candidate_value.fuzzy_equal(query_value) => {
                same = same.wrapping_add(1);
            }
            Some(_) => different = different.wrapping_add(1),
            None => redundant = redundant.wrapping_add(1),
        }
    }
    for key in candidate.keys() {
        if !query.contains_key(key) {
            missing = missing.wrapping_add(1);
        }
    }
    (
        same,
        different.wrapping_add(redundant).wrapping_add(missing),
    )
}

fn bedrock_candidate(name: &str, query: &BTreeMap<String, SearchValue>) -> Option<BlockState> {
    let mut names = name.trim().split(':');
    let block_name = match (names.next()?, names.next(), names.next()) {
        (name, None, None) => name,
        (_, Some(name), None) => name,
        _ => panic!("invalid block name namespace"),
    };
    let registry = registry();
    let group = registry.groups.get(block_name)?;
    let precise = precise_properties(query);
    let (id, _) = group
        .states
        .iter()
        .find(|(properties, _)| precise_properties(&normalized_properties(properties)) == precise)
        .map(|(_, id)| (*id, (0, 0)))
        .or_else(|| {
            group
                .states
                .iter()
                .map(|(properties, id)| (*id, property_score(properties, query)))
                .reduce(|best, candidate| {
                    if candidate.1.0 > best.1.0
                        || (candidate.1.0 == best.1.0 && candidate.1.1 < best.1.1)
                    {
                        candidate
                    } else {
                        best
                    }
                })
        })?;
    registry.nemc.get(id).cloned()
}

fn best_match<'a, T>(candidates: &'a [(Compound, T)], query: &Compound) -> Option<&'a T> {
    if let Some((_, value)) = candidates.iter().find(|(properties, _)| {
        properties.len() == query.len()
            && properties
                .iter()
                .all(|(key, value)| query.get(key).is_some_and(|other| equivalent(value, other)))
    }) {
        return Some(value);
    }
    if query.is_empty() {
        return candidates.first().map(|(_, value)| value);
    }
    let mut best = None;
    let mut best_score = (0, usize::MAX);
    for (properties, value) in candidates {
        let same = properties
            .iter()
            .filter(|(key, val)| query.get(*key).is_some_and(|q| equivalent(q, val)))
            .count();
        let differences = properties.len() + query.len() - same * 2;
        if best.is_none()
            || same > best_score.0
            || (same == best_score.0 && differences < best_score.1)
        {
            best = Some(value);
            best_score = (same, differences);
        }
    }
    best
}

fn native_state(state: &BlockState) -> BlockState {
    let registry = registry();
    registry
        .native
        .get(&runtime_id(state))
        .or_else(|| registry.defaults.get(&state.name))
        .cloned()
        .unwrap_or_else(|| BlockState::new("unknown"))
}

pub fn legacy(name: &str, data: u16) -> BlockState {
    let registry = registry();
    let Some(group) = registry.groups.get(base_name(name)) else {
        return BlockState::new("unknown");
    };
    let id = group
        .legacy
        .get(&data)
        .copied()
        .or_else(|| group.states.get(data as usize).map(|(_, id)| *id))
        .or_else(|| group.states.first().map(|(_, id)| *id));
    id.and_then(|id| registry.nemc.get(id))
        .map(native_state)
        .unwrap_or_else(|| BlockState::new("unknown"))
}

pub fn java(text: &str) -> BlockState {
    let text = text.trim();
    if let Some((name, data)) = text.rsplit_once(' ')
        && let Ok(data) = data.parse::<u16>()
    {
        return legacy(name, data);
    }
    let index = text.find(['[', '{']).unwrap_or(text.len());
    let name = text[..index].trim();
    let properties = parse_properties(&text[index..]);
    let registry = registry();
    registry
        .groups
        .get(base_name(name))
        .and_then(|group| best_match(&group.states, &properties))
        .and_then(|id| registry.nemc.get(*id))
        .map(native_state)
        .unwrap_or_else(|| BlockState::new("unknown"))
}

fn bedrock_fuzzy_lookup(
    name: &str,
    properties: &Compound,
) -> std::result::Result<Option<BlockState>, String> {
    let mut query = BTreeMap::new();
    for (key, value) in properties {
        let value = query_value(value)?;
        query.insert(key.replace('"', "").replace("minecraft:", ""), value);
    }
    Ok(bedrock_candidate(name, &query))
}

pub fn bedrock(name: &str, properties: Compound) -> BlockState {
    match bedrock_fuzzy_lookup(name, &properties) {
        Ok(Some(state)) => state,
        Ok(None) => BlockState::air(),
        Err(error) => {
            println!("{error}");
            BlockState::air()
        }
    }
}

pub(crate) fn bedrock_fuzzy_runtime(name: &str, properties: Compound) -> BlockState {
    match bedrock_fuzzy_lookup(name, &properties) {
        Ok(Some(state)) => native_state(&state),
        Ok(None) => BlockState::new("unknown"),
        Err(error) => {
            println!("{error}");
            BlockState::new("unknown")
        }
    }
}

pub(crate) fn bedrock_exact(name: &str, properties: Compound) -> BlockState {
    native_state(&BlockState {
        name: BlockState::new(name).name,
        properties,
    })
}

/// 按持久化调色板的游戏版本升级，再应用与 BWO 相同的原生状态回退。
pub fn bedrock_versioned(name: &str, properties: Compound, version: i32) -> BlockState {
    let state = BlockState {
        name: BlockState::new(name.to_lowercase()).name,
        properties,
    };
    native_state(&upgrade::upgrade(&state, version))
}

pub fn to_java(state: &BlockState) -> String {
    registry()
        .java
        .get(base_name(&state.name))
        .and_then(|states| best_match(states, &state.properties))
        .cloned()
        .unwrap_or_else(|| "minecraft:air".into())
}

pub fn to_legacy(state: &BlockState) -> (String, u16) {
    let registry = registry();
    if let Some(group) = registry.groups.get(base_name(&state.name)) {
        let mut values: Vec<_> = group.legacy.iter().collect();
        values.sort_by_key(|(value, _)| **value);
        for (value, id) in values {
            if registry
                .nemc
                .get(*id)
                .is_some_and(|candidate| native_state(candidate) == *state)
            {
                return (state.name.clone(), *value);
            }
        }
    }
    (state.name.clone(), 0)
}

pub fn from_schematic(id: u8, data: u8) -> BlockState {
    let registry = registry();
    let name = registry
        .schematic
        .get(id as usize)
        .map_or("air", String::as_str);
    let state = legacy(name, u16::from(data));
    if state.is_air() {
        legacy(name, 0)
    } else {
        state
    }
}

pub fn to_schematic(state: &BlockState) -> (u8, u8) {
    static REVERSE: OnceLock<HashMap<u32, (u8, u8)>> = OnceLock::new();
    let reverse = REVERSE.get_or_init(|| {
        let mut reverse = HashMap::new();
        for id in 0..=255u8 {
            for data in 0..=255u8 {
                let runtime = runtime_id(&from_schematic(id, data));
                let entry = reverse.entry(runtime).or_insert((id, data));
                if data == 0 && entry.1 != 0 {
                    *entry = (id, data);
                }
            }
        }
        reverse
    });
    reverse.get(&runtime_id(state)).copied().unwrap_or((0, 0))
}

/// BWO 使用排序后的 LittleEndian NBT 的 FNV-1a 哈希作为网络 RuntimeID。
pub fn runtime_id(state: &BlockState) -> u32 {
    if base_name(&state.name) == "unknown" {
        return u32::MAX - 1;
    }
    let mut fields = BTreeMap::new();
    fields.insert("name".into(), Nbt::String(state.name.clone()));
    fields.insert("states".into(), Nbt::Compound(state.properties.clone()));
    let bytes = nbt::encode("", &Nbt::Compound(fields), Encoding::LittleEndian)
        .expect("方块状态包含无效属性");
    bytes.into_iter().fold(0x811c9dc5u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x01000193)
    })
}

pub fn from_runtime_id(id: u32) -> BlockState {
    registry()
        .native
        .get(&id)
        .cloned()
        .unwrap_or_else(|| BlockState::new("unknown"))
}

pub fn bdx_runtime(index: usize) -> BlockState {
    static BDX: OnceLock<Vec<(String, u16)>> = OnceLock::new();
    let values = BDX.get_or_init(|| {
        serde_json::from_slice(include_bytes!("../assets/bdx_runtimeIds_117.json"))
            .expect("BDX 117 表损坏")
    });
    values.get(index).map_or_else(
        || BlockState::new("unknown"),
        |(name, data)| legacy(name, *data),
    )
}

#[cfg(test)]
mod mapping_diagnostics {
    use super::*;

    #[test]
    fn chest_legacy_anchor_is_north() {
        let registry = registry();
        let group = &registry.groups["chest"];
        let id = group.legacy[&2];
        let source = &registry.nemc[id];
        assert_eq!(
            source.properties.get("minecraft:cardinal_direction"),
            Some(&Nbt::String("north".into())),
            "nemc id={id}, source={source:?}"
        );
    }

    #[test]
    fn float64_to_int32_matches_go_boundaries() {
        assert_eq!(go_float64_to_i32(1.9), 1);
        assert_eq!(go_float64_to_i32(-1.9), -1);
        assert_eq!(go_float64_to_i32(2_147_483_647.0), i32::MAX);
        assert_eq!(go_float64_to_i32(2_147_483_648.0), i32::MIN);
        assert_eq!(go_float64_to_i32(-2_147_483_649.0), i32::MIN);
        assert_eq!(go_float64_to_i32(f64::NAN), i32::MIN);
        assert_eq!(go_float64_to_i32(f64::INFINITY), i32::MIN);
        assert_eq!(go_float64_to_i32(f64::NEG_INFINITY), i32::MIN);
    }

    #[test]
    fn fuzzy_string_sentinel_and_amd64_integer_narrowing_match_go() {
        let minus_one = search_string(b"-1".to_vec(), false);
        let padded_minus_one = search_string(b"-0001".to_vec(), false);
        assert!(!minus_one.has_int());
        assert!(!SearchValue::Bool(true).fuzzy_equal(&minus_one));
        assert!(!SearchValue::Int(-1).fuzzy_equal(&padded_minus_one));

        let wrapped = search_string(b"2147483648".to_vec(), false);
        assert!(wrapped.has_int());
        assert_eq!(wrapped.int_value(), i32::MIN);
        assert!(SearchValue::Int(i32::MIN).fuzzy_equal(&wrapped));
        let narrowed_minus_one = search_string(b"4294967295".to_vec(), false);
        assert!(!narrowed_minus_one.has_int());
        assert!(!SearchValue::Bool(true).fuzzy_equal(&narrowed_minus_one));
    }

    #[test]
    fn fuzzy_empty_query_keeps_first_state_and_errors_use_go_value_format() {
        let properties = [("facing".into(), Nbt::String("north".into()))]
            .into_iter()
            .collect();
        assert_eq!(property_score(&properties, &BTreeMap::new()), (0, 0));
        assert_eq!(go_format_nbt(&Nbt::Byte(-1)), "-1");
        assert_eq!(
            go_format_nbt(&Nbt::List(vec![Nbt::Int(1), Nbt::String("x".into())])),
            "[1 x]"
        );
        assert_eq!(
            query_value(&Nbt::Byte(-1)).unwrap_err(),
            "as an uint8 in prop, value should be 0 or 1, get 255"
        );
        assert_eq!(
            query_value(&Nbt::Byte(2)).unwrap_err(),
            "as an uint8 in prop, value should be 0 or 1, get 2"
        );
    }
}
