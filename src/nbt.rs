use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
};
mod encoder;
pub use encoder::{Encoder, RawString, TypedArray, TypedList, is_default};
mod decoder;
pub use decoder::{Decoder, NbtDecode, unmarshal, unmarshal_with_encoding};
pub use nbt_decode_derive::NbtDecode;
pub type Compound = BTreeMap<String, Nbt>;
pub type RawCompound = BTreeMap<Vec<u8>, Nbt>;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "type", content = "value")]
pub enum Nbt {
    #[default]
    End,
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    RawString(Vec<u8>),
    List(Vec<Nbt>),
    Compound(Compound),
    RawCompound(RawCompound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}
impl Nbt {
    pub fn tag(&self) -> u8 {
        match self {
            Self::End => 0,
            Self::Byte(_) => 1,
            Self::Short(_) => 2,
            Self::Int(_) => 3,
            Self::Long(_) => 4,
            Self::Float(_) => 5,
            Self::Double(_) => 6,
            Self::ByteArray(_) => 7,
            Self::String(_) => 8,
            Self::RawString(_) => 8,
            Self::List(_) => 9,
            Self::Compound(_) => 10,
            Self::RawCompound(_) => 10,
            Self::IntArray(_) => 11,
            Self::LongArray(_) => 12,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Byte(v) => Some(*v as i64),
            Self::Short(v) => Some(*v as i64),
            Self::Int(v) => Some(*v as i64),
            Self::Long(v) => Some(*v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Self::String(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_compound(&self) -> Option<&Compound> {
        if let Self::Compound(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_raw_compound(&self) -> Option<&RawCompound> {
        if let Self::RawCompound(value) = self {
            Some(value)
        } else {
            None
        }
    }
    pub fn as_list(&self) -> Option<&[Nbt]> {
        if let Self::List(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn scalar_string(&self) -> String {
        match self {
            Self::End => "END".into(),
            Self::String(v) => v.clone(),
            Self::RawString(v) => String::from_utf8_lossy(v).into_owned(),
            Self::Byte(v) => v.to_string(),
            Self::Short(v) => v.to_string(),
            Self::Int(v) => v.to_string(),
            Self::Long(v) => v.to_string(),
            Self::Float(v) => v.to_string(),
            Self::Double(v) => v.to_string(),
            _ => to_snbt(self),
        }
    }
    pub fn from_json(v: &serde_json::Value) -> Self {
        use serde_json::Value;
        match v {
            Value::Null => Self::String(String::new()),
            Value::Bool(v) => Self::Byte(i8::from(*v)),
            Value::Number(v) => {
                if let Some(i) = v.as_i64() {
                    i32::try_from(i).map(Self::Int).unwrap_or(Self::Long(i))
                } else {
                    Self::Double(v.as_f64().unwrap_or_default())
                }
            }
            Value::String(v) => Self::String(v.clone()),
            Value::Array(v) => Self::List(v.iter().map(Self::from_json).collect()),
            Value::Object(v) => Self::Compound(
                v.iter()
                    .map(|(k, v)| (k.clone(), Self::from_json(v)))
                    .collect(),
            ),
        }
    }
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::{Value, json};
        match self {
            Self::End => Value::Null,
            Self::Byte(v) => json!(v),
            Self::Short(v) => json!(v),
            Self::Int(v) => json!(v),
            Self::Long(v) => json!(v),
            Self::Float(v) => json!(v),
            Self::Double(v) => json!(v),
            Self::String(v) => json!(v),
            Self::RawString(v) => json!(String::from_utf8_lossy(v)),
            Self::ByteArray(v) => json!(v),
            Self::IntArray(v) => json!(v),
            Self::LongArray(v) => json!(v),
            Self::List(v) => Value::Array(v.iter().map(Self::to_json).collect()),
            Self::Compound(v) => {
                Value::Object(v.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())
            }
            Self::RawCompound(v) => Value::Object(
                v.iter()
                    .map(|(k, v)| (String::from_utf8_lossy(k).into_owned(), v.to_json()))
                    .collect(),
            ),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    BigEndian,
    LittleEndian,
    NetworkBigEndian,
    NetworkLittleEndian,
}
impl Encoding {
    fn le(self) -> bool {
        matches!(self, Self::LittleEndian | Self::NetworkLittleEndian)
    }
    fn net(self) -> bool {
        self == Self::NetworkLittleEndian
    }
}
pub struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
    pub encoding: Encoding,
}
impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], encoding: Encoding) -> Self {
        Self {
            data,
            pos: 0,
            encoding,
        }
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| Error::invalid("NBT 长度溢出"))?;
        let s = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| Error::invalid("NBT 数据截断"))?;
        self.pos = end;
        Ok(s)
    }
    pub fn byte(&mut self) -> Result<u8> {
        let value = self.data.get(self.pos).copied();
        self.pos += 1;
        value.ok_or_else(|| {
            Error::Io(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))
        })
    }
    pub fn skip_byte(&mut self) -> Result<()> {
        self.byte().map(|_| ())
    }
    pub fn tag_type(&mut self) -> Result<u8> {
        self.byte()
    }
    pub fn varuint(&mut self, max: usize) -> Result<u64> {
        let mut v = 0;
        for i in 0..max {
            let b = self.byte()?;
            if i == 9 && b > 1 {
                return Err(Error::invalid("varint 溢出"));
            }
            v |= u64::from(b & 127) << (i * 7);
            if b & 128 == 0 {
                return Ok(v);
            }
        }
        Err(Error::invalid("varint 无效"))
    }
    pub fn short(&mut self) -> Result<i16> {
        let le = self.encoding.le();
        let available = self.data.len().saturating_sub(self.pos);
        if available < 2 {
            self.pos += available;
            return Err(Error::Io(std::io::Error::from(
                std::io::ErrorKind::UnexpectedEof,
            )));
        }
        let b = self.take(2)?.try_into().unwrap();
        Ok(if le {
            i16::from_le_bytes(b)
        } else {
            i16::from_be_bytes(b)
        })
    }
    pub fn skip_short(&mut self) -> Result<()> {
        self.short().map(|_| ())
    }
    fn skip_bytes(&mut self, count: usize) -> Result<()> {
        let available = self.data.len().saturating_sub(self.pos);
        if available < count {
            self.pos += available;
            return Err(Error::Io(std::io::Error::from(
                std::io::ErrorKind::UnexpectedEof,
            )));
        }
        self.pos += count;
        Ok(())
    }
    pub fn skip_int(&mut self) -> Result<()> {
        self.skip_bytes(4)
    }
    pub fn skip_float(&mut self) -> Result<()> {
        self.skip_bytes(4)
    }
    pub fn skip_long(&mut self) -> Result<()> {
        self.skip_bytes(8)
    }
    pub fn skip_double(&mut self) -> Result<()> {
        self.skip_bytes(8)
    }
    pub fn skip_string(&mut self) -> Result<()> {
        let length = self.short()?;
        if length < 0 {
            return Err(Error::invalid("NBT 字符串长度为负数"));
        }
        self.skip_bytes(length as usize)
    }
    pub fn skip_byte_array(&mut self) -> Result<()> {
        let length = self.int()?;
        if length < 0 {
            return Err(Error::invalid("NBT 字节数组长度为负数"));
        }
        self.skip_bytes(length as usize)
    }
    pub fn skip_int_array(&mut self) -> Result<()> {
        let count = self.int()?;
        if count < 0 {
            panic!("runtime error: makeslice: len out of range");
        }
        if self.encoding.net() {
            for _ in 0..count {
                self.int()?;
            }
            Ok(())
        } else {
            self.skip_bytes(count as usize * 4)
        }
    }
    pub fn skip_long_array(&mut self) -> Result<()> {
        let count = self.int()?;
        if count < 0 {
            panic!("runtime error: makeslice: len out of range");
        }
        if self.encoding.net() {
            for _ in 0..count {
                self.long()?;
            }
            Ok(())
        } else {
            self.skip_bytes(count as usize * 8)
        }
    }
    pub fn skip_list(&mut self) -> Result<()> {
        let element_tag = self.byte()?;
        if element_tag > 12 {
            return Err(Error::invalid(format!("无效 NBT 列表元素标签 {element_tag}")));
        }
        let count = self.int()?;
        if count < 0 {
            return Err(Error::invalid("NBT 列表长度为负数"));
        }
        for _ in 0..count {
            if element_tag == 0 {
                continue;
            }
            self.skip_tag_value(element_tag)?;
        }
        Ok(())
    }
    pub fn skip_compound(&mut self) -> Result<()> {
        loop {
            let tag = self.byte()?;
            if tag > 12 {
                return Err(Error::invalid(format!("无效 NBT 标签 {tag}")));
            }
            if tag == 0 {
                return Ok(());
            }
            self.skip_string()?;
            self.skip_tag_value(tag)?;
        }
    }
    pub fn skip_tag_value(&mut self, tag: u8) -> Result<()> {
        match tag {
            0 => Ok(()),
            1 => self.skip_byte(),
            2 => self.skip_short(),
            3 => self.skip_int(),
            4 => self.skip_long(),
            5 => self.skip_float(),
            6 => self.skip_double(),
            7 => self.skip_byte_array(),
            8 => self.skip_string(),
            9 => self.skip_list(),
            10 => self.skip_compound(),
            11 => self.skip_int_array(),
            12 => self.skip_long_array(),
            _ => Err(Error::invalid(format!("无效 NBT 标签 {tag}"))),
        }
    }
    pub fn skip_tag(&mut self) -> Result<()> {
        let tag = self.byte()?;
        if tag > 12 {
            return Err(Error::invalid(format!("无效 NBT 标签 {tag}")));
        }
        if tag == 0 {
            return Ok(());
        }
        self.skip_string()?;
        self.skip_tag_value(tag)
    }
    pub fn int(&mut self) -> Result<i32> {
        if self.encoding.net() {
            let mut value = 0u32;
            for index in 0..5 {
                let byte = self.byte()?;
                value |= u32::from(byte & 0x7f) << (index * 7);
                if byte & 0x80 == 0 {
                    let decoded = (value >> 1) as i32;
                    return Ok(if value & 1 != 0 { !decoded } else { decoded });
                }
            }
            Err(Error::invalid("varint 无效"))
        } else {
            let le = self.encoding.le();
            let available = self.data.len().saturating_sub(self.pos);
            if available < 4 {
                self.pos += available;
                return Err(Error::Io(std::io::Error::from(
                    std::io::ErrorKind::UnexpectedEof,
                )));
            }
            let b = self.take(4)?.try_into().unwrap();
            Ok(if le {
                i32::from_le_bytes(b)
            } else {
                i32::from_be_bytes(b)
            })
        }
    }
    pub fn long(&mut self) -> Result<i64> {
        if self.encoding.net() {
            let mut value = 0u64;
            for index in 0..10 {
                let byte = self.byte()?;
                value |= u64::from(byte & 0x7f) << (index * 7);
                if byte & 0x80 == 0 {
                    let decoded = (value >> 1) as i64;
                    return Ok(if value & 1 != 0 { !decoded } else { decoded });
                }
            }
            Err(Error::invalid("varint 无效"))
        } else {
            let le = self.encoding.le();
            let available = self.data.len().saturating_sub(self.pos);
            if available < 8 {
                self.pos += available;
                return Err(Error::Io(std::io::Error::from(
                    std::io::ErrorKind::UnexpectedEof,
                )));
            }
            let b = self.take(8)?.try_into().unwrap();
            Ok(if le {
                i64::from_le_bytes(b)
            } else {
                i64::from_be_bytes(b)
            })
        }
    }
    pub fn float(&mut self) -> Result<f32> {
        let le = self.encoding.le();
        let available = self.data.len().saturating_sub(self.pos);
        if available < 4 {
            self.pos += available;
            return Err(Error::Io(std::io::Error::from(
                std::io::ErrorKind::UnexpectedEof,
            )));
        }
        let b = self.take(4)?.try_into().unwrap();
        Ok(if le {
            f32::from_le_bytes(b)
        } else {
            f32::from_be_bytes(b)
        })
    }
    pub fn double(&mut self) -> Result<f64> {
        let le = self.encoding.le();
        let available = self.data.len().saturating_sub(self.pos);
        if available < 8 {
            self.pos += available;
            return Err(Error::Io(std::io::Error::from(
                std::io::ErrorKind::UnexpectedEof,
            )));
        }
        let b = self.take(8)?.try_into().unwrap();
        Ok(if le {
            f64::from_le_bytes(b)
        } else {
            f64::from_be_bytes(b)
        })
    }
    fn string_bytes(&mut self) -> Result<Vec<u8>> {
        let n = if self.encoding.net() {
            usize::try_from(self.varuint(5)?).map_err(|_| Error::invalid("字符串过长"))?
        } else {
            self.short()? as u16 as usize
        };
        if self.encoding.net() && n > i16::MAX as usize {
            return Err(Error::invalid("字符串过长"));
        }
        let available = self.data.len().saturating_sub(self.pos);
        if n > available {
            self.pos += available;
            return Err(Error::Io(std::io::Error::from(
                std::io::ErrorKind::UnexpectedEof,
            )));
        }
        Ok(self.take(n)?.to_vec())
    }

    pub fn tag_string_bytes(&mut self) -> Result<Vec<u8>> {
        self.string_bytes()
    }

    pub fn string(&mut self) -> Result<String> {
        String::from_utf8(self.string_bytes()?)
            .map_err(|_| Error::invalid("NBT 标签名称 UTF-8 无效"))
    }
    pub fn byte_array(&mut self) -> Result<Vec<u8>> {
        let n = self
            .int()
            .map_err(|_| Error::invalid("nbt: 操作 'Int32' 期间缓冲区意外结束"))?;
        if n < 0 {
            return Err(Error::invalid(
                "nbt: 操作 'ReadTagByteArray' 期间缓冲区意外结束",
            ));
        }
        let n = n as usize;
        let available = self.data.len().saturating_sub(self.pos);
        if n > available {
            self.pos += available;
            return Err(Error::invalid(
                "nbt: 操作 'ReadTagByteArray' 期间缓冲区意外结束",
            ));
        }
        Ok(self.take(n)?.to_vec())
    }
    pub fn read_tag(&mut self) -> Result<(u8, String)> {
        let (tag, bytes) = self.read_tag_bytes()?;
        let name = String::from_utf8(bytes)
            .map_err(|_| Error::invalid("NBT 标签名称 UTF-8 无效"))?;
        Ok((tag, name))
    }
    pub fn read_tag_bytes(&mut self) -> Result<(u8, Vec<u8>)> {
        let tag = self.tag_type()?;
        if tag > 12 {
            return Err(Error::invalid(format!("无效 NBT 标签 {tag}")));
        }
        if tag == 0 {
            return Ok((tag, Vec::new()));
        }
        Ok((tag, self.string_bytes()?))
    }
    pub fn read_tag_value(&mut self, tag: u8) -> Result<Option<Nbt>> {
        if tag == 0 {
            return Ok(None);
        }
        Ok(Some(self.value(tag, 0)?))
    }
    pub fn int_array(&mut self) -> Result<Vec<i32>> {
        let count = self.int()?;
        if count < 0 {
            panic!("runtime error: makeslice: len out of range");
        }
        let mut values = Vec::with_capacity(count as usize);
        for _ in 0..count {
            values.push(self.int()?);
        }
        Ok(values)
    }
    pub fn long_array(&mut self) -> Result<Vec<i64>> {
        let count = self.int()?;
        if count < 0 {
            panic!("runtime error: makeslice: len out of range");
        }
        let mut values = Vec::with_capacity(count as usize);
        for _ in 0..count {
            values.push(self.long()?);
        }
        Ok(values)
    }
    pub fn list(&mut self) -> Result<Vec<Nbt>> {
        self.list_at_depth(0, false)
    }
    fn list_at_depth(&mut self, depth: usize, enforce_depth_limit: bool) -> Result<Vec<Nbt>> {
        let element_tag = self.byte()?;
        if element_tag > 12 {
            return Err(Error::invalid(format!("无效 NBT 列表元素标签 {element_tag}")));
        }
        let count = self.int()?;
        if count < 0 {
            return Err(Error::invalid("NBT 列表长度为负数"));
        }
        let mut values = Vec::with_capacity((count as usize).min(4096));
        for _ in 0..count {
            if element_tag == 0 {
                values.push(Nbt::End);
            } else {
                values.push(self.value_inner(element_tag, depth + 1, enforce_depth_limit)?);
            }
        }
        Ok(values)
    }
    pub fn compound(&mut self) -> Result<Nbt> {
        self.compound_at_depth(0, false)
    }
    fn compound_at_depth(
        &mut self,
        depth: usize,
        enforce_depth_limit: bool,
    ) -> Result<Nbt> {
        let mut values = RawCompound::new();
        let mut all_names_utf8 = true;
        loop {
            let (tag, name) = self.read_tag_bytes()?;
            if tag == 0 {
                break;
            }
            all_names_utf8 &= std::str::from_utf8(&name).is_ok();
            let value = self.value_inner(tag, depth + 1, enforce_depth_limit)?;
            values.insert(name, value);
        }
        if all_names_utf8 {
            Ok(Nbt::Compound(
                values
                    .into_iter()
                    .map(|(key, value)| (String::from_utf8(key).unwrap(), value))
                    .collect(),
            ))
        } else {
            Ok(Nbt::RawCompound(values))
        }
    }
    pub fn value(&mut self, tag: u8, depth: usize) -> Result<Nbt> {
        self.value_inner(tag, depth, false)
    }
    fn value_inner(&mut self, tag: u8, depth: usize, enforce_depth_limit: bool) -> Result<Nbt> {
        if enforce_depth_limit && depth > 512 {
            return Err(Error::invalid("NBT 嵌套过深"));
        }
        Ok(match tag {
            1 => Nbt::Byte(self.byte()? as i8),
            2 => Nbt::Short(self.short()?),
            3 => Nbt::Int(self.int()?),
            4 => Nbt::Long(self.long()?),
            5 => Nbt::Float(self.float()?),
            6 => Nbt::Double(self.double()?),
            7 => Nbt::ByteArray(self.byte_array()?.into_iter().map(|b| b as i8).collect()),
            8 => {
                let bytes = self.string_bytes()?;
                match String::from_utf8(bytes) {
                    Ok(value) => Nbt::String(value),
                    Err(error) => Nbt::RawString(error.into_bytes()),
                }
            }
            9 => Nbt::List(self.list_at_depth(depth, enforce_depth_limit)?),
            10 => self.compound_at_depth(depth, enforce_depth_limit)?,
            11 => {
                Nbt::IntArray(self.int_array()?)
            }
            12 => {
                Nbt::LongArray(self.long_array()?)
            }
            _ => return Err(Error::invalid(format!("无效 NBT 标签 {tag}"))),
        })
    }
    pub fn named(&mut self) -> Result<(String, Nbt)> {
        let tag = self.byte()?;
        let name = if self.encoding == Encoding::NetworkBigEndian && tag == 10 {
            String::new()
        } else {
            self.string()?
        };
        Ok((name, self.value_inner(tag, 0, true)?))
    }
}
pub fn decode(bytes: &[u8], encoding: Encoding) -> Result<(String, Nbt)> {
    if encoding == Encoding::NetworkLittleEndian && bytes.len() > 4 * 1024 * 1024 {
        return Err(Error::invalid("网络 NBT 超过 4MiB"));
    }
    Reader::new(bytes, encoding).named()
}

pub trait TagWriteSink {
    fn write(&mut self, bytes: &[u8]) -> (usize, Option<std::io::Error>);
}

pub struct IoWriteSink<W>(pub W);

impl<W> IoWriteSink<W> {
    pub fn new(writer: W) -> Self {
        Self(writer)
    }

    pub fn into_inner(self) -> W {
        self.0
    }
}

impl<W: std::io::Write> TagWriteSink for IoWriteSink<W> {
    fn write(&mut self, bytes: &[u8]) -> (usize, Option<std::io::Error>) {
        match self.0.write(bytes) {
            Ok(written) => (written, None),
            Err(error) => (0, Some(error)),
        }
    }
}

pub struct OffsetWriter<S> {
    sink: S,
    offset: u64,
}

impl<S> OffsetWriter<S> {
    pub fn new(sink: S) -> Self {
        Self { sink, offset: 0 }
    }

    pub fn with_offset(sink: S, offset: u64) -> Self {
        Self { sink, offset }
    }

    pub fn get_offset(&self) -> u64 {
        self.offset
    }

    pub fn write(&mut self, bytes: &[u8]) -> (usize, Option<std::io::Error>)
    where
        S: TagWriteSink,
    {
        let (written, error) = self.sink.write(bytes);
        self.offset += written as u64;
        (written, error)
    }

    pub fn into_inner(self) -> S {
        self.sink
    }
}

pub struct Writer {
    pub data: Vec<u8>,
    pub encoding: Encoding,
}
impl Writer {
    pub fn new(encoding: Encoding) -> Self {
        Self {
            data: vec![],
            encoding,
        }
    }
    pub fn write_tag_type(&mut self, tag: u8) -> Result<()> {
        if tag > 12 {
            return Err(Error::invalid(format!(
                "nbt: 在偏移 {} 处的操作 'WriteTagType' 发现未知标签 '%!v(PANIC=String method: 未知标签)'",
                self.data.len()
            )));
        }
        self.data.push(tag);
        Ok(())
    }
    pub fn write_tag(&mut self, tag: u8, name: &str) -> Result<()> {
        self.write_tag_type(tag)?;
        if tag == 0 || (self.encoding == Encoding::NetworkBigEndian && tag == 10) {
            return Ok(());
        }
        self.write_tag_string(name)
    }
    pub fn write_tag_value(&mut self, tag: u8, value: &Nbt) -> Result<()> {
        if tag == 0 {
            return Ok(());
        }
        if tag > 12 {
            return Err(Error::invalid(format!(
                "nbt: 在偏移 {} 处的操作 'WriteTagValue' 发现未知标签 '%!v(PANIC=String method: 未知标签)'",
                self.data.len()
            )));
        }
        if tag != value.tag() {
            let value_type = match value {
                Nbt::End => "<nil>",
                Nbt::Byte(_) => "uint8",
                Nbt::Short(_) => "int16",
                Nbt::Int(_) => "int32",
                Nbt::Long(_) => "int64",
                Nbt::Float(_) => "float32",
                Nbt::Double(_) => "float64",
                Nbt::ByteArray(_) => "[]uint8",
                Nbt::String(_) | Nbt::RawString(_) => "string",
                Nbt::List(_) => "[]interface {}",
                Nbt::Compound(_) | Nbt::RawCompound(_) => "map[string]interface {}",
                Nbt::IntArray(_) => "[]int32",
                Nbt::LongArray(_) => "[]int64",
            };
            return Err(Error::invalid(format!(
                "nbt: 值类型 {value_type}（{}）无法转换为 NBT 标签",
                ""
            )));
        }
        self.value_at_depth(value, 0, "")
    }
    pub fn write_tag_list(&mut self, values: &[Nbt]) -> Result<()> {
        self.write_tag_list_at_depth(values, 0, "")
    }
    pub fn write_tag_compound(&mut self, values: &Compound) -> Result<()> {
        self.write_tag_compound_at_depth(values, 0, "")
    }
    pub fn write_tag_compound_to<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        values: &Compound,
    ) -> Result<()> {
        self.write_tag_compound_to_at_depth(writer, values, 0, "")
    }
    pub fn write_tag_list_to<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        values: &[Nbt],
    ) -> Result<()> {
        self.write_tag_list_to_at_depth(writer, values, 0, "")
    }
    fn write_tag_list_to_at_depth<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        values: &[Nbt],
        depth: usize,
        list_name: &str,
    ) -> Result<()> {
        if depth >= 512 {
            return Err(Error::Other("nbt: 达到最大嵌套深度 512".into()));
        }
        let element_tag = values.first().map_or(0, Nbt::tag);
        if element_tag == 0 && !values.is_empty() {
            return Err(Self::incompatible_list_value(&values[0], list_name, 0));
        }
        self.write_tag_byte_to(writer, element_tag)?;
        self.write_tag_int32_to(writer, values.len() as i32)?;
        for (index, value) in values.iter().enumerate() {
            if value.tag() != element_tag {
                return Err(Self::incompatible_list_value(value, list_name, index));
            }
            let value_name = if list_name.is_empty() {
                format!("列表[{index}]")
            } else {
                format!("{list_name}[{index}]")
            };
            self.write_value_to_at_depth(writer, value, depth + 1, &value_name)?;
        }
        Ok(())
    }
    fn write_value_to_at_depth<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        value: &Nbt,
        depth: usize,
        value_name: &str,
    ) -> Result<()> {
        match value {
            Nbt::End => Ok(()),
            Nbt::Byte(value) => self.write_tag_byte_to(writer, *value as u8),
            Nbt::Short(value) => self.write_tag_int16_to(writer, *value),
            Nbt::Int(value) => self.write_tag_int32_to(writer, *value),
            Nbt::Long(value) => self.write_tag_int64_to(writer, *value),
            Nbt::Float(value) => self.write_tag_float32_to(writer, *value),
            Nbt::Double(value) => self.write_tag_float64_to(writer, *value),
            Nbt::String(value) => self.write_tag_string_to(writer, value),
            Nbt::RawString(value) => self.write_tag_string_bytes_to(writer, value),
            Nbt::ByteArray(values) => {
                let bytes: Vec<u8> = values.iter().map(|value| *value as u8).collect();
                self.write_tag_byte_array_to(writer, &bytes)
            }
            Nbt::IntArray(values) => self.write_tag_int32_array_to(writer, values),
            Nbt::LongArray(values) => self.write_tag_int64_array_to(writer, values),
            Nbt::List(values) => {
                self.write_tag_list_to_at_depth(writer, values, depth, value_name)
            }
            Nbt::Compound(values) => {
                self.write_tag_compound_to_at_depth(writer, values, depth, value_name)
            }
            Nbt::RawCompound(values) => {
                self.write_raw_compound_to_at_depth(writer, values, depth, value_name)
            }
        }
    }
    fn write_tag_compound_to_at_depth<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        values: &Compound,
        depth: usize,
        compound_name: &str,
    ) -> Result<()> {
        if depth >= 512 {
            return Err(Error::Other("nbt: 达到最大嵌套深度 512".into()));
        }
        for (name, value) in values {
            let entry_name = if compound_name.is_empty() {
                name.clone()
            } else {
                format!("{compound_name}.{name}")
            };
            if value.tag() == 0 {
                return Err(Self::incompatible_value(value, &entry_name));
            }
            self.write_tag_byte_to(writer, value.tag())?;
            self.write_tag_string_to(writer, name)?;
            self.write_value_to_at_depth(writer, value, depth + 1, &entry_name)?;
        }
        self.write_tag_byte_to(writer, 0)
    }
    fn write_raw_compound_to_at_depth<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        values: &RawCompound,
        depth: usize,
        compound_name: &str,
    ) -> Result<()> {
        if depth >= 512 {
            return Err(Error::Other("nbt: 达到最大嵌套深度 512".into()));
        }
        for (name, value) in values {
            let name_text = String::from_utf8_lossy(name);
            let entry_name = if compound_name.is_empty() {
                name_text.to_string()
            } else {
                format!("{compound_name}.{name_text}")
            };
            if value.tag() == 0 {
                return Err(Self::incompatible_value(value, &entry_name));
            }
            self.write_tag_byte_to(writer, value.tag())?;
            self.write_tag_string_bytes_to(writer, name)?;
            self.write_value_to_at_depth(writer, value, depth + 1, &entry_name)?;
        }
        self.write_tag_byte_to(writer, 0)
    }
    fn write_tag_string_bytes_to<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        value: &[u8],
    ) -> Result<()> {
        if value.len() > i16::MAX as usize {
            return Err(Error::Other(format!(
                "nbt: 偏移 {} 的字符串无效: 字符串长度超过最大限制（长度={}）",
                writer.offset,
                value.len()
            )));
        }
        if self.encoding.net() {
            let mut prefix = Self::new(self.encoding);
            prefix.varuint(value.len() as u64);
            for byte in prefix.data {
                self.write_tag_byte_to(writer, byte)?;
            }
        } else {
            self.write_tag_int16_to(writer, value.len() as i16)?;
        }
        if !value.is_empty() {
            let (written, error) = writer.sink.write(value);
            writer.offset += written as u64;
            if let Some(error) = error {
                return Err(Error::Other(format!(
                    "nbt: 在偏移 {} 的操作 'WriteString' 写入失败: {error}",
                    writer.offset
                )));
            }
        }
        Ok(())
    }
    fn write_tag_list_at_depth(
        &mut self,
        values: &[Nbt],
        depth: usize,
        list_name: &str,
    ) -> Result<()> {
        if depth >= 512 {
            return Err(Error::Other("nbt: 达到最大嵌套深度 512".into()));
        }
        let element_tag = values.first().map_or(0, Nbt::tag);
        if element_tag == 0 && !values.is_empty() {
            return Err(Self::incompatible_list_value(&values[0], list_name, 0));
        }
        self.data.push(element_tag);
        self.int(values.len() as i32);
        for (index, value) in values.iter().enumerate() {
            if value.tag() != element_tag {
                return Err(Self::incompatible_list_value(value, list_name, index));
            }
            let value_name = if list_name.is_empty() {
                format!("列表[{index}]")
            } else {
                format!("{list_name}[{index}]")
            };
            self.value_at_depth(value, depth + 1, &value_name)?;
        }
        Ok(())
    }
    fn write_tag_compound_at_depth(
        &mut self,
        values: &Compound,
        depth: usize,
        compound_name: &str,
    ) -> Result<()> {
        if depth >= 512 {
            return Err(Error::Other("nbt: 达到最大嵌套深度 512".into()));
        }
        for (name, value) in values {
            let entry_name = if compound_name.is_empty() {
                name.clone()
            } else {
                format!("{compound_name}.{name}")
            };
            if value.tag() == 0 {
                return Err(Self::incompatible_value(value, &entry_name));
            }
            self.data.push(value.tag());
            self.write_tag_string(name)?;
            self.value_at_depth(value, depth + 1, &entry_name)?;
        }
        self.data.push(0);
        Ok(())
    }
    fn write_raw_compound_at_depth(
        &mut self,
        values: &RawCompound,
        depth: usize,
        compound_name: &str,
    ) -> Result<()> {
        if depth >= 512 {
            return Err(Error::Other("nbt: 达到最大嵌套深度 512".into()));
        }
        for (name, value) in values {
            let name_text = String::from_utf8_lossy(name);
            let entry_name = if compound_name.is_empty() {
                name_text.to_string()
            } else {
                format!("{compound_name}.{name_text}")
            };
            if value.tag() == 0 {
                return Err(Self::incompatible_value(value, &entry_name));
            }
            self.data.push(value.tag());
            if name.len() > i16::MAX as usize {
                return Err(Error::Other(format!(
                    "nbt: 偏移 {} 的字符串无效: 字符串长度超过最大限制（长度={}）",
                    self.data.len(),
                    name.len()
                )));
            }
            if self.encoding.net() {
                self.varuint(name.len() as u64);
            } else {
                self.short(name.len() as i16);
            }
            self.data.extend_from_slice(name);
            self.value_at_depth(value, depth + 1, &entry_name)?;
        }
        self.data.push(0);
        Ok(())
    }
    fn incompatible_list_value(value: &Nbt, list_name: &str, index: usize) -> Error {
        let value_name = if list_name.is_empty() {
            format!("列表[{index}]")
        } else {
            format!("{list_name}[{index}]")
        };
        Self::incompatible_value(value, &value_name)
    }
    fn incompatible_value(value: &Nbt, value_name: &str) -> Error {
        let value_type = match value {
            Nbt::End => "<nil>",
            Nbt::Byte(_) => "uint8",
            Nbt::Short(_) => "int16",
            Nbt::Int(_) => "int32",
            Nbt::Long(_) => "int64",
            Nbt::Float(_) => "float32",
            Nbt::Double(_) => "float64",
            Nbt::ByteArray(_) => "[]uint8",
            Nbt::String(_) | Nbt::RawString(_) => "string",
            Nbt::List(_) => "[]interface {}",
            Nbt::Compound(_) | Nbt::RawCompound(_) => "map[string]interface {}",
            Nbt::IntArray(_) => "[]int32",
            Nbt::LongArray(_) => "[]int64",
        };
        Error::Other(format!(
            "nbt: 值类型 {value_type}（{value_name}）无法转换为 NBT 标签"
        ))
    }
    pub fn write_tag_to<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        tag: u8,
        name: &str,
    ) -> Result<()> {
        if tag > 12 {
            return Err(Error::invalid(format!(
                "nbt: 在偏移 {} 处的操作 'WriteTagType' 发现未知标签 '%!v(PANIC=String method: 未知标签)'",
                writer.offset
            )));
        }
        let (written, error) = writer.sink.write(&[tag]);
        writer.offset += written as u64;
        if let Some(error) = error {
            return Err(Error::Other(format!(
                "nbt: 在偏移 {} 的操作 'WriteTagType' 写入失败: {error}",
                writer.offset
            )));
        }
        if written != 1 {
            return Err(Error::Other(format!(
                "nbt: 在偏移 {} 的操作 'WriteTagType' 写入失败: short write",
                writer.offset
            )));
        }
        if tag == 0 || (self.encoding == Encoding::NetworkBigEndian && tag == 10) {
            return Ok(());
        }
        self.write_tag_string_to(writer, name)
    }
    pub fn write_tag_byte(&mut self, value: u8) -> Result<()> {
        self.data.push(value);
        Ok(())
    }
    pub fn write_tag_byte_to<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        value: u8,
    ) -> Result<()> {
        let (written, error) = writer.sink.write(&[value]);
        writer.offset += written as u64;
        if let Some(error) = error {
            return Err(Error::Other(format!(
                "nbt: 在偏移 {} 的操作 'WriteTagByte' 写入失败: {error}",
                writer.offset
            )));
        }
        if written != 1 {
            return Err(Error::Other(format!(
                "nbt: 在偏移 {} 的操作 'WriteTagByte' 写入失败: short write",
                writer.offset
            )));
        }
        Ok(())
    }
    pub fn write_tag_int16(&mut self, value: i16) -> Result<()> {
        self.short(value);
        Ok(())
    }
    pub fn write_tag_int16_to<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        value: i16,
    ) -> Result<()> {
        let bytes = if self.encoding.le() {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        };
        let (written, error) = writer.sink.write(&bytes);
        writer.offset += written as u64;
        if let Some(error) = error {
            return Err(Error::Other(format!(
                "nbt: 在偏移 {} 的操作 'WriteInt16' 写入失败: {error}",
                writer.offset
            )));
        }
        Ok(())
    }
    pub fn write_tag_int32(&mut self, value: i32) -> Result<()> {
        self.int(value);
        Ok(())
    }
    pub fn write_tag_int32_to<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        value: i32,
    ) -> Result<()> {
        if self.encoding.net() {
            let mut encoded = Self::new(self.encoding);
            encoded.int(value);
            for byte in encoded.data {
                let (written, error) = writer.sink.write(&[byte]);
                writer.offset += written as u64;
                if error.is_some() || written != 1 {
                    return Err(Error::Other(format!(
                        "nbt: 在偏移 {} 的操作 'WriteInt32' 写入失败: <nil>",
                        writer.offset
                    )));
                }
            }
        } else {
            let bytes = if self.encoding.le() {
                value.to_le_bytes()
            } else {
                value.to_be_bytes()
            };
            let (written, error) = writer.sink.write(&bytes);
            writer.offset += written as u64;
            if error.is_some() {
                return Err(Error::Other(format!(
                    "nbt: 在偏移 {} 的操作 'WriteInt32' 写入失败: <nil>",
                    writer.offset
                )));
            }
        }
        Ok(())
    }
    pub fn write_tag_int64(&mut self, value: i64) -> Result<()> {
        self.long(value);
        Ok(())
    }
    pub fn write_tag_int64_to<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        value: i64,
    ) -> Result<()> {
        if self.encoding.net() {
            let mut encoded = Self::new(self.encoding);
            encoded.long(value);
            for byte in encoded.data {
                let (written, error) = writer.sink.write(&[byte]);
                writer.offset += written as u64;
                if error.is_some() || written != 1 {
                    return Err(Error::Other(format!(
                        "nbt: 在偏移 {} 的操作 'WriteInt64' 写入失败: <nil>",
                        writer.offset
                    )));
                }
            }
        } else {
            let bytes = if self.encoding.le() {
                value.to_le_bytes()
            } else {
                value.to_be_bytes()
            };
            let (written, error) = writer.sink.write(&bytes);
            writer.offset += written as u64;
            if error.is_some() {
                return Err(Error::Other(format!(
                    "nbt: 在偏移 {} 的操作 'WriteInt64' 写入失败: <nil>",
                    writer.offset
                )));
            }
        }
        Ok(())
    }
    pub fn write_tag_float32(&mut self, value: f32) -> Result<()> {
        let bits = value.to_bits();
        self.data.extend(if self.encoding.le() {
            bits.to_le_bytes()
        } else {
            bits.to_be_bytes()
        });
        Ok(())
    }
    pub fn write_tag_float32_to<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        value: f32,
    ) -> Result<()> {
        let bits = value.to_bits();
        let bytes = if self.encoding.le() {
            bits.to_le_bytes()
        } else {
            bits.to_be_bytes()
        };
        let (written, error) = writer.sink.write(&bytes);
        writer.offset += written as u64;
        if error.is_some() {
            return Err(Error::Other(format!(
                "nbt: 在偏移 {} 的操作 'WriteFloat32' 写入失败: <nil>",
                writer.offset
            )));
        }
        Ok(())
    }
    pub fn write_tag_float64(&mut self, value: f64) -> Result<()> {
        let bits = value.to_bits();
        self.data.extend(if self.encoding.le() {
            bits.to_le_bytes()
        } else {
            bits.to_be_bytes()
        });
        Ok(())
    }
    pub fn write_tag_float64_to<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        value: f64,
    ) -> Result<()> {
        let bits = value.to_bits();
        let bytes = if self.encoding.le() {
            bits.to_le_bytes()
        } else {
            bits.to_be_bytes()
        };
        let (written, error) = writer.sink.write(&bytes);
        writer.offset += written as u64;
        if error.is_some() {
            return Err(Error::Other(format!(
                "nbt: 在偏移 {} 的操作 'WriteFloat64' 写入失败: <nil>",
                writer.offset
            )));
        }
        Ok(())
    }
    pub fn write_tag_string(&mut self, value: &str) -> Result<()> {
        if value.len() > i16::MAX as usize {
            return Err(Error::Other(format!(
                "nbt: 偏移 {} 的字符串无效: 字符串长度超过最大限制（长度={}）",
                self.data.len(),
                value.len()
            )));
        }
        if self.encoding.net() {
            self.varuint(value.len() as u64);
        } else {
            self.short(value.len() as i16);
        }
        self.data.extend_from_slice(value.as_bytes());
        Ok(())
    }
    pub fn write_tag_string_to<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        value: &str,
    ) -> Result<()> {
        if value.len() > i16::MAX as usize {
            return Err(Error::Other(format!(
                "nbt: 偏移 {} 的字符串无效: 字符串长度超过最大限制（长度={}）",
                writer.offset,
                value.len()
            )));
        }
        let length = value.len();
        let length_bytes = if self.encoding.net() {
            let mut prefix = Self::new(self.encoding);
            prefix.varuint(length as u64);
            prefix.data
        } else if self.encoding.le() {
            (length as i16).to_le_bytes().to_vec()
        } else {
            (length as i16).to_be_bytes().to_vec()
        };
        if self.encoding.net() {
            for byte in length_bytes {
                let (written, error) = writer.sink.write(&[byte]);
                writer.offset += written as u64;
                if error.is_some() || written != 1 {
                    return Err(Error::Other(format!(
                        "nbt: 在偏移 {} 的操作 'WriteString' 写入失败: <nil>",
                        writer.offset
                    )));
                }
            }
        } else {
            let (written, error) = writer.sink.write(&length_bytes);
            writer.offset += written as u64;
            if error.is_some() {
                return Err(Error::Other(format!(
                    "nbt: 在偏移 {} 的操作 'WriteString' 写入失败: <nil>",
                    writer.offset
                )));
            }
        }
        if !value.is_empty() {
            let (written, error) = writer.sink.write(value.as_bytes());
            writer.offset += written as u64;
            if let Some(error) = error {
                return Err(Error::Other(format!(
                    "nbt: 在偏移 {} 的操作 'WriteString' 写入失败: {error}",
                    writer.offset
                )));
            }
        }
        Ok(())
    }
    pub fn write_tag_byte_array(&mut self, data: &[u8]) -> Result<()> {
        if data.len() > i32::MAX as usize {
            return Err(Error::Other(format!(
                "nbt: 在偏移 {} 的操作 'WriteTagByteArray' 写入失败: 字节数组长度 {} 超过最大值 {}",
                self.data.len(),
                data.len(),
                i32::MAX
            )));
        }
        self.int(data.len() as i32);
        self.data.extend_from_slice(data);
        Ok(())
    }
    pub fn write_tag_byte_array_to<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        data: &[u8],
    ) -> Result<()> {
        if data.len() > i32::MAX as usize {
            return Err(Error::Other(format!(
                "nbt: 在偏移 {} 的操作 'WriteTagByteArray' 写入失败: 字节数组长度 {} 超过最大值 {}",
                writer.offset,
                data.len(),
                i32::MAX
            )));
        }
        let length = data.len() as i32;
        let length_bytes = if self.encoding.net() {
            let mut prefix = Self::new(self.encoding);
            prefix.int(length);
            prefix.data
        } else if self.encoding.le() {
            length.to_le_bytes().to_vec()
        } else {
            length.to_be_bytes().to_vec()
        };
        if self.encoding.net() {
            for byte in length_bytes {
                let (written, error) = writer.sink.write(&[byte]);
                writer.offset += written as u64;
                if error.is_some() || written != 1 {
                    return Err(Error::Other(format!(
                        "nbt: 在偏移 {} 的操作 'WriteInt32' 写入失败: <nil>",
                        writer.offset
                    )));
                }
            }
        } else {
            let (written, error) = writer.sink.write(&length_bytes);
            writer.offset += written as u64;
            if error.is_some() {
                return Err(Error::Other(format!(
                    "nbt: 在偏移 {} 的操作 'WriteInt32' 写入失败: <nil>",
                    writer.offset
                )));
            }
        }
        if !data.is_empty() {
            let (written, error) = writer.sink.write(data);
            writer.offset += written as u64;
            if let Some(error) = error {
                return Err(Error::Other(format!(
                    "nbt: 在偏移 {} 的操作 'WriteTagByteArray' 写入失败: {error}",
                    writer.offset
                )));
            }
            if written != data.len() {
                return Err(Error::Other(format!(
                    "nbt: 在偏移 {} 的操作 'WriteTagByteArray' 写入失败: short write",
                    writer.offset
                )));
            }
        }
        Ok(())
    }
    pub fn write_tag_int32_array(&mut self, data: &[i32]) -> Result<()> {
        if data.len() > i32::MAX as usize {
            return Err(Error::Other(format!(
                "nbt: 在偏移 {} 的操作 'WriteTagInt32Array' 写入失败: int32 数组长度 {} 超过最大值 {}",
                self.data.len(),
                data.len(),
                i32::MAX
            )));
        }
        self.int(data.len() as i32);
        for value in data {
            self.int(*value);
        }
        Ok(())
    }
    pub fn write_tag_int32_array_to<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        data: &[i32],
    ) -> Result<()> {
        if data.len() > i32::MAX as usize {
            return Err(Error::Other(format!(
                "nbt: 在偏移 {} 的操作 'WriteTagInt32Array' 写入失败: int32 数组长度 {} 超过最大值 {}",
                writer.offset,
                data.len(),
                i32::MAX
            )));
        }
        for value in std::iter::once(&(data.len() as i32)).chain(data.iter()) {
            let encoded = if self.encoding.net() {
                let mut temporary = Self::new(self.encoding);
                temporary.int(*value);
                temporary.data
            } else if self.encoding.le() {
                value.to_le_bytes().to_vec()
            } else {
                value.to_be_bytes().to_vec()
            };
            if self.encoding.net() {
                for byte in encoded {
                    let (written, error) = writer.sink.write(&[byte]);
                    writer.offset += written as u64;
                    if error.is_some() || written != 1 {
                        return Err(Error::Other(format!(
                            "nbt: 在偏移 {} 的操作 'WriteInt32' 写入失败: <nil>",
                            writer.offset
                        )));
                    }
                }
            } else {
                let (written, error) = writer.sink.write(&encoded);
                writer.offset += written as u64;
                if error.is_some() {
                    return Err(Error::Other(format!(
                        "nbt: 在偏移 {} 的操作 'WriteInt32' 写入失败: <nil>",
                        writer.offset
                    )));
                }
            }
        }
        Ok(())
    }
    pub fn write_tag_int64_array(&mut self, data: &[i64]) -> Result<()> {
        if data.len() > i32::MAX as usize {
            return Err(Error::Other(format!(
                "nbt: 在偏移 {} 的操作 'WriteTagInt64Array' 写入失败: int64 数组长度 {} 超过最大值 {}",
                self.data.len(),
                data.len(),
                i32::MAX
            )));
        }
        self.int(data.len() as i32);
        for value in data {
            self.long(*value);
        }
        Ok(())
    }
    pub fn write_tag_int64_array_to<S: TagWriteSink>(
        &self,
        writer: &mut OffsetWriter<S>,
        data: &[i64],
    ) -> Result<()> {
        if data.len() > i32::MAX as usize {
            return Err(Error::Other(format!(
                "nbt: 在偏移 {} 的操作 'WriteTagInt64Array' 写入失败: int64 数组长度 {} 超过最大值 {}",
                writer.offset,
                data.len(),
                i32::MAX
            )));
        }
        let mut length_writer = Self::new(self.encoding);
        length_writer.int(data.len() as i32);
        if self.encoding.net() {
            for byte in length_writer.data {
                let (written, error) = writer.sink.write(&[byte]);
                writer.offset += written as u64;
                if error.is_some() || written != 1 {
                    return Err(Error::Other(format!(
                        "nbt: 在偏移 {} 的操作 'WriteInt32' 写入失败: <nil>",
                        writer.offset
                    )));
                }
            }
        } else {
            let (written, error) = writer.sink.write(&length_writer.data);
            writer.offset += written as u64;
            if error.is_some() {
                return Err(Error::Other(format!(
                    "nbt: 在偏移 {} 的操作 'WriteInt32' 写入失败: <nil>",
                    writer.offset
                )));
            }
        }
        for value in data {
            let encoded = if self.encoding.net() {
                let mut temporary = Self::new(self.encoding);
                temporary.long(*value);
                temporary.data
            } else if self.encoding.le() {
                value.to_le_bytes().to_vec()
            } else {
                value.to_be_bytes().to_vec()
            };
            if self.encoding.net() {
                for byte in encoded {
                    let (written, error) = writer.sink.write(&[byte]);
                    writer.offset += written as u64;
                    if error.is_some() || written != 1 {
                        return Err(Error::Other(format!(
                            "nbt: 在偏移 {} 的操作 'WriteInt64' 写入失败: <nil>",
                            writer.offset
                        )));
                    }
                }
            } else {
                let (written, error) = writer.sink.write(&encoded);
                writer.offset += written as u64;
                if error.is_some() {
                    return Err(Error::Other(format!(
                        "nbt: 在偏移 {} 的操作 'WriteInt64' 写入失败: <nil>",
                        writer.offset
                    )));
                }
            }
        }
        Ok(())
    }
    pub fn varuint(&mut self, mut n: u64) {
        while n >= 128 {
            self.data.push(n as u8 | 128);
            n >>= 7;
        }
        self.data.push(n as u8);
    }
    pub fn short(&mut self, n: i16) {
        self.data.extend(if self.encoding.le() {
            n.to_le_bytes()
        } else {
            n.to_be_bytes()
        });
    }
    pub fn int(&mut self, n: i32) {
        if self.encoding.net() {
            self.varuint(u64::from(((n as u32) << 1) ^ ((n >> 31) as u32)));
        } else {
            self.data.extend(if self.encoding.le() {
                n.to_le_bytes()
            } else {
                n.to_be_bytes()
            });
        }
    }
    pub fn long(&mut self, n: i64) {
        if self.encoding.net() {
            self.varuint(((n as u64) << 1) ^ ((n >> 63) as u64));
        } else {
            self.data.extend(if self.encoding.le() {
                n.to_le_bytes()
            } else {
                n.to_be_bytes()
            });
        }
    }
    pub fn string(&mut self, s: &str) -> Result<()> {
        if s.len() > u16::MAX as usize {
            return Err(Error::invalid("NBT 字符串过长"));
        }
        if self.encoding.net() {
            self.varuint(s.len() as u64)
        } else {
            self.short(s.len() as i16)
        }
        self.data.extend_from_slice(s.as_bytes());
        Ok(())
    }
    fn string_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.len() > u16::MAX as usize {
            return Err(Error::invalid("NBT 字符串过长"));
        }
        if self.encoding.net() {
            self.varuint(bytes.len() as u64);
        } else {
            self.short(bytes.len() as i16);
        }
        self.data.extend_from_slice(bytes);
        Ok(())
    }
    pub fn value(&mut self, v: &Nbt) -> Result<()> {
        match v {
            Nbt::End => {}
            Nbt::Byte(n) => self.data.push(*n as u8),
            Nbt::Short(n) => self.short(*n),
            Nbt::Int(n) => self.int(*n),
            Nbt::Long(n) => self.long(*n),
            Nbt::Float(n) => self.data.extend(if self.encoding.le() {
                n.to_le_bytes()
            } else {
                n.to_be_bytes()
            }),
            Nbt::Double(n) => self.data.extend(if self.encoding.le() {
                n.to_le_bytes()
            } else {
                n.to_be_bytes()
            }),
            Nbt::String(s) => self.string(s)?,
            Nbt::RawString(bytes) => {
                self.string_bytes(bytes)?;
            }
            Nbt::ByteArray(v) => {
                self.int(i32::try_from(v.len()).map_err(|_| Error::invalid("NBT 数组过长"))?);
                self.data.extend(v.iter().map(|b| *b as u8));
            }
            Nbt::List(v) => {
                let t = v.first().map_or(0, Nbt::tag);
                if v.iter().any(|n| n.tag() != t) {
                    return Err(Error::invalid("NBT List 包含不同类型"));
                }
                self.data.push(t);
                self.int(
                    v.len()
                        .try_into()
                        .map_err(|_| Error::invalid("NBT List 过长"))?,
                );
                for n in v {
                    self.value(n)?;
                }
            }
            Nbt::Compound(m) => {
                for (k, v) in m {
                    self.data.push(v.tag());
                    self.string(k)?;
                    self.value(v)?;
                }
                self.data.push(0);
            }
            Nbt::RawCompound(m) => {
                for (key, value) in m {
                    self.data.push(value.tag());
                    self.string_bytes(key)?;
                    self.value(value)?;
                }
                self.data.push(0);
            }
            Nbt::IntArray(v) => {
                self.int(
                    v.len()
                        .try_into()
                        .map_err(|_| Error::invalid("NBT 数组过长"))?,
                );
                for n in v {
                    self.int(*n);
                }
            }
            Nbt::LongArray(v) => {
                self.int(
                    v.len()
                        .try_into()
                        .map_err(|_| Error::invalid("NBT 数组过长"))?,
                );
                for n in v {
                    self.long(*n);
                }
            }
        }
        Ok(())
    }
    fn value_at_depth(&mut self, v: &Nbt, depth: usize, value_name: &str) -> Result<()> {
        match v {
            Nbt::End => {}
            Nbt::Byte(n) => self.data.push(*n as u8),
            Nbt::Short(n) => self.short(*n),
            Nbt::Int(n) => self.int(*n),
            Nbt::Long(n) => self.long(*n),
            Nbt::Float(n) => self.data.extend(if self.encoding.le() {
                n.to_le_bytes()
            } else {
                n.to_be_bytes()
            }),
            Nbt::Double(n) => self.data.extend(if self.encoding.le() {
                n.to_le_bytes()
            } else {
                n.to_be_bytes()
            }),
            Nbt::String(value) => self.write_tag_string(value)?,
            Nbt::RawString(bytes) => {
                if bytes.len() > i16::MAX as usize {
                    return Err(Error::Other(format!(
                        "nbt: 偏移 {} 的字符串无效: 字符串长度超过最大限制（长度={}）",
                        self.data.len(),
                        bytes.len()
                    )));
                }
                if self.encoding.net() {
                    self.varuint(bytes.len() as u64);
                } else {
                    self.short(bytes.len() as i16);
                }
                self.data.extend_from_slice(bytes);
            }
            Nbt::ByteArray(values) => {
                self.int(i32::try_from(values.len()).map_err(|_| Error::invalid("NBT 数组过长"))?);
                self.data.extend(values.iter().map(|value| *value as u8));
            }
            Nbt::List(values) => self.write_tag_list_at_depth(values, depth, value_name)?,
            Nbt::Compound(values) => {
                self.write_tag_compound_at_depth(values, depth, value_name)?
            }
            Nbt::RawCompound(values) => {
                self.write_raw_compound_at_depth(values, depth, value_name)?
            }
            Nbt::IntArray(values) => {
                self.int(values.len().try_into().map_err(|_| Error::invalid("NBT 数组过长"))?);
                for value in values {
                    self.int(*value);
                }
            }
            Nbt::LongArray(values) => {
                self.int(values.len().try_into().map_err(|_| Error::invalid("NBT 数组过长"))?);
                for value in values {
                    self.long(*value);
                }
            }
        }
        Ok(())
    }
    pub fn named(&mut self, name: &str, v: &Nbt) -> Result<()> {
        self.data.push(v.tag());
        if self.encoding != Encoding::NetworkBigEndian || v.tag() != 10 {
            self.string(name)?;
        }
        self.value(v)
    }
}
pub fn encode(name: &str, v: &Nbt, encoding: Encoding) -> Result<Vec<u8>> {
    let mut w = Writer::new(encoding);
    w.named(name, v)?;
    Ok(w.data)
}

