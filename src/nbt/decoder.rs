use super::{Encoding, Nbt, RawCompound};
use crate::{Error, Result};
use std::{any::type_name, collections::BTreeMap, io::Read};

const MAX_DEPTH: usize = 512;
const MAX_NETWORK_OFFSET: u64 = 4 * 1024 * 1024;

pub trait NbtDecode: Sized {
    const TAG: u8 = 255;
    const BYTE_SLICE: bool = false;

    fn go_type() -> String {
        go_type_name(type_name::<Self>())
    }

    fn from_list_byte(_value: u8) -> Option<Self> {
        None
    }

    fn decode_tag<R: Read>(&mut self, decoder: &mut Decoder<R>, tag: u8, name: &str) -> Result<()>;

    fn decode_compound_field<R: Read>(
        &mut self,
        _decoder: &mut Decoder<R>,
        _tag: u8,
        _raw_name: &str,
        _full_name: &str,
    ) -> Result<bool> {
        Ok(false)
    }
}

pub struct Decoder<R> {
    reader: R,
    pub encoding: Encoding,
    pub allow_zero: bool,
    offset: u64,
    depth: usize,
}

impl<R: Read> Decoder<R> {
    pub fn new(reader: R) -> Self {
        Self::with_encoding(reader, Encoding::NetworkLittleEndian)
    }

    pub fn with_encoding(reader: R, encoding: Encoding) -> Self {
        Self {
            reader,
            encoding,
            allow_zero: false,
            offset: 0,
            depth: 0,
        }
    }

    pub fn decode<T: NbtDecode>(&mut self, target: &mut T) -> Result<()> {
        let (tag, name) = self.read_tag_header()?;
        if tag == 0 {
            return if self.allow_zero {
                Ok(())
            } else {
                Err(self.unexpected_tag(tag))
            };
        }
        target.decode_tag(self, tag, &name)
    }

    pub fn get_offset(&self) -> u64 {
        self.offset
    }

    pub fn into_inner(self) -> R {
        self.reader
    }

    pub fn invalid_type(&self, tag: u8, name: &str, expected: &str) -> Error {
        Error::invalid(format!(
            "nbt: 在偏移 {} 的标签 '{}' 类型无效: 无法将 {} 反序列化为 {}",
            self.offset,
            name,
            tag_name(tag),
            go_type_name(expected)
        ))
    }

    pub fn unexpected_named_tag(&self, name: &str, tag: u8) -> Error {
        Error::invalid(format!(
            "nbt: 在偏移 {} 处遇到意外命名标签 '{}' 类型为 {}: 目标结构中不存在",
            self.offset,
            name,
            tag_name(tag)
        ))
    }

    pub fn unexpected_tag(&self, tag: u8) -> Error {
        Error::invalid(format!(
            "nbt: 在偏移 {} 处遇到意外标签 {}: 该标签在当前上下文无效",
            self.offset,
            tag_name(tag)
        ))
    }

    pub fn invalid_type_after_payload(&mut self, tag: u8, name: &str, expected: &str) -> Error {
        if tag == 0 {
            return self.unexpected_tag(tag);
        }
        if tag == 10 {
            self.depth += 1;
            return self.invalid_type(tag, name, expected);
        }
        if tag == 9 {
            self.depth += 1;
            let element_tag = match self.read_byte("Slice") {
                Ok(tag) => tag,
                Err(_) => return self.buffer_error("Slice"),
            };
            if element_tag > 12 {
                return self.unknown_tag("Slice", element_tag);
            }
            return self.invalid_type(tag, name, expected);
        }
        if tag > 12 {
            return self.unknown_tag("Match", tag);
        }
        if let Err(error) = self.read_tag_payload_to_discard(tag) {
            return error;
        }
        self.invalid_type(tag, name, expected)
    }

    pub fn invalid_struct_type(&mut self, tag: u8, name: &str, expected: &str) -> Error {
        if tag == 10 {
            self.depth += 1;
            return self.invalid_type(tag, name, expected);
        }
        if tag > 12 {
            return self.unknown_tag("Match", tag);
        }
        self.invalid_type_after_payload(tag, name, expected)
    }

