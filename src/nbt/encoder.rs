use super::Encoding;
use crate::{Error, Result};
use serde::ser::{self, Impossible, Serialize};
use std::io::Write;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Value {
    Bool(bool),
    Byte(u8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<u8>),
    String(String),
    RawString(Vec<u8>),
    List(u8, Vec<Value>),
    Compound(Vec<(String, Value)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Value {
    pub(super) fn tag(&self) -> u8 {
        match self {
            Self::Bool(_) | Self::Byte(_) => 1,
            Self::Short(_) => 2,
            Self::Int(_) => 3,
            Self::Long(_) => 4,
            Self::Float(_) => 5,
            Self::Double(_) => 6,
            Self::ByteArray(_) => 7,
            Self::String(_) => 8,
            Self::RawString(_) => 8,
            Self::List(_, _) => 9,
            Self::Compound(_) => 10,
            Self::IntArray(_) => 11,
            Self::LongArray(_) => 12,
        }
    }
}

#[derive(Debug)]
struct SerError {
    message: String,
    incompatible_type: Option<String>,
    value_name: Option<String>,
    partial: Option<PartialValue>,
    depth_leak: usize,
}

#[derive(Debug)]
pub(super) enum PartialValue {
    Complete(Value),
    Compound(Vec<(String, PartialValue)>),
    List {
        tag: u8,
        len: usize,
        values: Vec<PartialValue>,
        prefix_written: bool,
    },
}

impl PartialValue {
    pub(super) fn tag(&self) -> u8 {
        match self {
            Self::Complete(value) => value.tag(),
            Self::Compound(_) => 10,
            Self::List { .. } => 9,
        }
    }
}

impl SerError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            incompatible_type: None,
            value_name: None,
            partial: None,
            depth_leak: 0,
        }
    }

    fn incompatible(type_name: impl Into<String>) -> Self {
        let mut error = Self::new("");
        error.incompatible_type = Some(type_name.into());
        error
    }

    fn display_message(&self) -> String {
        match &self.incompatible_type {
            Some(type_name) => format!(
                "nbt: 值类型 {type_name}（{}）无法转换为 NBT 标签",
                self.value_name.as_deref().unwrap_or("")
            ),
            None => self.message.clone(),
        }
    }
}

impl std::fmt::Display for SerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display_message())
    }
}

impl std::error::Error for SerError {}

impl ser::Error for SerError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        Self::new(msg.to_string())
    }
}

type SerResult<T> = std::result::Result<T, SerError>;

fn go_type_name<T: ?Sized>() -> String {
    let rust_name = rust_type_name(std::any::type_name::<T>());
    match rust_name {
        "bool" => "bool".into(),
        "u8" => "uint8".into(),
        "i8" => "int8".into(),
        "i16" => "int16".into(),
        "i32" => "int32".into(),
        "i64" => "int64".into(),
        "i128" => "int128".into(),
        "u16" => "uint16".into(),
        "u32" => "uint32".into(),
        "u64" => "uint64".into(),
        "u128" => "uint128".into(),
        "f32" => "float32".into(),
        "f64" => "float64".into(),
        "alloc::string::String" => "string".into(),
        other => other.to_string(),
    }
}

fn rust_type_name(mut name: &str) -> &str {
    while let Some(rest) = name.strip_prefix('&') {
        name = rest.trim_start();
        if let Some(rest) = name.strip_prefix("mut ") {
            name = rest;
        }
    }
    name
}

fn generic_inner<'a>(name: &'a str, path: &str) -> Option<&'a str> {
    let inner = name.strip_prefix(path)?.strip_prefix('<')?;
    inner.strip_suffix('>')
}

fn static_tag(name: &str) -> Option<u8> {
    let name = rust_type_name(name);
    match name {
        "bool" | "u8" => Some(1),
        "i16" => Some(2),
        "i32" => Some(3),
        "i64" => Some(4),
        "f32" => Some(5),
        "f64" => Some(6),
        "alloc::string::String" | "std::string::String" | "&str" => Some(8),
        _ if name.starts_with("[") => static_array(name).0,
        _ if name.starts_with("alloc::vec::Vec<") || name.starts_with("std::vec::Vec<") => Some(9),
        _ if name.starts_with("std::collections::") => Some(10),
        _ => None,
    }
}

