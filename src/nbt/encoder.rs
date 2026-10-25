use super::{Encoding, Writer};
use crate::{Error, Result};
use serde::ser::{self, Impossible, Serialize};
use std::io::Write;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Value {
    Byte(u8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<u8>),
    String(String),
    List(u8, Vec<Value>),
    Compound(Vec<(String, Value)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Value {
    pub(super) fn tag(&self) -> u8 {
        match self {
            Self::Byte(_) => 1,
            Self::Short(_) => 2,
            Self::Int(_) => 3,
            Self::Long(_) => 4,
            Self::Float(_) => 5,
            Self::Double(_) => 6,
            Self::ByteArray(_) => 7,
            Self::String(_) => 8,
            Self::List(_, _) => 9,
            Self::Compound(_) => 10,
            Self::IntArray(_) => 11,
            Self::LongArray(_) => 12,
        }
    }
}

#[derive(Debug)]
struct SerError(String);

impl std::fmt::Display for SerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SerError {}

impl ser::Error for SerError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        Self(msg.to_string())
    }
}

type SerResult<T> = std::result::Result<T, SerError>;

#[derive(Clone, Copy)]
struct Serializer {
    expected_list_tag: Option<u8>,
}

impl Serializer {
    fn encode<T: ?Sized + Serialize>(value: &T) -> SerResult<Value> {
        value.serialize(Self {
            expected_list_tag: None,
        })
    }
}

struct Sequence {
    values: Vec<Value>,
    tuple: bool,
    expected_tag: Option<u8>,
}

