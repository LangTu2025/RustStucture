//! JSON 流式访问工具。仅保留当前选中的值，跳过字段不构建 DOM。
use crate::Result;
use serde::de::{DeserializeOwned, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::fmt;
use std::io::Read;
use std::marker::PhantomData;

/// 依次处理顶层数组的元素。回调错误立即终止读取。
pub fn visit_array<R, T, F>(reader: R, mut visit: F) -> Result<()>
where
    R: Read,
    T: DeserializeOwned,
    F: FnMut(T) -> Result<()>,
{
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    ArraySeed::<T, F> {
        visit: &mut visit,
        element: PhantomData,
    }
    .deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(())
}

struct ArraySeed<'a, T, F> {
    visit: &'a mut F,
    element: PhantomData<T>,
}

impl<'de, T, F> DeserializeSeed<'de> for ArraySeed<'_, T, F>
where
    T: DeserializeOwned,
    F: FnMut(T) -> Result<()>,
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<(), D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(self)
    }
}

impl<'de, T, F> Visitor<'de> for ArraySeed<'_, T, F>
where
    T: DeserializeOwned,
    F: FnMut(T) -> Result<()>,
{
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON 数组")
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        while let Some(value) = sequence.next_element::<T>()? {
            (self.visit)(value).map_err(serde::de::Error::custom)?;
        }
        Ok(())
    }
}

/// 按顺序筛选顶层对象字段。未选中字段使用 IgnoredAny 解析，无需分配整个值。
pub fn visit_object<R, S, F>(reader: R, mut select: S, mut visit: F) -> Result<()>
where
    R: Read,
    S: FnMut(&str) -> bool,
    F: FnMut(String, Value) -> Result<()>,
{
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    ObjectSeed {
        select: &mut select,
        visit: &mut visit,
    }
    .deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(())
}

struct ObjectSeed<'a, S, F> {
    select: &'a mut S,
    visit: &'a mut F,
}

impl<'de, S, F> DeserializeSeed<'de> for ObjectSeed<'_, S, F>
where
    S: FnMut(&str) -> bool,
    F: FnMut(String, Value) -> Result<()>,
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<(), D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(self)
    }
}

impl<'de, S, F> Visitor<'de> for ObjectSeed<'_, S, F>
where
    S: FnMut(&str) -> bool,
    F: FnMut(String, Value) -> Result<()>,
{
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON 对象")
    }

    fn visit_map<A>(self, mut object: A) -> std::result::Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        while let Some(key) = object.next_key::<String>()? {
            if (self.select)(&key) {
                let value = object.next_value::<Value>()?;
                (self.visit)(key, value).map_err(serde::de::Error::custom)?;
            } else {
                object.next_value::<IgnoredAny>()?;
            }
        }
        Ok(())
    }
}

/// 校验并跳过一个完整 JSON 文档，拒绝截断输入及尾随数据。
pub fn skip_value(reader: impl Read) -> Result<()> {
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    IgnoredAny::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(())
}

use serde::Deserialize;