    pub fn field_name(&self, parent: &str, child: &str) -> String {
        join_name(parent, child)
    }

    pub fn decode_value<T: NbtDecode>(
        &mut self,
        tag: u8,
        name: &str,
        target: &mut T,
    ) -> Result<()> {
        target.decode_tag(self, tag, name)
    }

    pub fn decode_compound<F>(&mut self, parent: &str, mut field: F) -> Result<()>
    where
        F: FnMut(&mut Self, u8, &str, &str) -> Result<()>,
    {
        self.depth += 1;
        loop {
            let (tag, name) = self.read_tag_header()?;
            if tag == 0 {
                self.depth -= 1;
                return Ok(());
            }
            if tag > 12 {
                return Err(self.unknown_tag("Struct", tag));
            }
            let full_name = join_name(parent, &name);
            field(self, tag, &name, &full_name)?;
        }
    }

    pub fn decode_compound_dynamic(
        &mut self,
        parent: &str,
        output: &mut RawCompound,
    ) -> Result<()> {
        self.depth += 1;
        loop {
            let (tag, raw_name) = self.read_tag_header_bytes()?;
            if tag == 0 {
                self.depth -= 1;
                return Ok(());
            }
            if tag > 12 {
                return Err(self.unknown_tag("Map", tag));
            }
            let name = String::from_utf8_lossy(&raw_name);
            let mut value = Nbt::End;
            value.decode_tag(self, tag, &join_name(parent, &name))?;
            output.insert(raw_name, value);
        }
    }

    fn decode_compound_map<T: NbtDecode + Default>(
        &mut self,
        parent: &str,
        output: &mut BTreeMap<String, T>,
    ) -> Result<()> {
        self.depth += 1;
        let mut decoded = BTreeMap::new();
        loop {
            let (tag, name) = self.read_tag_header()?;
            if tag == 0 {
                self.depth -= 1;
                *output = decoded;
                return Ok(());
            }
            if tag > 12 {
                return Err(self.unknown_tag("Map", tag));
            }
            let full_name = join_name(parent, &name);
            let mut value = T::default();
            value.decode_tag(self, tag, &full_name)?;
            decoded.insert(name, value);
        }
    }

    pub fn decode_list<T: NbtDecode + Default>(
        &mut self,
        parent: &str,
        output: &mut Vec<T>,
    ) -> Result<()> {
        self.depth += 1;
        let element_tag = self.read_byte("Slice")?;
        if element_tag > 12 {
            return Err(self.unknown_tag("Slice", element_tag));
        }
        if element_tag == 1 && !T::BYTE_SLICE && T::TAG != 255 {
            let length = self.read_count("ByteSlice")?;
            self.read_bytes(length, "ByteSlice")?;
            if length == 0 {
                *output = Vec::new();
                self.depth -= 1;
                return Ok(());
            }
            return Err(self.invalid_type(1, parent, type_name::<T>()));
        }
        if element_tag == 1 && T::BYTE_SLICE {
            let length = self.read_count("ByteSlice")?;
            let bytes = self.read_bytes(length, "ByteSlice")?;
            let values = bytes
                .into_iter()
                .map(T::from_list_byte)
                .collect::<Option<Vec<_>>>();
            if let Some(values) = values {
                *output = values;
                self.depth -= 1;
                return Ok(());
            }
            return Err(self.invalid_type(1, parent, type_name::<T>()));
        }
        if element_tag == 3 && T::TAG != 3 && T::TAG != 255 {
            let length = self.read_count("Int32Slice")?;
            for _ in 0..length {
                self.read_i32()
                    .map_err(|_| self.buffer_error("Int32Slice"))?;
            }
            return Err(self.invalid_type(3, parent, type_name::<T>()));
        }
        if element_tag == 4 && T::TAG != 4 && T::TAG != 255 {
            let length = self.read_count("Int64Slice")?;
            for _ in 0..length {
                self.read_i64()
                    .map_err(|_| self.buffer_error("Int64Slice"))?;
            }
            return Err(self.invalid_type(4, parent, type_name::<T>()));
        }
        let length = self.read_count("Int32")?;
        let mut values = Vec::with_capacity(length.min(4096));
        for index in 0..length {
            if element_tag == 0 {
                if T::TAG == 255 {
                    let mut value = T::default();
                    value.decode_tag(self, 0, "")?;
                    values.push(value);
                    continue;
                }
                return Err(self.invalid_type(0, parent, type_name::<T>()));
            }
            let mut value = T::default();
            let name = format!("{parent}[{index}]");
            value.decode_tag(self, element_tag, &name)?;
            values.push(value);
        }
        *output = values;
        self.depth -= 1;
        Ok(())
    }