fn static_list_tag(name: &str) -> Option<u8> {
    let name = rust_type_name(name);
    let element =
        generic_inner(name, "alloc::vec::Vec").or_else(|| generic_inner(name, "std::vec::Vec"))?;
    static_tag(element)
}

fn static_array(name: &str) -> (Option<u8>, Option<String>) {
    let name = rust_type_name(name);
    let Some(inner) = name
        .strip_prefix('[')
        .and_then(|name| name.strip_suffix(']'))
    else {
        return (None, None);
    };
    let Some((element, length)) = inner.rsplit_once(';') else {
        return (None, None);
    };
    let Ok(length) = length.trim().parse::<usize>() else {
        return (None, None);
    };
    let tag = match rust_type_name(element.trim()) {
        "u8" => Some(7),
        "i32" => Some(11),
        "i64" => Some(12),
        _ => None,
    };
    let type_name = if let Some(tag) = tag {
        let element_name = match tag {
            7 => "uint8",
            11 => "int32",
            _ => "int64",
        };
        Some(format!("[{length}]{element_name}"))
    } else {
        Some(format!("[{length}]{}", go_type_name_text(element.trim())))
    };
    (tag, type_name)
}

fn go_type_name_text(name: &str) -> &str {
    match rust_type_name(name) {
        "bool" => "bool",
        "u8" => "uint8",
        "i8" => "int8",
        "i16" => "int16",
        "i32" => "int32",
        "i64" => "int64",
        "u16" => "uint16",
        "u32" => "uint32",
        "u64" => "uint64",
        "f32" => "float32",
        "f64" => "float64",
        value => value,
    }
}

struct Serializer {
    expected_list_tag: Option<u8>,
    expected_array_tag: Option<u8>,
    expected_array_type: Option<String>,
}

impl Serializer {
    fn encode<T: ?Sized + Serialize>(value: &T) -> SerResult<Value> {
        let type_name = std::any::type_name::<T>();
        let (expected_array_tag, expected_array_type) = static_array(type_name);
        value.serialize(Self {
            expected_list_tag: static_list_tag(type_name),
            expected_array_tag,
            expected_array_type,
        })
    }
}

struct Sequence {
    values: Vec<Value>,
    tuple: bool,
    expected_tag: Option<u8>,
    declared_len: Option<usize>,
    expected_array_tag: Option<u8>,
    expected_array_type: Option<String>,
}

impl Sequence {
    fn serialize_one<T: ?Sized + Serialize>(&mut self, value: &T) -> SerResult<()> {
        match Serializer::encode(value) {
            Ok(value) => {
                self.values.push(value);
                Ok(())
            }
            Err(mut error) => {
                if self.tuple {
                    if let Some(type_name) = error.incompatible_type.take() {
                        error.incompatible_type =
                            Some(format!("[{}]{type_name}", self.declared_len.unwrap_or(0)));
                    }
                    return Err(error);
                }
                if !self.tuple {
                    error.depth_leak += 1;
                }
                if let Some(type_name) = error.incompatible_type.take() {
                    error.incompatible_type = Some(if self.tuple {
                        format!("[{}]{type_name}", self.declared_len.unwrap_or(0))
                    } else {
                        format!("[]{type_name}")
                    });
                }
                let prefix_written = !self.values.is_empty() || error.partial.is_some();
                let tag = self
                    .expected_tag
                    .or_else(|| self.values.first().map(Value::tag))
                    .or_else(|| error.partial.as_ref().map(PartialValue::tag))
                    .unwrap_or(0);
                let mut values = std::mem::take(&mut self.values)
                    .into_iter()
                    .map(PartialValue::Complete)
                    .collect::<Vec<_>>();
                if let Some(partial) = error.partial.take() {
                    values.push(partial);
                }
                error.partial = Some(PartialValue::List {
                    tag,
                    len: self.declared_len.unwrap_or(values.len()),
                    values,
                    prefix_written,
                });
                Err(error)
            }
        }
    }