pub fn encode_compound_with_position(
    name: &str,
    fields: &Compound,
    position: (i32, i32, i32),
    encoding: Encoding,
) -> Result<Vec<u8>> {
    let mut writer = Writer::new(encoding);
    writer.data.push(10);
    if encoding != Encoding::NetworkBigEndian {
        writer.string(name)?;
    }
    let coordinates = [("x", position.0), ("y", position.1), ("z", position.2)];
    let mut coordinate_index = 0;
    let mut fields = fields.iter().peekable();
    while coordinate_index < coordinates.len() || fields.peek().is_some() {
        let coordinate = coordinates.get(coordinate_index);
        let field = fields.peek();
        match (coordinate, field) {
            (Some((key, value)), Some((field_key, field_value)))
                if key.as_bytes() <= field_key.as_bytes() =>
            {
                writer.data.push(3);
                writer.string(key)?;
                writer.int(*value);
                coordinate_index += 1;
                if key.as_bytes() == field_key.as_bytes() {
                    fields.next();
                }
            }
            (_, Some((key, value))) => {
                writer.data.push(value.tag());
                writer.string(key)?;
                writer.value(value)?;
                fields.next();
            }
            (Some((key, value)), None) => {
                writer.data.push(3);
                writer.string(key)?;
                writer.int(*value);
                coordinate_index += 1;
            }
            (None, None) => break,
        }
    }
    writer.data.push(0);
    Ok(writer.data)
}