    fn read_tag_header(&mut self) -> Result<(u8, String)> {
        let (tag, bytes) = self.read_tag_header_bytes()?;
        let name =
            String::from_utf8(bytes).map_err(|_| Error::invalid("NBT 标签名称 UTF-8 无效"))?;
        Ok((tag, name))
    }

    fn read_tag_header_bytes(&mut self) -> Result<(u8, Vec<u8>)> {
        if self.depth >= MAX_DEPTH {
            return Err(Error::invalid("nbt: 达到最大嵌套深度 512"));
        }
        if self.encoding == Encoding::NetworkLittleEndian && self.offset >= MAX_NETWORK_OFFSET {
            return Err(Error::invalid(
                "nbt: NetworkLittleEndian 格式读取字节数限制 4194304 已耗尽",
            ));
        }
        let tag = self.read_byte("ReadTag")?;
        if tag == 0 {
            return Ok((tag, Vec::new()));
        }
        if self.encoding == Encoding::NetworkBigEndian && tag == 10 && self.depth == 0 {
            return Ok((tag, Vec::new()));
        }
        Ok((tag, self.read_string_bytes()?))
    }

    fn read_exact_op(&mut self, output: &mut [u8], op: &str) -> Result<()> {
        let mut filled = 0;
        while filled < output.len() {
            match self.reader.read(&mut output[filled..]) {
                Ok(0) => return Err(self.buffer_error(op)),
                Ok(read) => {
                    filled += read;
                    self.offset += read as u64;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(self.buffer_error(op)),
            }
        }
        Ok(())
    }

    fn read_byte(&mut self, op: &str) -> Result<u8> {
        let mut byte = [0];
        self.read_exact_op(&mut byte, op)?;
        Ok(byte[0])
    }

    fn read_bytes(&mut self, count: usize, op: &str) -> Result<Vec<u8>> {
        let mut bytes = vec![0; count];
        self.read_exact_op(&mut bytes, op)?;
        Ok(bytes)
    }

    fn read_i16(&mut self) -> Result<i16> {
        let mut bytes = [0; 2];
        self.read_exact_op(&mut bytes, "Int16")?;
        Ok(if self.encoding.le() {
            i16::from_le_bytes(bytes)
        } else {
            i16::from_be_bytes(bytes)
        })
    }

    fn read_i32(&mut self) -> Result<i32> {
        if self.encoding.net() {
            let start = self.offset;
            let mut value = 0u32;
            for index in 0..5 {
                let byte = self.read_byte("Int32")?;
                value |= u32::from(byte & 0x7f).wrapping_shl(index * 7);
                if byte & 0x80 == 0 {
                    let decoded = (value >> 1) as i32;
                    return Ok(if value & 1 == 1 { !decoded } else { decoded });
                }
            }
            return Err(Error::invalid(format!(
                "nbt: varint 在读取 5 字节后未终止, 偏移 {}",
                start + 5
            )));
        }
        let mut bytes = [0; 4];
        self.read_exact_op(&mut bytes, "Int32")?;
        Ok(if self.encoding.le() {
            i32::from_le_bytes(bytes)
        } else {
            i32::from_be_bytes(bytes)
        })
    }

    fn read_i64(&mut self) -> Result<i64> {
        if self.encoding.net() {
            let start = self.offset;
            let mut value = 0u64;
            for index in 0..10 {
                let byte = self.read_byte("Int64")?;
                value |= u64::from(byte & 0x7f).wrapping_shl(index * 7);
                if byte & 0x80 == 0 {
                    let decoded = (value >> 1) as i64;
                    return Ok(if value & 1 == 1 { !decoded } else { decoded });
                }
            }
            return Err(Error::invalid(format!(
                "nbt: varint 在读取 10 字节后未终止, 偏移 {}",
                start + 10
            )));
        }
        let mut bytes = [0; 8];
        self.read_exact_op(&mut bytes, "Int64")?;
        Ok(if self.encoding.le() {
            i64::from_le_bytes(bytes)
        } else {
            i64::from_be_bytes(bytes)
        })
    }

    fn read_f32(&mut self) -> Result<f32> {
        let mut bytes = [0; 4];
        self.read_exact_op(&mut bytes, "Float32")?;
        Ok(if self.encoding.le() {
            f32::from_le_bytes(bytes)
        } else {
            f32::from_be_bytes(bytes)
        })
    }

    fn read_f64(&mut self) -> Result<f64> {
        let mut bytes = [0; 8];
        self.read_exact_op(&mut bytes, "Float64")?;
        Ok(if self.encoding.le() {
            f64::from_le_bytes(bytes)
        } else {
            f64::from_be_bytes(bytes)
        })
    }

    fn read_string_bytes(&mut self) -> Result<Vec<u8>> {
        let length = if self.encoding.net() {
            let start = self.offset;
            let mut value = 0u32;
            for index in 0..5 {
                let byte = self.read_byte("StringLength")?;
                value |= u32::from(byte & 0x7f).wrapping_shl(index * 7);
                if byte & 0x80 == 0 {
                    if value > i16::MAX as u32 {
                        return Err(Error::invalid(format!(
                            "nbt: 偏移 {} 的字符串无效: 字符串长度超过最大限制（长度={}）",
                            self.offset, value
                        )));
                    }
                    let _ = start;
                    return self.read_bytes(value as usize, "String");
                }
            }
            return Err(Error::invalid(format!(
                "nbt: varint 在读取 5 字节后未终止, 偏移 {}",
                start + 5
            )));
        } else {
            self.read_i16()
                .map_err(|_| self.buffer_error("String"))? as u16 as usize
        };
        self.read_bytes(length, "String")
    }

    fn read_count(&mut self, op: &str) -> Result<usize> {
        let count = self.read_i32().map_err(|_| self.buffer_error(op))?;
        if count < 0 {
            panic!("runtime error: makeslice: len out of range");
        }
        Ok(count as usize)
    }

    fn buffer_error(&self, op: &str) -> Error {
        Error::invalid(format!("nbt: 操作 '{op}' 期间缓冲区意外结束"))
    }

    fn unknown_tag(&self, op: &str, tag: u8) -> Error {
        Error::invalid(format!(
            "nbt: 在偏移 {} 处的操作 '{}' 发现未知标签 '{}'",
            self.offset, op, tag
        ))
    }
}

fn join_name(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.to_owned()
    } else {
        format!("{parent}.{child}")
    }
}