    fn finish(self) -> SerResult<Value> {
        if self.tuple {
            if self.values.is_empty() {
                return match self.expected_array_tag {
                    Some(7) => Ok(Value::ByteArray(Vec::new())),
                    Some(11) => Ok(Value::IntArray(Vec::new())),
                    Some(12) => Ok(Value::LongArray(Vec::new())),
                    _ => Err(SerError::incompatible(
                        self.expected_array_type
                            .unwrap_or_else(|| "[0]struct".to_string()),
                    )),
                };
            }
            let first = self.values.first().map(Value::tag);
            if let Some(tag) = first {
                if self.values.iter().all(|value| value.tag() == tag) {
                    match tag {
                        1 => {
                            if !self
                                .values
                                .iter()
                                .all(|value| matches!(value, Value::Byte(_)))
                            {
                                return Err(SerError::new("unsupported Go NBT array element type"));
                            }
                            return Ok(Value::ByteArray(
                                self.values
                                    .into_iter()
                                    .map(|value| match value {
                                        Value::Byte(byte) => byte,
                                        _ => unreachable!(),
                                    })
                                    .collect(),
                            ));
                        }
                        3 => {
                            return Ok(Value::IntArray(
                                self.values
                                    .into_iter()
                                    .map(|value| match value {
                                        Value::Int(number) => number,
                                        _ => unreachable!(),
                                    })
                                    .collect(),
                            ));
                        }
                        4 => {
                            return Ok(Value::LongArray(
                                self.values
                                    .into_iter()
                                    .map(|value| match value {
                                        Value::Long(number) => number,
                                        _ => unreachable!(),
                                    })
                                    .collect(),
                            ));
                        }
                        _ => {}
                    }
                }
                let element_type = match self.values.first() {
                    Some(Value::Bool(_)) => "bool",
                    Some(Value::Short(_)) => "int16",
                    Some(Value::Float(_)) => "float32",
                    Some(Value::Double(_)) => "float64",
                    Some(Value::String(_)) | Some(Value::RawString(_)) => "string",
                    Some(Value::List(_, _)) => "slice",
                    Some(Value::Compound(_)) => "struct",
                    Some(Value::Byte(_)) => "uint8",
                    Some(Value::Int(_)) => "int32",
                    Some(Value::Long(_)) => "int64",
                    Some(Value::ByteArray(_)) => "[ ]uint8",
                    Some(Value::IntArray(_)) => "[ ]int32",
                    Some(Value::LongArray(_)) => "[ ]int64",
                    None => unreachable!(),
                };
                return Err(SerError::incompatible(format!(
                    "[{}]{element_type}",
                    self.declared_len.unwrap_or(0)
                )));
            }
        }
        let tag = self
            .expected_tag
            .or_else(|| self.values.first().map(Value::tag))
            .unwrap_or(0);
        if tag == 0 && !self.values.is_empty() {
            return Err(SerError::new("NBT List cannot contain TAG_End values"));
        }
        if tag > 12 || self.values.iter().any(|value| value.tag() != tag) {
            return Err(SerError::new("NBT List 包含不同类型"));
        }
        Ok(Value::List(tag, self.values))
    }
}

impl ser::SerializeSeq for Sequence {
    type Ok = Value;
    type Error = SerError;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> SerResult<()> {
        self.serialize_one(value)
    }
    fn end(self) -> SerResult<Value> {
        self.finish()
    }
}

impl ser::SerializeTuple for Sequence {
    type Ok = Value;
    type Error = SerError;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> SerResult<()> {
        self.serialize_one(value)
    }
    fn end(self) -> SerResult<Value> {
        self.finish()
    }
}

impl ser::SerializeTupleStruct for Sequence {
    type Ok = Value;
    type Error = SerError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> SerResult<()> {
        self.serialize_one(value)
    }
    fn end(self) -> SerResult<Value> {
        self.finish()
    }
}

struct MapBuilder {
    fields: Vec<(String, Value)>,
    key: Option<String>,
    map_type: Option<String>,
}

