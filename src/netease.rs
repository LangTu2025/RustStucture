//! 网易世界文件的流式 XOR 加解密。
use crate::{Error, Result};
use rsa::rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const HEADER: [u8; 4] = [0x80, 0x1d, 0x30, 0x01];

/// 独立保留的恢复记录，成功后继续保留，不参与加解密。
pub const RECOVERY_FILE: &str = ".rust-water-structure-netease-key.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RecoveryRecord {
    version: u8,
    manifest: String,
    key: Vec<u8>,
}

impl RecoveryRecord {
    fn validate_format(&self) -> Result<()> {
        if self.version != 1
            || !self.manifest.starts_with("MANIFEST")
            || self.manifest.contains(['/', '\\', '\r', '\n'])
            || self.manifest.trim() != self.manifest
            || self.key.len() != self.manifest.len() + 1
        {
            return Err(Error::invalid("网易恢复记录的版本、MANIFEST或密钥长度无效"));
        }
        Ok(())
    }

    fn validate(&self, path: &Path) -> Result<()> {
        self.validate_format()?;
        if !path.join(&self.manifest).is_file() {
            return Err(Error::invalid("网易恢复记录指向的MANIFEST不存在"));
        }
        Ok(())
    }
}

fn files_and_manifest(path: &Path) -> Result<(Vec<PathBuf>, String)> {
    if !path.is_dir() {
        return Err(Error::invalid("不是世界数据库目录"));
    }
    let mut files = fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files
        .retain(|path| path.is_file() && path.file_name().is_none_or(|name| name != RECOVERY_FILE));
    files.sort();
    if !path.join("CURRENT").is_file() && !path.join(RECOVERY_FILE).is_file() {
        return Err(Error::invalid("缺少 CURRENT"));
    }
    let manifest = files
        .iter()
        .filter_map(|path| path.file_name()?.to_str())
        .rfind(|name| name.starts_with("MANIFEST"))
        .ok_or_else(|| Error::invalid("缺少 MANIFEST"))?
        .to_owned();
    Ok((files, manifest))
}

fn encrypted(path: &Path) -> Result<bool> {
    let mut file = File::open(path)?;
    let mut bytes = [0; 4];
    Ok(file.read(&mut bytes)? == 4 && bytes == HEADER)
}

fn transform(path: &Path, key: &[u8], encrypt: bool, buffer_size: usize) -> Result<()> {
    if key.is_empty() {
        return Err(Error::invalid("加密密钥为空"));
    }
    let mut source = File::open(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| Error::invalid("文件没有父目录"))?;
    let mut target = tempfile::NamedTempFile::new_in(parent)?;
    if encrypt {
        target.write_all(&HEADER)?;
    } else {
        let mut header = [0; 4];
        source.read_exact(&mut header)?;
        if header != HEADER {
            return Err(Error::invalid("文件未加密"));
        }
    }
    let mut buffer = vec![
        0;
        if buffer_size == 0 {
            1 << 20
        } else {
            buffer_size
        }
    ];
    let mut offset = 0;
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        for byte in &mut buffer[..count] {
            *byte ^= key[offset % key.len()];
            offset += 1;
        }
        target.write_all(&buffer[..count])?;
    }
    target.flush()?;
    target.as_file().sync_all()?;
    drop(source);
    let persisted = target
        .persist(path)
        .map_err(|error| Error::Io(error.error))?;
    persisted.sync_all()?;
    Ok(())
}