fn tag_name(tag: u8) -> &'static str {
    match tag {
        0 => "TAG_End",
        1 => "TAG_Byte",
        2 => "TAG_Short",
        3 => "TAG_Int",
        4 => "TAG_Long",
        5 => "TAG_Float",
        6 => "TAG_Double",
        7 => "TAG_ByteArray",
        8 => "TAG_String",
        9 => "TAG_List",
        10 => "TAG_Compound",
        11 => "TAG_IntArray",
        12 => "TAG_LongArray",
        _ => "unknown",
    }
}

fn go_type_name(name: &str) -> String {
    match name {
        "u8" => "uint8".into(),
        "i16" => "int16".into(),
        "i32" => "int32".into(),
        "i64" => "int64".into(),
        "f32" => "float32".into(),
        "f64" => "float64".into(),
        "bool" => "bool".into(),
        "alloc::string::String" | "std::string::String" => "string".into(),
        other => other.to_owned(),
    }
}

macro_rules! scalar_decode {
    ($type:ty, $tag:literal, $read:ident, $expected:literal) => {
        impl NbtDecode for $type {
            const TAG: u8 = $tag;

            fn decode_tag<R: Read>(
                &mut self,
                decoder: &mut Decoder<R>,
                tag: u8,
                name: &str,
            ) -> Result<()> {
                if tag == $tag {
                    let value = decoder.$read()?;
                    *self = value as $type;
                    Ok(())
                } else if tag == 0 {
                    Err(decoder.unexpected_tag(tag))
                } else {
                    Err(decoder.invalid_type_after_payload(tag, name, $expected))
                }
            }
        }
    };
}