impl ser::SerializeMap for MapBuilder {
    type Ok = Value;
    type Error = SerError;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> SerResult<()> {
        match Serializer::encode(key)? {
            Value::String(key) => {
                self.key = Some(key);
                Ok(())
            }
            _ => {
                let mut error =
                    SerError::incompatible(self.map_type.clone().unwrap_or_else(|| "map".into()));
                error.depth_leak += 1;
                error.partial = Some(PartialValue::Compound(
                    self.fields
                        .drain(..)
                        .map(|(name, value)| (name, PartialValue::Complete(value)))
                        .collect(),
                ));
                Err(error)
            }
        }
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> SerResult<()> {
        let key = self
            .key
            .take()
            .ok_or_else(|| SerError::new("map value without key"))?;
        match Serializer::encode(value) {
            Ok(value) => {
                self.fields.push((key, value));
                Ok(())
            }
            Err(mut error) => {
                error.depth_leak += 1;
                if error.value_name.is_none() {
                    error.value_name = Some(key.clone());
                }
                let mut fields = std::mem::take(&mut self.fields)
                    .into_iter()
                    .map(|(name, value)| (name, PartialValue::Complete(value)))
                    .collect::<Vec<_>>();
                if let Some(partial) = error.partial.take() {
                    fields.push((key, partial));
                }
                error.partial = Some(PartialValue::Compound(fields));
                Err(error)
            }
        }
    }
    fn end(self) -> SerResult<Value> {
        Ok(Value::Compound(self.fields))
    }
    fn serialize_entry<K: ?Sized + Serialize, V: ?Sized + Serialize>(
        &mut self,
        key: &K,
        value: &V,
    ) -> SerResult<()> {
        self.map_type = Some(format!(
            "map[{}]{}",
            go_type_name::<K>(),
            go_type_name::<V>()
        ));
        self.serialize_key(key)?;
        self.serialize_value(value)
    }
}

impl ser::SerializeStruct for MapBuilder {
    type Ok = Value;
    type Error = SerError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> SerResult<()> {
        match Serializer::encode(value) {
            Ok(value) => {
                self.fields.push((key.to_string(), value));
                Ok(())
            }
            Err(mut error) => {
                error.depth_leak += 1;
                if error.value_name.is_none() {
                    error.value_name = Some(key.to_string());
                }
                let mut fields = std::mem::take(&mut self.fields)
                    .into_iter()
                    .map(|(name, value)| (name, PartialValue::Complete(value)))
                    .collect::<Vec<_>>();
                if let Some(partial) = error.partial.take() {
                    fields.push((key.to_string(), partial));
                }
                error.partial = Some(PartialValue::Compound(fields));
                Err(error)
            }
        }
    }
    fn end(self) -> SerResult<Value> {
        Ok(Value::Compound(self.fields))
    }
}

impl ser::Serializer for Serializer {
    type Ok = Value;
    type Error = SerError;
    type SerializeSeq = Sequence;
    type SerializeTuple = Sequence;
    type SerializeTupleStruct = Sequence;
    type SerializeTupleVariant = Impossible<Value, SerError>;
    type SerializeMap = MapBuilder;
    type SerializeStruct = MapBuilder;
    type SerializeStructVariant = Impossible<Value, SerError>;