fn optional_read(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn load_record(path: &Path) -> Result<Option<RecoveryRecord>> {
    let Some(bytes) = optional_read(&path.join(RECOVERY_FILE))? else {
        return Ok(None);
    };
    let record: RecoveryRecord = serde_json::from_slice(&bytes).map_err(|_| {
        Error::invalid(format!(
            "已有 {RECOVERY_FILE} 不是有效恢复记录，未覆盖该文件"
        ))
    })?;
    // 正常编辑世界会轮换MANIFEST；先验证记录格式，再由恢复流程判断能否安全更新。
    record.validate_format()?;
    Ok(Some(record))
}

fn record_tempfile(path: &Path, record: &RecoveryRecord) -> Result<tempfile::NamedTempFile> {
    let mut target = tempfile::NamedTempFile::new_in(path)?;
    target.write_all(&serde_json::to_vec_pretty(record)?)?;
    target.flush()?;
    target.as_file().sync_all()?;
    Ok(target)
}

fn persist_record(path: &Path, record: &RecoveryRecord) -> Result<()> {
    let matches_existing = |existing: &RecoveryRecord| {
        if existing == record {
            Ok(())
        } else {
            Err(Error::invalid(
                "已有恢复记录的密钥或MANIFEST不匹配，未修改任何记录",
            ))
        }
    };
    if let Some(existing) = load_record(path)? {
        return matches_existing(&existing);
    }
    let target = record_tempfile(path, record)?;
    match target.persist_noclobber(path.join(RECOVERY_FILE)) {
        Ok(file) => file.sync_all()?,
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing =
                load_record(path)?.ok_or_else(|| Error::invalid("恢复记录在写入期间发生变化"))?;
            matches_existing(&existing)?;
        }
        Err(error) => return Err(error.error.into()),
    }
    Ok(())
}

fn refresh_record_for_plain_world(
    path: &Path,
    files: &[PathBuf],
    existing: &RecoveryRecord,
    current: &[u8],
    manifest: &str,
) -> Result<RecoveryRecord> {
    let mut record = existing.clone();
    record.manifest = manifest.into();
    if record.key.len() != manifest.len() + 1 {
        record.key = vec![0; manifest.len() + 1];
        OsRng.fill_bytes(&mut record.key);
    }
    record.validate(path)?;
    for file in files {
        if encrypted(file)? {
            return Err(Error::invalid(
                "MANIFEST已变化但目录仍有加密文件，不能刷新恢复密钥",
            ));
        }
    }

    // 只有全明文目录才能替换本工具的合法旧记录，且新记录须先于任何数据库改动落盘。
    let target = record_tempfile(path, &record)?;
    if load_record(path)?.as_ref() != Some(existing)
        || optional_read(&path.join("CURRENT"))?.as_deref() != Some(current)
    {
        return Err(Error::invalid(
            "恢复记录或CURRENT在更新期间发生变化，未覆盖记录",
        ));
    }
    let persisted = target
        .persist(path.join(RECOVERY_FILE))
        .map_err(|error| error.error)?;
    persisted.sync_all()?;
    Ok(record)
}