scalar_decode!(i16, 2, read_i16, "int16");
scalar_decode!(i32, 3, read_i32, "int32");
scalar_decode!(i64, 4, read_i64, "int64");
scalar_decode!(f32, 5, read_f32, "float32");
scalar_decode!(f64, 6, read_f64, "float64");

impl NbtDecode for u8 {
    const TAG: u8 = 1;
    const BYTE_SLICE: bool = true;
    fn from_list_byte(value: u8) -> Option<Self> {
        Some(value)
    }
    fn decode_tag<R: Read>(&mut self, d: &mut Decoder<R>, tag: u8, name: &str) -> Result<()> {
        if tag == 1 {
            *self = d.read_byte("Byte")?;
            Ok(())
        } else {
            if tag != 0 {
                return Err(d.invalid_type_after_payload(tag, name, "uint8"));
            }
            Err(d.invalid_type(tag, name, "uint8"))
        }
    }
}

impl NbtDecode for bool {
    const TAG: u8 = 1;
    fn decode_tag<R: Read>(&mut self, d: &mut Decoder<R>, tag: u8, name: &str) -> Result<()> {
        if tag == 1 {
            *self = d.read_byte("Byte")? == 1;
            Ok(())
        } else {
            if tag != 0 {
                return Err(d.invalid_type_after_payload(tag, name, "bool"));
            }
            Err(d.invalid_type(tag, name, "bool"))
        }
    }
}

impl NbtDecode for String {
    const TAG: u8 = 8;
    fn decode_tag<R: Read>(&mut self, d: &mut Decoder<R>, tag: u8, name: &str) -> Result<()> {
        if tag == 8 {
            let bytes = d.read_string_bytes()?;
            *self = String::from_utf8_lossy(&bytes).into_owned();
            Ok(())
        } else {
            if tag != 0 {
                return Err(d.invalid_type_after_payload(tag, name, "string"));
            }
            Err(d.invalid_type(tag, name, "string"))
        }
    }
}

impl NbtDecode for Nbt {
    const BYTE_SLICE: bool = true;
    fn from_list_byte(value: u8) -> Option<Self> {
        Some(Nbt::Byte(value as i8))
    }
    fn decode_tag<R: Read>(&mut self, d: &mut Decoder<R>, tag: u8, name: &str) -> Result<()> {
        match tag {
            0 => *self = Nbt::End,
            1 => *self = Nbt::Byte(d.read_byte("Byte")? as i8),
            2 => *self = Nbt::Short(d.read_i16()?),
            3 => *self = Nbt::Int(d.read_i32()?),
            4 => *self = Nbt::Long(d.read_i64()?),
            5 => *self = Nbt::Float(d.read_f32()?),
            6 => *self = Nbt::Double(d.read_f64()?),
            7 => {
                let length = d.read_count("ByteArray")?;
                let bytes = d.read_bytes(length, "ByteArray")?;
                *self = Nbt::ByteArray(bytes.into_iter().map(|byte| byte as i8).collect());
            }
            8 => {
                let bytes = d.read_string_bytes()?;
                *self = String::from_utf8(bytes.clone())
                    .map(Nbt::String)
                    .unwrap_or(Nbt::RawString(bytes));
            }
            9 => {
                let mut values = Vec::new();
                d.decode_list(name, &mut values)?;
                *self = Nbt::List(values);
            }
            10 => {
                let mut values = RawCompound::new();
                d.decode_compound_dynamic(name, &mut values)?;
                *self = if values.keys().all(|key| std::str::from_utf8(key).is_ok()) {
                    Nbt::Compound(
                        values
                            .into_iter()
                            .map(|(key, value)| (String::from_utf8(key).unwrap(), value))
                            .collect(),
                    )
                } else {
                    Nbt::RawCompound(values)
                };
            }
            11 => {
                let length = d.read_count("Int32Slice")?;
                let mut values = Vec::with_capacity(length.min(4096));
                for _ in 0..length {
                    values.push(d.read_i32().map_err(|_| d.buffer_error("Int32Slice"))?);
                }
                *self = Nbt::IntArray(values);
            }
            12 => {
                let length = d.read_count("Int64Slice")?;
                let mut values = Vec::with_capacity(length.min(4096));
                for _ in 0..length {
                    values.push(d.read_i64().map_err(|_| d.buffer_error("Int64Slice"))?);
                }
                *self = Nbt::LongArray(values);
            }
            _ => return Err(d.unknown_tag("Match", tag)),
        }
        Ok(())
    }
}