    fn serialize_bool(self, value: bool) -> SerResult<Value> {
        Ok(Value::Bool(value))
    }
    fn serialize_i8(self, _: i8) -> SerResult<Value> {
        Err(SerError::incompatible("int8"))
    }
    fn serialize_i16(self, value: i16) -> SerResult<Value> {
        Ok(Value::Short(value))
    }
    fn serialize_i32(self, value: i32) -> SerResult<Value> {
        Ok(Value::Int(value))
    }
    fn serialize_i64(self, value: i64) -> SerResult<Value> {
        Ok(Value::Long(value))
    }
    fn serialize_i128(self, _: i128) -> SerResult<Value> {
        Err(SerError::new("unsupported Go NBT kind: int128"))
    }
    fn serialize_u8(self, value: u8) -> SerResult<Value> {
        Ok(Value::Byte(value))
    }
    fn serialize_u16(self, _: u16) -> SerResult<Value> {
        Err(SerError::incompatible("uint16"))
    }
    fn serialize_u32(self, _: u32) -> SerResult<Value> {
        Err(SerError::incompatible("uint32"))
    }
    fn serialize_u64(self, _: u64) -> SerResult<Value> {
        Err(SerError::incompatible("uint64"))
    }
    fn serialize_u128(self, _: u128) -> SerResult<Value> {
        Err(SerError::new("unsupported Go NBT kind: uint128"))
    }
    fn serialize_f32(self, value: f32) -> SerResult<Value> {
        Ok(Value::Float(value))
    }
    fn serialize_f64(self, value: f64) -> SerResult<Value> {
        Ok(Value::Double(value))
    }
    fn serialize_char(self, value: char) -> SerResult<Value> {
        Ok(Value::Int(value as u32 as i32))
    }
    fn serialize_str(self, value: &str) -> SerResult<Value> {
        Ok(Value::String(value.to_string()))
    }
    fn serialize_bytes(self, value: &[u8]) -> SerResult<Value> {
        Ok(Value::List(
            1,
            value.iter().copied().map(Value::Byte).collect(),
        ))
    }
    fn serialize_none(self) -> SerResult<Value> {
        Err(SerError::new("nil Go pointer cannot be encoded"))
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> SerResult<Value> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> SerResult<Value> {
        Err(SerError::new("unsupported Go NBT kind: unit"))
    }
    fn serialize_unit_struct(self, _: &'static str) -> SerResult<Value> {
        Ok(Value::Compound(Vec::new()))
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> SerResult<Value> {
        Err(SerError::new("unsupported Go NBT enum"))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        name: &'static str,
        value: &T,
    ) -> SerResult<Value> {
        if name == "NbtRawString" {
            let value = value.serialize(self)?;
            let Value::List(1, values) = value else {
                return Err(SerError::new("invalid raw NBT string"));
            };
            return values
                .into_iter()
                .map(|value| match value {
                    Value::Byte(value) => Ok(value),
                    _ => Err(SerError::new("invalid raw NBT string byte")),
                })
                .collect::<SerResult<Vec<_>>>()
                .map(Value::RawString);
        }
        if let Some(tag) = name
            .strip_prefix("NbtTypedList_")
            .and_then(|tag| tag.parse::<u8>().ok())
        {
            return value.serialize(Self {
                expected_list_tag: Some(tag),
                expected_array_tag: None,
                expected_array_type: None,
            });
        }
        if let Some(tag) = name
            .strip_prefix("NbtTypedArray_")
            .and_then(|tag| tag.parse::<u8>().ok())
        {
            let value = value.serialize(self)?;
            let (actual_tag, values) = match value {
                Value::List(actual_tag, values) => (actual_tag, values),
                Value::ByteArray(values) => (1, values.into_iter().map(Value::Byte).collect()),
                Value::IntArray(values) => (3, values.into_iter().map(Value::Int).collect()),
                Value::LongArray(values) => (4, values.into_iter().map(Value::Long).collect()),
                _ => return Err(SerError::new("invalid typed NBT array")),
            };
            let expected_element_tag = match tag {
                7 => 1,
                11 => 3,
                12 => 4,
                _ => return Err(SerError::new("typed NBT array tag must be 7, 11, or 12")),
            };
            if !values.is_empty() && actual_tag != expected_element_tag {
                return Err(SerError::new("typed NBT array element type mismatch"));
            }
            return match tag {
                7 => values
                    .into_iter()
                    .map(|value| match value {
                        Value::Byte(byte) => Ok(byte),
                        _ => Err(SerError::new("typed NBT byte array element type mismatch")),
                    })
                    .collect::<SerResult<Vec<_>>>()
                    .map(Value::ByteArray),
                11 => values
                    .into_iter()
                    .map(|value| match value {
                        Value::Int(number) => Ok(number),
                        _ => Err(SerError::new("typed NBT int array element type mismatch")),
                    })
                    .collect::<SerResult<Vec<_>>>()
                    .map(Value::IntArray),
                12 => values
                    .into_iter()
                    .map(|value| match value {
                        Value::Long(number) => Ok(number),
                        _ => Err(SerError::new("typed NBT long array element type mismatch")),
                    })
                    .collect::<SerResult<Vec<_>>>()
                    .map(Value::LongArray),
                _ => Err(SerError::new("typed NBT array tag must be 7, 11, or 12")),
            };
        }
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> SerResult<Value> {
        Err(SerError::new("unsupported Go NBT enum"))
    }
    fn serialize_seq(self, len: Option<usize>) -> SerResult<Sequence> {
        Ok(Sequence {
            values: Vec::with_capacity(len.unwrap_or(0)),
            tuple: false,
            expected_tag: self.expected_list_tag,
            declared_len: len,
            expected_array_tag: self.expected_array_tag,
            expected_array_type: self.expected_array_type,
        })
    }
    fn serialize_tuple(self, len: usize) -> SerResult<Sequence> {
        Ok(Sequence {
            values: Vec::with_capacity(len),
            tuple: true,
            expected_tag: None,
            declared_len: Some(len),
            expected_array_tag: self.expected_array_tag,
            expected_array_type: self.expected_array_type,
        })
    }
    fn serialize_tuple_struct(self, _: &'static str, len: usize) -> SerResult<Sequence> {
        self.serialize_tuple(len)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> SerResult<Self::SerializeTupleVariant> {
        Err(SerError::new("unsupported Go NBT enum"))
    }
    fn serialize_map(self, len: Option<usize>) -> SerResult<MapBuilder> {
        Ok(MapBuilder {
            fields: Vec::with_capacity(len.unwrap_or(0)),
            key: None,
            map_type: None,
        })
    }
    fn serialize_struct(self, _: &'static str, len: usize) -> SerResult<MapBuilder> {
        Ok(MapBuilder {
            fields: Vec::with_capacity(len),
            key: None,
            map_type: None,
        })
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> SerResult<Self::SerializeStructVariant> {
        Err(SerError::new("unsupported Go NBT enum"))
    }
}

struct EncoderOutput<'a, W> {
    writer: super::OffsetWriter<super::IoWriteSink<&'a mut W>>,
    encoding: Encoding,
    depth: usize,
}

impl<'a, W: Write> EncoderOutput<'a, W> {
    fn new(writer: &'a mut W, encoding: Encoding, offset: u64, depth: usize) -> Self {
        Self {
            writer: super::OffsetWriter::with_offset(super::IoWriteSink::new(writer), offset),
            encoding,
            depth,
        }
    }

    fn offset(&self) -> u64 {
        self.writer.get_offset()
    }

    fn depth(&self) -> usize {
        self.depth
    }

    fn write_byte(&mut self, value: u8) -> Result<()> {
        let (written, error) = self.writer.write(&[value]);
        if let Some(error) = error {
            return Err(Error::Io(error));
        }
        if written != 1 {
            return Err(Error::Io(std::io::Error::other("short write")));
        }
        Ok(())
    }

    fn write_fixed(&mut self, bytes: &[u8], op: &str) -> Result<()> {
        let (_, error) = self.writer.write(bytes);
        if let Some(error) = error {
            return Err(self.failed(op, error));
        }
        Ok(())
    }

    fn write_network_varint(&mut self, mut value: u64) -> Result<()> {
        while value >= 0x80 {
            self.write_byte(value as u8 | 0x80)?;
            value >>= 7;
        }
        self.write_byte(value as u8)
    }

    fn write_i16(&mut self, value: i16) -> Result<()> {
        let bytes = if self.encoding.le() {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        };
        self.write_fixed(&bytes, "WriteInt16")
    }

    fn write_i32(&mut self, value: i32) -> Result<()> {
        if self.encoding.net() {
            let zigzag = ((value as u32) << 1) ^ ((value >> 31) as u32);
            let mut encoded = zigzag as u64;
            while encoded >= 0x80 {
                if let Err(error) = self.write_byte(encoded as u8 | 0x80) {
                    return Err(self.failed("WriteInt32", error));
                }
                encoded >>= 7;
            }
            return self
                .write_byte(encoded as u8)
                .map_err(|error| self.failed("WriteInt32", error));
        }
        let bytes = if self.encoding.le() {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        };
        self.write_fixed(&bytes, "WriteInt32")
    }

    fn write_i64(&mut self, value: i64) -> Result<()> {
        if self.encoding.net() {
            let zigzag = ((value as u64) << 1) ^ ((value >> 63) as u64);
            let mut encoded = zigzag;
            while encoded >= 0x80 {
                if let Err(error) = self.write_byte(encoded as u8 | 0x80) {
                    return Err(self.failed("WriteInt64", error));
                }
                encoded >>= 7;
            }
            return self
                .write_byte(encoded as u8)
                .map_err(|error| self.failed("WriteInt64", error));
        }
        let bytes = if self.encoding.le() {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        };
        self.write_fixed(&bytes, "WriteInt64")
    }

    fn write_string(&mut self, value: &str) -> Result<()> {
        self.write_string_bytes(value.as_bytes())
    }

    fn write_string_bytes(&mut self, value: &[u8]) -> Result<()> {
        let offset = self.offset();
        if value.len() > i16::MAX as usize {
            return Err(Error::Other(format!(
                "nbt: 偏移 {offset} 的字符串无效: 字符串长度超过最大限制（长度={}）",
                value.len()
            )));
        }
        if self.encoding.net() {
            self.write_network_varint(value.len() as u64)
                .map_err(|error| self.failed("WriteString", error))?;
        } else {
            self.write_i16(value.len() as i16)
                .map_err(|error| self.failed("WriteString", error))?;
        }
        if !value.is_empty() {
            let (_, error) = self.writer.write(value);
            if let Some(error) = error {
                return Err(self.failed("WriteString", error));
            }
        }
        Ok(())
    }

    fn write_tag(&mut self, value: &Value, name: &str) -> Result<()> {
        if self.depth >= 512 {
            return Err(Error::Other("nbt: 达到最大嵌套深度 512".into()));
        }
        self.write_byte(value.tag())?;
        if !(self.encoding == Encoding::NetworkBigEndian && value.tag() == 10 && self.depth == 0) {
            self.write_string(name)?;
        }
        Ok(())
    }

    fn write_root(&mut self, value: &Value) -> Result<()> {
        self.write_tag(value, "")?;
        self.write_payload(value)
    }

    fn write_partial_root(&mut self, value: &PartialValue) -> Result<()> {
        let tag = value.tag();
        if self.depth >= 512 {
            return Err(Error::Other("nbt: 达到最大嵌套深度 512".into()));
        }
        self.write_byte(tag)?;
        if !(self.encoding == Encoding::NetworkBigEndian && tag == 10 && self.depth == 0) {
            self.write_string("")?;
        }
        self.write_partial_payload(value)
    }

    fn write_payload(&mut self, value: &Value) -> Result<()> {
        match value {
            Value::Bool(value) => self.write_byte(u8::from(*value)),
            Value::Byte(value) => self.write_byte(*value),
            Value::Short(value) => self.write_i16(*value),
            Value::Int(value) => self.write_i32(*value),
            Value::Long(value) => self.write_i64(*value),
            Value::Float(value) => {
                let bytes = if self.encoding.le() {
                    value.to_bits().to_le_bytes()
                } else {
                    value.to_bits().to_be_bytes()
                };
                self.write_fixed(&bytes, "WriteFloat32")
            }
            Value::Double(value) => {
                let bytes = if self.encoding.le() {
                    value.to_bits().to_le_bytes()
                } else {
                    value.to_bits().to_be_bytes()
                };
                self.write_fixed(&bytes, "WriteFloat64")
            }
            Value::String(value) => self.write_string(value),
            Value::RawString(value) => self.write_string_bytes(value),
            Value::ByteArray(values) => {
                self.write_i32(values.len() as i32)?;
                let (_, error) = self.writer.write(values);
                match error {
                    Some(error) => Err(self.failed("WriteByteArray", error)),
                    None => Ok(()),
                }
            }
            Value::IntArray(values) => {
                self.write_i32(values.len() as i32)?;
                for value in values {
                    self.write_i32(*value)?;
                }
                Ok(())
            }
            Value::LongArray(values) => {
                self.write_i32(values.len() as i32)?;
                for value in values {
                    self.write_i64(*value)?;
                }
                Ok(())
            }
            Value::List(tag, values) => {
                self.depth += 1;
                let result = (|| {
                    if let Err(error) = self.write_byte(*tag) {
                        return Err(self.failed("WriteSlice", error));
                    }
                    self.write_i32(values.len() as i32)?;
                    for value in values {
                        self.write_payload(value)?;
                    }
                    Ok(())
                })();
                if result.is_ok() {
                    self.depth -= 1;
                }
                result
            }
            Value::Compound(fields) => {
                self.depth += 1;
                for (name, value) in fields {
                    self.write_tag(value, name)?;
                    self.write_payload(value)?;
                }
                self.depth -= 1;
                self.write_byte(0)
            }
        }
    }

    fn write_partial_payload(&mut self, value: &PartialValue) -> Result<()> {
        match value {
            PartialValue::Complete(value) => self.write_payload(value),
            PartialValue::Compound(fields) => {
                self.depth += 1;
                for (name, value) in fields {
                    let tag = value.tag();
                    if self.depth >= 512 {
                        return Err(Error::Other("nbt: 达到最大嵌套深度 512".into()));
                    }
                    self.write_byte(tag)?;
                    self.write_string(name)?;
                    self.write_partial_payload(value)?;
                }
                Ok(())
            }
            PartialValue::List {
                tag,
                len,
                values,
                prefix_written,
            } => {
                if !prefix_written {
                    return Ok(());
                }
                self.depth += 1;
                if let Err(error) = self.write_byte(*tag) {
                    return Err(self.failed("WriteSlice", error));
                }
                self.write_i32(*len as i32)?;
                for value in values {
                    self.write_partial_payload(value)?;
                }
                Ok(())
            }
        }
    }

    fn failed(&self, op: &str, error: impl std::fmt::Display) -> Error {
        Error::Other(format!(
            "nbt: 在偏移 {} 的操作 '{op}' 写入失败: {error}",
            self.offset()
        ))
    }
}

/// Marks an empty or dynamically typed Rust sequence with its Go-compatible NBT element tag.
pub struct TypedList<T> {
    pub element_tag: u8,
    pub values: Vec<T>,
}

pub struct TypedArray<T> {
    pub element_tag: u8,
    pub values: Vec<T>,
}

pub struct RawString(pub Vec<u8>);

struct RawBytes<'a>(&'a [u8]);

impl Serialize for RawBytes<'_> {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_bytes(self.0)
    }
}

impl Serialize for RawString {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_newtype_struct("NbtRawString", &RawBytes(&self.0))
    }
}

impl<T> TypedArray<T> {
    pub fn new(element_tag: u8, values: Vec<T>) -> Self {
        Self {
            element_tag,
            values,
        }
    }
}

impl<T: Serialize> Serialize for TypedArray<T> {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let name = match self.element_tag {
            7 => "NbtTypedArray_7",
            11 => "NbtTypedArray_11",
            12 => "NbtTypedArray_12",
            _ => return Err(ser::Error::custom("typed array tag must be 7, 11, or 12")),
        };
        serializer.serialize_newtype_struct(name, &self.values)
    }
}