impl Sequence {
    fn finish(self) -> SerResult<Value> {
        if self.tuple {
            let first = self.values.first().map(Value::tag);
            if let Some(tag) = first {
                if self.values.iter().all(|value| value.tag() == tag) {
                    match tag {
                        1 => {
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
                return Err(SerError("unsupported Go NBT array element type".into()));
            }
        }
        let tag = self
            .expected_tag
            .or_else(|| self.values.first().map(Value::tag))
            .unwrap_or(0);
        if tag == 0 && !self.values.is_empty() {
            return Err(SerError("NBT List cannot contain TAG_End values".into()));
        }
        if tag > 12 || self.values.iter().any(|value| value.tag() != tag) {
            return Err(SerError("NBT List 包含不同类型".into()));
        }
        Ok(Value::List(tag, self.values))
    }
}

impl ser::SerializeSeq for Sequence {
    type Ok = Value;
    type Error = SerError;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> SerResult<()> {
        self.values.push(Serializer::encode(value)?);
        Ok(())
    }
    fn end(self) -> SerResult<Value> {
        self.finish()
    }
}

impl ser::SerializeTuple for Sequence {
    type Ok = Value;
    type Error = SerError;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> SerResult<()> {
        self.values.push(Serializer::encode(value)?);
        Ok(())
    }
    fn end(self) -> SerResult<Value> {
        self.finish()
    }
}

impl ser::SerializeTupleStruct for Sequence {
    type Ok = Value;
    type Error = SerError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> SerResult<()> {
        self.values.push(Serializer::encode(value)?);
        Ok(())
    }
    fn end(self) -> SerResult<Value> {
        self.finish()
    }
}

struct MapBuilder {
    fields: Vec<(String, Value)>,
    key: Option<String>,
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
            _ => Err(SerError("NBT map key must be a string".into())),
        }
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> SerResult<()> {
        let key = self
            .key
            .take()
            .ok_or_else(|| SerError("map value without key".into()))?;
        self.fields.push((key, Serializer::encode(value)?));
        Ok(())
    }
    fn end(self) -> SerResult<Value> {
        Ok(Value::Compound(self.fields))
    }
    fn serialize_entry<K: ?Sized + Serialize, V: ?Sized + Serialize>(
        &mut self,
        key: &K,
        value: &V,
    ) -> SerResult<()> {
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
        self.fields
            .push((key.to_string(), Serializer::encode(value)?));
        Ok(())
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
        Ok(Value::Byte(u8::from(value)))
    }
    fn serialize_i8(self, _: i8) -> SerResult<Value> {
        Err(SerError("unsupported Go NBT kind: int8".into()))
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
        Err(SerError("unsupported Go NBT kind: int128".into()))
    }
    fn serialize_u8(self, value: u8) -> SerResult<Value> {
        Ok(Value::Byte(value))
    }
    fn serialize_u16(self, _: u16) -> SerResult<Value> {
        Err(SerError("unsupported Go NBT kind: uint16".into()))
    }
    fn serialize_u32(self, _: u32) -> SerResult<Value> {
        Err(SerError("unsupported Go NBT kind: uint32".into()))
    }
    fn serialize_u64(self, _: u64) -> SerResult<Value> {
        Err(SerError("unsupported Go NBT kind: uint64".into()))
    }
    fn serialize_u128(self, _: u128) -> SerResult<Value> {
        Err(SerError("unsupported Go NBT kind: uint128".into()))
    }
    fn serialize_f32(self, value: f32) -> SerResult<Value> {
        Ok(Value::Float(value))
    }
    fn serialize_f64(self, value: f64) -> SerResult<Value> {
        Ok(Value::Double(value))
    }
    fn serialize_char(self, value: char) -> SerResult<Value> {
        Err(SerError(format!(
            "unsupported Go NBT kind: Rust char ({value:?})"
        )))
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
        Err(SerError("nil Go pointer cannot be encoded".into()))
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> SerResult<Value> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> SerResult<Value> {
        Err(SerError("unsupported Go NBT kind: unit".into()))
    }
    fn serialize_unit_struct(self, _: &'static str) -> SerResult<Value> {
        Ok(Value::Compound(Vec::new()))
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> SerResult<Value> {
        Err(SerError("unsupported Go NBT enum".into()))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        name: &'static str,
        value: &T,
    ) -> SerResult<Value> {
        if let Some(tag) = name
            .strip_prefix("NbtTypedList_")
            .and_then(|tag| tag.parse::<u8>().ok())
        {
            return value.serialize(Self {
                expected_list_tag: Some(tag),
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
                _ => return Err(SerError("invalid typed NBT array".into())),
            };
            let expected_element_tag = match tag {
                7 => 1,
                11 => 3,
                12 => 4,
                _ => return Err(SerError("typed NBT array tag must be 7, 11, or 12".into())),
            };
            if !values.is_empty() && actual_tag != expected_element_tag {
                return Err(SerError("typed NBT array element type mismatch".into()));
            }
            return match tag {
                7 => values
                    .into_iter()
                    .map(|value| match value {
                        Value::Byte(byte) => Ok(byte),
                        _ => Err(SerError(
                            "typed NBT byte array element type mismatch".into(),
                        )),
                    })
                    .collect::<SerResult<Vec<_>>>()
                    .map(Value::ByteArray),
                11 => values
                    .into_iter()
                    .map(|value| match value {
                        Value::Int(number) => Ok(number),
                        _ => Err(SerError("typed NBT int array element type mismatch".into())),
                    })
                    .collect::<SerResult<Vec<_>>>()
                    .map(Value::IntArray),
                12 => values
                    .into_iter()
                    .map(|value| match value {
                        Value::Long(number) => Ok(number),
                        _ => Err(SerError(
                            "typed NBT long array element type mismatch".into(),
                        )),
                    })
                    .collect::<SerResult<Vec<_>>>()
                    .map(Value::LongArray),
                _ => Err(SerError("typed NBT array tag must be 7, 11, or 12".into())),
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
        Err(SerError("unsupported Go NBT enum".into()))
    }
    fn serialize_seq(self, len: Option<usize>) -> SerResult<Sequence> {
        Ok(Sequence {
            values: Vec::with_capacity(len.unwrap_or(0)),
            tuple: false,
            expected_tag: self.expected_list_tag,
        })
    }
    fn serialize_tuple(self, len: usize) -> SerResult<Sequence> {
        Ok(Sequence {
            values: Vec::with_capacity(len),
            tuple: true,
            expected_tag: None,
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
        Err(SerError("unsupported Go NBT enum".into()))
    }
    fn serialize_map(self, len: Option<usize>) -> SerResult<MapBuilder> {
        Ok(MapBuilder {
            fields: Vec::with_capacity(len.unwrap_or(0)),
            key: None,
        })
    }
    fn serialize_struct(self, _: &'static str, len: usize) -> SerResult<MapBuilder> {
        Ok(MapBuilder {
            fields: Vec::with_capacity(len),
            key: None,
        })
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> SerResult<Self::SerializeStructVariant> {
        Err(SerError("unsupported Go NBT enum".into()))
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
}

impl<W: Write> Encoder<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            encoding: Encoding::NetworkLittleEndian,
            offset: 0,
        }
    }
    pub fn with_encoding(writer: W, encoding: Encoding) -> Self {
        Self {
            writer,
            encoding,
            offset: 0,
        }
    }
    pub fn encode<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        let encoded =
            Serializer::encode(value).map_err(|error| Error::invalid(error.to_string()))?;
        let mut writer = super::OffsetWriter::with_offset(
            super::IoWriteSink::new(&mut self.writer),
            self.offset,
        );
        let result = Writer::new(self.encoding).write_encoded_root_to(&mut writer, &encoded);
        self.offset = writer.get_offset();
        result
    }
    pub fn into_inner(self) -> W {
        self.writer
    }
}