impl NbtDecode for super::RawString {
    const TAG: u8 = 8;
    fn decode_tag<R: Read>(&mut self, d: &mut Decoder<R>, tag: u8, name: &str) -> Result<()> {
        if tag == 8 {
            self.0 = d.read_string_bytes()?;
            Ok(())
        } else {
            Err(d.invalid_type_after_payload(tag, name, "string"))
        }
    }
}

impl<T: NbtDecode + Default> NbtDecode for Vec<T> {
    const TAG: u8 = 9;
    const BYTE_SLICE: bool = T::BYTE_SLICE;
    fn decode_tag<R: Read>(&mut self, d: &mut Decoder<R>, tag: u8, name: &str) -> Result<()> {
        if tag == 9 {
            d.decode_list(name, self)
        } else {
            if tag == 7 {
                let length = d.read_count("ByteArray")?;
                let _ = d.read_bytes(length, "ByteArray")?;
            } else if tag == 11 || tag == 12 {
                let count = d.read_count(if tag == 11 {
                    "Int32Slice"
                } else {
                    "Int64Slice"
                })?;
                for _ in 0..count {
                    if tag == 11 {
                        d.read_i32()?;
                    } else {
                        d.read_i64()?;
                    }
                }
            } else if tag == 10 {
                d.depth += 1;
            } else if tag != 0 {
                d.read_tag_payload_to_discard(tag)?;
            }
            if tag == 0 {
                Err(d.unexpected_tag(tag))
            } else {
                Err(d.invalid_type(tag, name, "slice"))
            }
        }
    }
}

impl<T: NbtDecode + Default> NbtDecode for BTreeMap<String, T> {
    const TAG: u8 = 10;
    fn decode_tag<R: Read>(&mut self, d: &mut Decoder<R>, tag: u8, name: &str) -> Result<()> {
        if tag != 10 {
            return Err(d.invalid_type_after_payload(tag, name, "map[string]interface {}"));
        }
        d.decode_compound_map(name, self)
    }
}

impl<T: NbtDecode + Default> NbtDecode for BTreeMap<Vec<u8>, T> {
    const TAG: u8 = 10;
    fn decode_tag<R: Read>(&mut self, d: &mut Decoder<R>, tag: u8, name: &str) -> Result<()> {
        if tag != 10 {
            return Err(d.invalid_type_after_payload(tag, name, "map[string]interface {}"));
        }
        d.depth += 1;
        let mut decoded = BTreeMap::new();
        loop {
            let (tag, key) = d.read_tag_header_bytes()?;
            if tag == 0 {
                d.depth -= 1;
                *self = decoded;
                return Ok(());
            }
            if tag > 12 {
                return Err(d.unknown_tag("Map", tag));
            }
            let field = String::from_utf8_lossy(&key);
            let mut value = T::default();
            value.decode_tag(d, tag, &field)?;
            decoded.insert(key, value);
        }
    }
}

impl<const N: usize> NbtDecode for [u8; N] {
    const TAG: u8 = 7;
    fn go_type() -> String {
        format!("[{N}]uint8")
    }
    fn decode_tag<R: Read>(&mut self, d: &mut Decoder<R>, tag: u8, name: &str) -> Result<()> {
        if tag != 7 {
            return Err(d.invalid_type_after_payload(tag, name, &Self::go_type()));
        }
        let length = d.read_count("ByteArray")?;
        let bytes = d.read_bytes(length, "ByteArray")?;
        if length != N {
            return Err(d.array_size_error("ByteArray", N, length));
        }
        self.copy_from_slice(&bytes);
        Ok(())
    }
}

