use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
};
pub type Compound = BTreeMap<String, Nbt>;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum Nbt {
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
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}
impl Nbt {
    pub fn tag(&self) -> u8 {
        match self {
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
    pub fn as_list(&self) -> Option<&[Nbt]> {
        if let Self::List(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn scalar_string(&self) -> String {
        match self {
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
        Ok(self.take(n)?.to_vec())
    }

    pub fn tag_string_bytes(&mut self) -> Result<Vec<u8>> {
        self.string_bytes()
    }

    pub fn string(&mut self) -> Result<String> {
        String::from_utf8(self.string_bytes()?)
            .map_err(|_| Error::invalid("NBT 标签名称 UTF-8 无效"))
    }
    fn length(&mut self) -> Result<usize> {
        let n = self.int()?;
        if n < 0 {
            return Err(Error::invalid("NBT 数组长度为负数"));
        }
        let n = n as usize;
        if n > self.data.len().saturating_sub(self.pos) {
            return Err(Error::invalid("NBT 数组越界"));
        }
        Ok(n)
    }
    pub fn byte_array(&mut self) -> Result<Vec<u8>> {
        let n = self.int()?;
        if n < 0 {
            return Err(Error::invalid("NBT 数组长度为负数"));
        }
        let n = n as usize;
        let available = self.data.len().saturating_sub(self.pos);
        if n > available {
            self.pos += available;
            return Err(Error::invalid("NBT 数组越界"));
        }
        Ok(self.take(n)?.to_vec())
    }
    pub fn read_tag(&mut self) -> Result<(u8, String)> {
        let tag = self.tag_type()?;
        if tag > 12 {
            return Err(Error::invalid(format!("无效 NBT 标签 {tag}")));
        }
        if tag == 0 {
            return Ok((tag, String::new()));
        }
        Ok((tag, self.string()?))
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
    pub fn value(&mut self, tag: u8, depth: usize) -> Result<Nbt> {
        if depth > 512 {
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
            9 => {
                let t = self.byte()?;
                let n = self.length()?;
                let mut v = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    v.push(self.value(t, depth + 1)?);
                }
                Nbt::List(v)
            }
            10 => {
                let mut m = BTreeMap::new();
                loop {
                    let t = self.byte()?;
                    if t == 0 {
                        break;
                    }
                    let name = self.string()?;
                    m.insert(name, self.value(t, depth + 1)?);
                }
                Nbt::Compound(m)
            }
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
        Ok((name, self.value(tag, 0)?))
    }
}
pub fn decode(bytes: &[u8], encoding: Encoding) -> Result<(String, Nbt)> {
    if encoding == Encoding::NetworkLittleEndian && bytes.len() > 4 * 1024 * 1024 {
        return Err(Error::invalid("网络 NBT 超过 4MiB"));
    }
    Reader::new(bytes, encoding).named()
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
    pub fn value(&mut self, v: &Nbt) -> Result<()> {
        match v {
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
                if bytes.len() > u16::MAX as usize {
                    return Err(Error::invalid("NBT 字符串过长"));
                }
                if self.encoding.net() {
                    self.varuint(bytes.len() as u64);
                } else {
                    self.short(bytes.len() as i16);
                }
                self.data.extend(bytes);
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