pub fn to_snbt(v: &Nbt) -> String {
    match v {
        Nbt::End => "END".into(),
        Nbt::Byte(v) => format!("{v}b"),
        Nbt::Short(v) => format!("{v}s"),
        Nbt::Int(v) => v.to_string(),
        Nbt::Long(v) => format!("{v}l"),
        Nbt::Float(v) => format!("{v}f"),
        Nbt::Double(v) => format!("{v}d"),
        Nbt::String(v) => serde_json::to_string(v).unwrap(),
        Nbt::RawString(v) => serde_json::to_string(&String::from_utf8_lossy(v)).unwrap(),
        Nbt::List(v) => format!("[{}]", v.iter().map(to_snbt).collect::<Vec<_>>().join(",")),
        Nbt::Compound(v) => format!(
            "{{{}}}",
            v.iter()
                .map(|(k, v)| format!("{}:{}", serde_json::to_string(k).unwrap(), to_snbt(v)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Nbt::RawCompound(v) => format!(
            "{{{}}}",
            v.iter()
                .map(|(k, v)| format!(
                    "{}:{}",
                    serde_json::to_string(&String::from_utf8_lossy(k)).unwrap(),
                    to_snbt(v)
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Nbt::ByteArray(v) => format!(
            "[B;{}]",
            v.iter()
                .map(|v| format!("{v}b"))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Nbt::IntArray(v) => format!(
            "[I;{}]",
            v.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Nbt::LongArray(v) => format!(
            "[L;{}]",
            v.iter()
                .map(|v| format!("{v}l"))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}
pub fn parse_snbt(text: &str) -> Result<Nbt> {
    let mut p = Snbt {
        chars: text.chars().collect(),
        pos: 0,
    };
    let value = p.value(0)?;
    p.ws();
    if p.pos != p.chars.len() {
        return Err(Error::invalid("SNBT 尾部内容无效"));
    }
    Ok(value)
}
struct Snbt {
    chars: Vec<char>,
    pos: usize,
}
impl Snbt {
    fn ws(&mut self) {
        while self.chars.get(self.pos).is_some_and(|c| c.is_whitespace()) {
            self.pos += 1;
        }
    }
    fn peek(&mut self) -> Option<char> {
        self.ws();
        self.chars.get(self.pos).copied()
    }
    fn eat(&mut self, c: char) -> Result<()> {
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(Error::invalid(format!("SNBT 期望 {c}")))
        }
    }
    fn text(&mut self, key: bool) -> Result<String> {
        self.ws();
        if let Some(q @ ('\'' | '"')) = self.peek() {
            self.pos += 1;
            let mut out = String::new();
            while let Some(c) = self.chars.get(self.pos).copied() {
                self.pos += 1;
                if c == q {
                    return Ok(out);
                }
                if c == '\\' {
                    let n = *self
                        .chars
                        .get(self.pos)
                        .ok_or_else(|| Error::invalid("SNBT 转义不完整"))?;
                    self.pos += 1;
                    out.push(match n {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        _ => n,
                    });
                } else {
                    out.push(c);
                }
            }
            return Err(Error::invalid("SNBT 字符串未闭合"));
        }
        let start = self.pos;
        while self.chars.get(self.pos).is_some_and(|c| {
            !c.is_whitespace() && !matches!(c, ',' | '}' | ']') && (!key || !matches!(c, ':' | '='))
        }) {
            self.pos += 1;
        }
        if start == self.pos {
            return Err(Error::invalid("SNBT 空标记"));
        }
        Ok(self.chars[start..self.pos].iter().collect())
    }
    fn value(&mut self, depth: usize) -> Result<Nbt> {
        if depth > 512 {
            return Err(Error::invalid("SNBT 嵌套过深"));
        }
        match self.peek() {
            Some('{') => {
                self.pos += 1;
                let mut m = BTreeMap::new();
                while self.peek() != Some('}') {
                    let k = self.text(true)?;
                    self.eat(':')?;
                    m.insert(k, self.value(depth + 1)?);
                    if self.peek() != Some(',') {
                        break;
                    }
                    self.pos += 1;
                }
                self.eat('}')?;
                Ok(Nbt::Compound(m))
            }
            Some('[') => {
                self.pos += 1;
                self.ws();
                let array = self.chars.get(self.pos + 1) == Some(&';');
                let t = if array {
                    let t = self.chars[self.pos].to_ascii_uppercase();
                    self.pos += 2;
                    Some(t)
                } else {
                    None
                };
                let mut v = vec![];
                while self.peek() != Some(']') {
                    v.push(self.value(depth + 1)?);
                    if self.peek() != Some(',') {
                        break;
                    }
                    self.pos += 1;
                }
                self.eat(']')?;
                let nums = || {
                    v.iter()
                        .map(|v| {
                            v.as_i64()
                                .ok_or_else(|| Error::invalid("SNBT 数组需要整数"))
                        })
                        .collect::<Result<Vec<_>>>()
                };
                match t {
                    Some('B') => Ok(Nbt::ByteArray(
                        nums()?.into_iter().map(|n| n as i8).collect(),
                    )),
                    Some('I') => Ok(Nbt::IntArray(
                        nums()?.into_iter().map(|n| n as i32).collect(),
                    )),
                    Some('L') => Ok(Nbt::LongArray(nums()?)),
                    Some(_) => Err(Error::invalid("SNBT 数组类型无效")),
                    None => Ok(Nbt::List(v)),
                }
            }
            Some('\'' | '"') => Ok(Nbt::String(self.text(false)?)),
            Some(_) => {
                let s = self.text(false)?;
                let l = s.to_ascii_lowercase();
                if l == "true" {
                    return Ok(Nbt::Byte(1));
                }
                if l == "false" {
                    return Ok(Nbt::Byte(0));
                }
                let (body, suffix) = l.split_at(l.len().saturating_sub(1));
                let v = match suffix {
                    "b" => body.parse().ok().map(Nbt::Byte),
                    "s" => body.parse().ok().map(Nbt::Short),
                    "l" => body.parse().ok().map(Nbt::Long),
                    "f" => body.parse().ok().map(Nbt::Float),
                    "d" => body.parse().ok().map(Nbt::Double),
                    _ => s
                        .parse()
                        .ok()
                        .map(Nbt::Int)
                        .or_else(|| s.parse().ok().map(Nbt::Double)),
                };
                Ok(v.unwrap_or(Nbt::String(s)))
            }
            None => Err(Error::invalid("SNBT 为空")),
        }
    }
}
pub fn decompress_gzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = vec![];
    flate2::read::GzDecoder::new(Cursor::new(bytes)).read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod position_encoding_tests {
    use super::*;

    #[test]
    fn positional_encoding_matches_overwriting_a_compound_clone() {
        let fields = Compound::from([
            ("aa_before".into(), Nbt::Byte(1)),
            (
                "nested".into(),
                Nbt::Compound(Compound::from([("value".into(), Nbt::Int(9))])),
            ),
            ("x".into(), Nbt::Short(12)),
            ("zz_after".into(), Nbt::Long(42)),
        ]);
        let mut expected = fields.clone();
        expected.insert("x".into(), Nbt::Int(-10));
        expected.insert("y".into(), Nbt::Int(20));
        expected.insert("z".into(), Nbt::Int(-30));
        for encoding in [
            Encoding::LittleEndian,
            Encoding::BigEndian,
            Encoding::NetworkLittleEndian,
            Encoding::NetworkBigEndian,
        ] {
            assert_eq!(
                encode("", &Nbt::Compound(expected.clone()), encoding).unwrap(),
                encode_compound_with_position("", &fields, (-10, 20, -30), encoding).unwrap()
            );
        }
    }
}