impl<const N: usize> NbtDecode for [i32; N] {
    const TAG: u8 = 11;
    fn go_type() -> String {
        format!("[{N}]int32")
    }
    fn decode_tag<R: Read>(&mut self, d: &mut Decoder<R>, tag: u8, name: &str) -> Result<()> {
        if tag != 11 {
            return Err(d.invalid_type_after_payload(tag, name, &Self::go_type()));
        }
        let length = d.read_count("Int32Slice")?;
        let mut values = Vec::with_capacity(length.min(4096));
        for _ in 0..length {
            values.push(d.read_i32().map_err(|_| d.buffer_error("Int32Slice"))?);
        }
        if length != N {
            return Err(d.array_size_error("Int32Array", N, length));
        }
        self.copy_from_slice(&values);
        Ok(())
    }
}

impl<const N: usize> NbtDecode for [i64; N] {
    const TAG: u8 = 12;
    fn go_type() -> String {
        format!("[{N}]int64")
    }
    fn decode_tag<R: Read>(&mut self, d: &mut Decoder<R>, tag: u8, name: &str) -> Result<()> {
        if tag != 12 {
            return Err(d.invalid_type_after_payload(tag, name, &Self::go_type()));
        }
        let length = d.read_count("Int64Slice")?;
        let mut values = Vec::with_capacity(length.min(4096));
        for _ in 0..length {
            values.push(d.read_i64().map_err(|_| d.buffer_error("Int64Slice"))?);
        }
        if length != N {
            return Err(d.array_size_error("Int64Array", N, length));
        }
        self.copy_from_slice(&values);
        Ok(())
    }
}

impl<R: Read> Decoder<R> {
    fn array_size_error(&self, op: &str, expected: usize, actual: usize) -> Error {
        Error::invalid(format!(
            "nbt: 在偏移 {} 的操作 '{}' 处数组大小不匹配: 期望 {}, 实际 {}",
            self.offset, op, expected, actual
        ))
    }

    fn read_tag_payload_to_discard(&mut self, tag: u8) -> Result<()> {
        match tag {
            0 => Ok(()),
            1 => self.read_byte("Byte").map(|_| ()),
            2 => self.read_i16().map(|_| ()),
            3 => self.read_i32().map(|_| ()),
            4 => self.read_i64().map(|_| ()),
            5 => self.read_f32().map(|_| ()),
            6 => self.read_f64().map(|_| ()),
            7 => {
                let n = self.read_count("ByteArray")?;
                self.read_bytes(n, "ByteArray").map(|_| ())
            }
            8 => self.read_string_bytes().map(|_| ()),
            9 => {
                let elem = self.read_byte("Slice")?;
                if elem > 12 {
                    return Err(self.unknown_tag("Slice", elem));
                }
                let count = self.read_count("Slice")?;
                for _ in 0..count {
                    self.read_tag_payload_to_discard(elem)?;
                }
                Ok(())
            }
            10 => {
                self.depth += 1;
                loop {
                    let (child, _) = self.read_tag_header()?;
                    if child == 0 {
                        self.depth -= 1;
                        return Ok(());
                    }
                    if child > 12 {
                        return Err(self.unknown_tag("Map", child));
                    }
                    self.read_tag_payload_to_discard(child)?;
                }
            }
            11 | 12 => {
                let n = self.read_count(if tag == 11 {
                    "Int32Slice"
                } else {
                    "Int64Slice"
                })?;
                for _ in 0..n {
                    if tag == 11 {
                        self.read_i32()?;
                    } else {
                        self.read_i64()?;
                    }
                }
                Ok(())
            }
            _ => Err(self.unknown_tag("Match", tag)),
        }
    }
}

pub fn unmarshal<T: NbtDecode>(data: &[u8], target: &mut T) -> Result<()> {
    unmarshal_with_encoding(data, target, Encoding::NetworkLittleEndian)
}

pub fn unmarshal_with_encoding<T: NbtDecode>(
    data: &[u8],
    target: &mut T,
    encoding: Encoding,
) -> Result<()> {
    let mut decoder = Decoder::with_encoding(data, encoding);
    decoder.decode(target)
}
