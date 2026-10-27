//! BDX 指令流与离线签名校验。数值操作数遵循 BDX 大端序，方块 NBT 为小端序。
use crate::nbt::{Compound, Encoding, Nbt};
use crate::{Error, Result};
use rsa::{Pkcs1v15Sign, RsaPublicKey, pkcs1::DecodeRsaPublicKey};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlockReference {
    Legacy { name: u16, data: u16 },
    States { name: u16, states: u16 },
    InlineStates { name: u16, states: String },
    Runtime(u32),
    CommandBlock { data: u16 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Constant(String),
    Move {
        axis: usize,
        delta: i32,
    },
    Place {
        block: BlockReference,
        nbt: Option<Compound>,
    },
    SetCommandBlock(Compound),
    RuntimePool(u8),
    Debug(Vec<u8>),
    NoOperation,
    Terminate,
}

pub struct CommandReader<'a> {
    data: &'a [u8],
    pub position: usize,
}

impl<'a> CommandReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(count)
            .ok_or_else(|| Error::invalid("BDX 长度溢出"))?;
        let bytes = self
            .data
            .get(self.position..end)
            .ok_or_else(|| Error::invalid("BDX 指令截断"))?;
        self.position = end;
        Ok(bytes)
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn short(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn int(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn string(&mut self) -> Result<String> {
        let count = self.data[self.position..]
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| Error::invalid("BDX 字符串未终止"))?;
        let text = String::from_utf8(self.take(count)?.to_vec())
            .map_err(|e| Error::invalid(e.to_string()))?;
        self.take(1)?;
        Ok(text)
    }

    fn command_nbt(&mut self) -> Result<Compound> {
        let mode = self.int()?;
        let mut result = Compound::new();
        result.insert("id".into(), Nbt::String("CommandBlock".into()));
        result.insert("LPCommandMode".into(), Nbt::Int(mode as i32));
        for name in ["Command", "CustomName", "LastOutput"] {
            result.insert(name.into(), Nbt::String(self.string()?));
        }
        result.insert("TickDelay".into(), Nbt::Int(self.int()? as i32));
        for name in ["ExecuteOnFirstTick", "TrackOutput", "conditionalMode"] {
            result.insert(name.into(), Nbt::Byte((self.byte()? != 0) as i8));
        }
        result.insert("auto".into(), Nbt::Byte((self.byte()? == 0) as i8));
        Ok(result)
    }

    fn chest_nbt(&mut self) -> Result<Compound> {
        let count = self.byte()?;
        let mut items = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let mut item = Compound::new();
            item.insert("Name".into(), Nbt::String(self.string()?));
            item.insert("Count".into(), Nbt::Byte(self.byte()? as i8));
            item.insert("Damage".into(), Nbt::Short(self.short()? as i16));
            item.insert("Slot".into(), Nbt::Byte(self.byte()? as i8));
            items.push(Nbt::Compound(item));
        }
        Ok([(String::from("Items"), Nbt::List(items))]
            .into_iter()
            .collect())
    }

    fn block_reference(&mut self, opcode: u8) -> Result<BlockReference> {
        Ok(match opcode {
            5 | 41 => BlockReference::States {
                name: self.short()?,
                states: self.short()?,
            },
            7 | 27 | 40 => BlockReference::Legacy {
                name: self.short()?,
                data: self.short()?,
            },
            13 => BlockReference::InlineStates {
                name: self.short()?,
                states: self.string()?,
            },
            32 | 34 | 37 => BlockReference::Runtime(u32::from(self.short()?)),
            33 | 35 | 38 => BlockReference::Runtime(self.int()?),
            36 => BlockReference::CommandBlock {
                data: self.short()?,
            },
            _ => return Err(Error::invalid(format!("不是放置指令: {opcode}"))),
        })
    }

    /// 读取一条指令；遇到未知 opcode 或截断时返回错误，不尝试越界恢复。
    pub fn read(&mut self) -> Result<Option<Command>> {
        if self.position == self.data.len() {
            return Ok(None);
        }
        let opcode = self.byte()?;
        let command = match opcode {
            1 => Command::Constant(self.string()?),
            6 => Command::Move {
                axis: 2,
                delta: i32::from(self.short()? as i16),
            },
            8 | 18 => Command::Move { axis: 2, delta: 1 },
            9 => Command::NoOperation,
            12 => Command::Move {
                axis: 2,
                delta: self.int()? as i32,
            },
            14..=19 => Command::Move {
                axis: usize::from((opcode - 14) / 2),
                delta: if opcode % 2 == 0 { 1 } else { -1 },
            },
            20..=25 => Command::Move {
                axis: usize::from((opcode - 20) / 2),
                delta: if opcode % 2 == 0 {
                    i32::from(self.short()? as i16)
                } else {
                    self.int()? as i32
                },
            },
            28..=30 => Command::Move {
                axis: usize::from(opcode - 28),
                delta: i32::from(self.byte()? as i8),
            },
            26 => Command::SetCommandBlock(self.command_nbt()?),
            31 => Command::RuntimePool(self.byte()?),
            39 => {
                let length = self.int()? as usize;
                Command::Debug(self.take(length)?.to_vec())
            }
            5 | 7 | 13 | 27 | 32..=38 | 40 | 41 => {
                let block = self.block_reference(opcode)?;
                let nbt = match opcode {
                    27 | 34..=36 => Some(self.command_nbt()?),
                    37 | 38 | 40 => Some(self.chest_nbt()?),
                    41 => {
                        // Go 原实现保留两个额外字节，随后为完整 little-endian NBT 根。
                        self.take(2)?;
                        let mut reader = crate::nbt::Reader::new(
                            &self.data[self.position..],
                            Encoding::LittleEndian,
                        );
                        let (_, root) = reader.named()?;
                        self.position += reader.pos;
                        Some(
                            root.as_compound()
                                .cloned()
                                .ok_or_else(|| Error::invalid("BDX 方块 NBT 根必须为 Compound"))?,
                        )
                    }
                    _ => None,
                };
                Command::Place { block, nbt }
            }
            88 => Command::Terminate,
            _ => return Err(Error::invalid(format!("未知 BDX opcode: {opcode}"))),
        };
        Ok(Some(command))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureStatus {
    pub signed: bool,
    pub corrupted: bool,
    pub username: Option<String>,
    pub legacy: bool,
}

fn write_string(output: &mut Vec<u8>, value: &str) -> Result<()> {
    if value.contains('\0') {
        return Err(Error::invalid("BDX 字符串包含 NUL"));
    }
    output.extend_from_slice(value.as_bytes());
    output.push(0);
    Ok(())
}

fn write_command_nbt(output: &mut Vec<u8>, nbt: &Compound) -> Result<()> {
    let integer = |key: &str| nbt.get(key).and_then(Nbt::as_i64).unwrap_or(0);
    output.extend_from_slice(&(integer("LPCommandMode") as u32).to_be_bytes());
    for name in ["Command", "CustomName", "LastOutput"] {
        write_string(output, nbt.get(name).and_then(Nbt::as_str).unwrap_or(""))?;
    }
    output.extend_from_slice(&(integer("TickDelay") as i32).to_be_bytes());
    for name in ["ExecuteOnFirstTick", "TrackOutput", "conditionalMode"] {
        output.push((integer(name) != 0) as u8);
    }
    output.push((integer("auto") == 0) as u8);
    Ok(())
}

fn write_chest_nbt(output: &mut Vec<u8>, nbt: &Compound) -> Result<()> {
    let items = nbt.get("Items").and_then(Nbt::as_list).unwrap_or(&[]);
    output.push(u8::try_from(items.len()).map_err(|_| Error::invalid("BDX 容器超过255个槽"))?);
    for item in items {
        let item = item
            .as_compound()
            .ok_or_else(|| Error::invalid("BDX 物品不是 Compound"))?;
        let integer = |key: &str| item.get(key).and_then(Nbt::as_i64).unwrap_or(0);
        write_string(output, item.get("Name").and_then(Nbt::as_str).unwrap_or(""))?;
        output.push(integer("Count") as u8);
        output.extend_from_slice(&(integer("Damage") as u16).to_be_bytes());
        output.push(integer("Slot") as u8);
    }
    Ok(())
}

/// 写入一条标准 BDX 指令；移动指令自动选择等价的32位编码。
pub fn write_command(output: &mut Vec<u8>, command: &Command) -> Result<()> {
    match command {
        Command::Constant(text) => {
            output.push(1);
            write_string(output, text)?;
        }
        Command::Move { axis, delta } => {
            let opcode = [21, 23, 25]
                .get(*axis)
                .ok_or_else(|| Error::invalid("BDX 移动轴错误"))?;
            output.push(*opcode);
            output.extend_from_slice(&delta.to_be_bytes());
        }
        Command::RuntimePool(pool) => output.extend_from_slice(&[31, *pool]),
        Command::Debug(bytes) => {
            let length =
                u32::try_from(bytes.len()).map_err(|_| Error::invalid("BDX 调试数据过长"))?;
            output.push(39);
            output.extend_from_slice(&length.to_be_bytes());
            output.extend_from_slice(bytes);
        }
        Command::NoOperation => output.push(9),
        Command::Terminate => output.push(88),
        Command::SetCommandBlock(nbt) => {
            output.push(26);
            write_command_nbt(output, nbt)?;
        }
        Command::Place { block, nbt } => write_placement(output, block, nbt.as_ref())?,
    }
    Ok(())
}

fn write_placement(
    output: &mut Vec<u8>,
    block: &BlockReference,
    nbt: Option<&Compound>,
) -> Result<()> {
    let has_command = nbt.is_some_and(|nbt| nbt.contains_key("Command"));
    let has_items = nbt.is_some_and(|nbt| nbt.contains_key("Items"));
    match block {
        BlockReference::States { name, states } => {
            output.push(if nbt.is_some() { 41 } else { 5 });
            output.extend_from_slice(&name.to_be_bytes());
            output.extend_from_slice(&states.to_be_bytes());
            if let Some(nbt) = nbt {
                output.extend_from_slice(&states.to_be_bytes());
                output.extend(crate::nbt::encode(
                    "",
                    &Nbt::Compound(nbt.clone()),
                    Encoding::LittleEndian,
                )?);
            }
            return Ok(());
        }
        BlockReference::Legacy { name, data } => {
            if nbt.is_some() && !has_command && !has_items {
                return Err(Error::invalid("带任意 NBT 的 BDX 放置需要状态常量引用"));
            }
            output.push(if has_command {
                27
            } else if has_items {
                40
            } else {
                7
            });
            output.extend_from_slice(&name.to_be_bytes());
            output.extend_from_slice(&data.to_be_bytes());
        }
        BlockReference::InlineStates { name, states } => {
            if nbt.is_some() {
                return Err(Error::invalid("废弃 BDX 状态字符串指令不支持 NBT"));
            }
            output.push(13);
            output.extend_from_slice(&name.to_be_bytes());
            return write_string(output, states);
        }
        BlockReference::Runtime(id) => {
            if nbt.is_some() && !has_command && !has_items {
                return Err(Error::invalid("BDX runtime 指令只支持命令或容器 NBT"));
            }
            let wide = *id > u32::from(u16::MAX);
            let opcode = match (has_command, has_items, wide) {
                (true, _, false) => 34,
                (true, _, true) => 35,
                (_, true, false) => 37,
                (_, true, true) => 38,
                (_, _, false) => 32,
                (_, _, true) => 33,
            };
            output.push(opcode);
            if wide {
                output.extend_from_slice(&id.to_be_bytes());
            } else {
                output.extend_from_slice(&(*id as u16).to_be_bytes());
            }
        }
        BlockReference::CommandBlock { data } => {
            if !has_command {
                return Err(Error::invalid("BDX 命令方块指令缺少 Command"));
            }
            output.push(36);
            output.extend_from_slice(&data.to_be_bytes());
        }
    }
    if let Some(nbt) = nbt {
        if has_command {
            write_command_nbt(output, nbt)?;
        } else {
            write_chest_nbt(output, nbt)?;
        }
    }
    Ok(())
}

const SERVER_KEY: &str = "-----BEGIN RSA PUBLIC KEY-----\nMIIBCgKCAQEAzOoZfky1sYQXkTXWuYqf7HZ+tDSLyyuYOvyqt/dO4xahyNqvXcL5\n1A+eNFhsk6S5u84RuwsUk7oeNDpg/I0hbiRuJwCxFPJKNxDdj5Q5P5O0NTLR0TAT\nNBP7AjX6+XtNB/J6cV3fPcduqBbN4NjkNZxP4I1lgbupIR2lMKU9lXEn58nFSqSZ\nvG4BZfYLKUiu89IHaZOG5wgyDwwQrejxqkLUftmXibUO4s4gf8qAiLp3ukeIPYRj\nwGhGNlUfdU0foCxf2QwAoBV2xREL8/Sx1AIvmoVUg1SqCiIVMvbBkDoFfkzPZCgC\nLtmbkmqZJnpoBVHcBhBdUYsfyM6QwtWBNQIDAQAB\n-----END RSA PUBLIC KEY-----";

/// 校验解压后的完整 BDX 数据流，不发起网络请求。
pub fn verify_stream(bytes: &[u8]) -> Result<SignatureStatus> {
    let mut status = SignatureStatus {
        signed: false,
        corrupted: false,
        username: None,
        legacy: false,
    };
    if bytes.last().copied() != Some(90) {
        return Ok(status);
    }
    status.signed = true;
    if bytes.len() < 3 {
        return Err(Error::invalid("BDX 签名尾部截断"));
    }
    let length_byte = bytes[bytes.len() - 2];
    let (length, trailer) = if length_byte == 255 {
        if bytes.len() < 4 {
            return Err(Error::invalid("BDX 长签名尾部截断"));
        }
        (
            usize::from(u16::from_be_bytes(
                bytes[bytes.len() - 4..bytes.len() - 2].try_into().unwrap(),
            )),
            4,
        )
    } else {
        (usize::from(length_byte), 2)
    };
    if trailer == 4 && length >= bytes.len() - trailer {
        return Err(Error::invalid("Too long signature"));
    }
    let sign_at = bytes
        .len()
        .checked_sub(length + trailer)
        .ok_or_else(|| Error::invalid("BDX 签名长度越界"))?;
    let body_end = sign_at
        .checked_sub(1)
        .ok_or_else(|| Error::invalid("BDX 缺少签名标记"))?;
    let signature = &bytes[sign_at..bytes.len() - trailer];
    if !signature.starts_with(&[0, 0x8b]) {
        status.legacy = true;
        status.username =
            Some("Unknown User(This file is using a deprecated signing method)".into());
        return Ok(status);
    }
    if signature.len() < 4 {
        status.corrupted = true;
        return Ok(status);
    }
    let cert_length = usize::from(u16::from_le_bytes(signature[2..4].try_into().unwrap()));
    let cert_end = 4usize
        .checked_add(cert_length)
        .ok_or_else(|| Error::invalid("BDX 证书长度溢出"))?;
    let Some(certificate) = signature.get(4..cert_end) else {
        status.corrupted = true;
        return Ok(status);
    };
    let certificate = match std::str::from_utf8(certificate) {
        Ok(certificate) => certificate,
        Err(_) => {
            status.corrupted = true;
            return Ok(status);
        }
    };
    let validate = || -> Result<String> {
        let (content, cert_signature) = certificate
            .split_once("::")
            .ok_or_else(|| Error::invalid("BDX 证书分隔符错误"))?;
        let server_key =
            RsaPublicKey::from_pkcs1_pem(SERVER_KEY).map_err(|e| Error::Other(e.to_string()))?;
        let cert_signature =
            hex::decode(cert_signature).map_err(|e| Error::invalid(e.to_string()))?;
        server_key
            .verify(
                Pkcs1v15Sign::new::<Sha256>(),
                &Sha256::digest(content.as_bytes()),
                &cert_signature,
            )
            .map_err(|e| Error::invalid(e.to_string()))?;
        let (key, username) = content
            .split_once('|')
            .ok_or_else(|| Error::invalid("BDX 证书用户字段错误"))?;
        let user_key =
            RsaPublicKey::from_pkcs1_pem(key).map_err(|e| Error::invalid(e.to_string()))?;
        user_key
            .verify(
                Pkcs1v15Sign::new::<Sha256>(),
                &Sha256::digest(&bytes[..body_end]),
                &signature[cert_end..],
            )
            .map_err(|e| Error::invalid(e.to_string()))?;
        Ok(username.to_owned())
    };
    match validate() {
        Ok(username) => status.username = Some(username),
        Err(_) => status.corrupted = true,
    }
    Ok(status)
}