pub fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

impl<T> TypedList<T> {
    pub fn new(element_tag: u8, values: Vec<T>) -> Self {
        Self {
            element_tag,
            values,
        }
    }
}

impl<T: Serialize> Serialize for TypedList<T> {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let name = match self.element_tag {
            0 => "NbtTypedList_0",
            1 => "NbtTypedList_1",
            2 => "NbtTypedList_2",
            3 => "NbtTypedList_3",
            4 => "NbtTypedList_4",
            5 => "NbtTypedList_5",
            6 => "NbtTypedList_6",
            7 => "NbtTypedList_7",
            8 => "NbtTypedList_8",
            9 => "NbtTypedList_9",
            10 => "NbtTypedList_10",
            11 => "NbtTypedList_11",
            12 => "NbtTypedList_12",
            _ => return Err(ser::Error::custom("invalid NBT list element tag")),
        };
        serializer.serialize_newtype_struct(name, &self.values)
    }
}

pub struct Encoder<W> {
    writer: W,
    encoding: Encoding,
    offset: u64,
    depth: usize,
}

impl<W: Write> Encoder<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            encoding: Encoding::NetworkLittleEndian,
            offset: 0,
            depth: 0,
        }
    }
    pub fn with_encoding(writer: W, encoding: Encoding) -> Self {
        Self {
            writer,
            encoding,
            offset: 0,
            depth: 0,
        }
    }
    pub fn encode<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        let base_depth = self.depth;
        let mut encoder =
            EncoderOutput::new(&mut self.writer, self.encoding, self.offset, self.depth);
        let result = match Serializer::encode(value) {
            Ok(encoded) => encoder.write_root(&encoded),
            Err(error) => {
                let partial_result = match error.partial.as_ref() {
                    Some(partial) => encoder.write_partial_root(partial),
                    None => Ok(()),
                };
                if partial_result.is_ok() {
                    self.depth = base_depth + error.depth_leak;
                } else {
                    self.depth = encoder.depth();
                }
                partial_result.and_then(|()| Err(Error::invalid(error.display_message())))
            }
        };
        self.offset = encoder.offset();
        if result.is_ok() {
            self.depth = encoder.depth();
        }
        result
    }
    pub fn into_inner(self) -> W {
        self.writer
    }
}