fn recover_key(
    path: &Path,
    files: &[PathBuf],
    scanned_manifest: &str,
    encrypt: bool,
) -> Result<RecoveryRecord> {
    let existing = load_record(path)?;
    let current = optional_read(&path.join("CURRENT"))?;
    let record = if let Some(bytes) = current.as_ref().filter(|bytes| bytes.starts_with(&HEADER)) {
        let manifest = existing
            .as_ref()
            .map_or(scanned_manifest, |record| record.manifest.as_str());
        let expected = format!("{manifest}\n");
        if bytes.len() != HEADER.len() + expected.len() {
            return Err(Error::invalid("CURRENT 的加密密钥长度与MANIFEST不匹配"));
        }
        RecoveryRecord {
            version: 1,
            manifest: manifest.into(),
            key: bytes[HEADER.len()..]
                .iter()
                .zip(expected.bytes())
                .map(|(byte, expected)| byte ^ expected)
                .collect(),
        }
    } else if let Some(record) = existing.as_ref() {
        if let Some(bytes) = &current {
            let manifest = std::str::from_utf8(bytes)
                .map_err(|_| Error::invalid("明文CURRENT不是有效UTF-8"))?
                .trim();
            if manifest != record.manifest {
                if encrypt {
                    return refresh_record_for_plain_world(path, files, record, bytes, manifest);
                }
                return Err(Error::invalid("明文CURRENT与保留恢复记录的MANIFEST不一致"));
            }
        }
        record.clone()
    } else {
        if !encrypt {
            return Err(Error::invalid("目录未加密，且没有可用恢复记录"));
        }
        for file in files
            .iter()
            .filter(|file| file.file_name().is_none_or(|name| name != "CURRENT"))
        {
            if encrypted(file)? {
                return Err(Error::invalid(
                    "目录已部分加密但缺少恢复密钥，不能生成新密钥覆盖",
                ));
            }
        }
        let bytes = current
            .as_ref()
            .ok_or_else(|| Error::invalid("缺少CURRENT和恢复记录"))?;
        let current_manifest = std::str::from_utf8(bytes)
            .map_err(|_| Error::invalid("明文CURRENT不是有效UTF-8"))?
            .trim();
        let manifest = if path.join(current_manifest).is_file() {
            current_manifest
        } else {
            scanned_manifest
        };
        let mut key = vec![0; manifest.len() + 1];
        OsRng.fill_bytes(&mut key);
        RecoveryRecord {
            version: 1,
            manifest: manifest.into(),
            key,
        }
    };
    record.validate(path)?;
    // 持久化成功后才允许替换任何用户数据库文件。
    persist_record(path, &record)?;
    Ok(record)
}

fn write_current(path: &Path, record: &RecoveryRecord, encrypt: bool) -> Result<bool> {
    let current = path.join("CURRENT");
    let clear = format!("{}\n", record.manifest).into_bytes();
    let bytes = if encrypt {
        let mut bytes = HEADER.to_vec();
        bytes.extend(clear.iter().zip(&record.key).map(|(byte, key)| byte ^ key));
        bytes
    } else {
        clear
    };
    if optional_read(&current)?.as_deref() == Some(bytes.as_slice()) {
        return Ok(false);
    }
    let mut target = tempfile::NamedTempFile::new_in(path)?;
    target.write_all(&bytes)?;
    target.flush()?;
    target.as_file().sync_all()?;
    let persisted = target.persist(current).map_err(|error| error.error)?;
    persisted.sync_all()?;
    Ok(true)
}

fn transform_directory(
    path: &Path,
    buffer_size: usize,
    encrypt: bool,
    mut after_file: impl FnMut(&Path) -> Result<()>,
) -> Result<()> {
    let (files, manifest) = files_and_manifest(path)?;
    let record = recover_key(path, &files, &manifest, encrypt)?;
    let current = path.join("CURRENT");
    for file in files.iter().filter(|file| **file != current) {
        if encrypted(file)? != encrypt {
            transform(file, &record.key, encrypt, buffer_size)?;
            after_file(file)?;
        }
    }
    // CURRENT始终最后替换；恢复记录在成功后继续保留。
    if write_current(path, &record, encrypt)? {
        after_file(&current)?;
    }
    Ok(())
}

pub fn encrypt(path: &Path, buffer_size: usize) -> Result<()> {
    encrypt_with_callback(path, buffer_size, |_| Ok(()))
}

pub fn decrypt(path: &Path, buffer_size: usize) -> Result<()> {
    decrypt_with_callback(path, buffer_size, |_| Ok(()))
}

/// 每成功替换一个文件后回调；回调错误可用于取消，随后能够安全重试。
pub fn encrypt_with_callback(
    path: &Path,
    buffer_size: usize,
    after_file: impl FnMut(&Path) -> Result<()>,
) -> Result<()> {
    transform_directory(path, buffer_size, true, after_file)
}

/// 允许从CURRENT或保留记录恢复密钥，按文件头跳过已经完成的文件。
pub fn decrypt_with_callback(
    path: &Path,
    buffer_size: usize,
    after_file: impl FnMut(&Path) -> Result<()>,
) -> Result<()> {
    transform_directory(path, buffer_size, false, after_file)
}
