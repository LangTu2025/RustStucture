//! 线程安全的 Bedrock 世界增量缓存。坐标始终为真实世界坐标。
use super::{chunk_key, db_error, decode_subchunk, encode_storage};
use crate::nbt::{self, Compound, Encoding, Nbt, Reader};
use crate::{BlockState, Error, Pos, Result};
use bedrock_leveldb::{CompressionPolicy, Db, OpenOptions, WriteBatch, WriteOptions};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct CacheLimits {
    pub subchunks: usize,
    pub nbt_chunks: usize,
}

impl Default for CacheLimits {
    fn default() -> Self {
        Self {
            subchunks: 64,
            nbt_chunks: 64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    pub subchunks: usize,
    pub nbt_chunks: usize,
    pub dirty_subchunks: usize,
    pub dirty_nbt_chunks: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SubchunkKey {
    x: i32,
    y: i8,
    z: i32,
}

struct SubchunkEntry {
    layers: Vec<Vec<BlockState>>,
    dirty: bool,
    touched: u64,
}

struct NbtEntry {
    values: BTreeMap<Pos, Compound>,
    dirty: bool,
    touched: u64,
}

struct Inner {
    db: Option<Db>,
    limits: CacheLimits,
    subchunks: BTreeMap<SubchunkKey, SubchunkEntry>,
    nbt_chunks: BTreeMap<(i32, i32), NbtEntry>,
    clock: u64,
    flush_generation: u64,
    background_error: Option<String>,
    db_dirty: bool,
}

struct Shared {
    inner: Mutex<Inner>,
    changed: Condvar,
}

struct Worker {
    stop: Arc<(Mutex<bool>, Condvar)>,
    handle: JoinHandle<()>,
}

/// 有界 LRU 缓存；脏项淘汰前写入数据库，显式 flush/close 返回所有写入错误。
pub struct CachedWorld {
    shared: Arc<Shared>,
    worker: Mutex<Option<Worker>>,
}

fn poisoned() -> Error {
    Error::Other("世界缓存锁已损坏".into())
}

fn validate_position(pos: Pos, layer: u8) -> Result<(SubchunkKey, usize)> {
    if !(-64..=319).contains(&pos.1) {
        return Err(Error::invalid("方块超出主世界高度 [-64,319]"));
    }
    if layer > 1 {
        return Err(Error::Unsupported("世界方块层仅支持0和1".into()));
    }
    let key = SubchunkKey {
        x: pos.0.div_euclid(16),
        y: pos.1.div_euclid(16) as i8,
        z: pos.2.div_euclid(16),
    };
    let index =
        ((pos.0.rem_euclid(16) << 8) | (pos.2.rem_euclid(16) << 4) | pos.1.rem_euclid(16)) as usize;
    Ok((key, index))
}

fn add_subchunk(batch: &mut WriteBatch, key: SubchunkKey, entry: &SubchunkEntry) -> Result<()> {
    let layer_count =
        u8::try_from(entry.layers.len()).map_err(|_| Error::invalid("子区块层数过大"))?;
    let mut bytes = vec![9, layer_count, key.y as u8];
    for layer in &entry.layers {
        bytes.extend(encode_storage(layer)?);
    }
    batch.put(chunk_key(key.x, key.z, b'/', Some(key.y)), bytes);
    batch.put(chunk_key(key.x, key.z, b',', None), vec![41]);
    batch.put(
        chunk_key(key.x, key.z, b'6', None),
        2i32.to_le_bytes().to_vec(),
    );
    Ok(())
}

fn add_nbt(batch: &mut WriteBatch, key: (i32, i32), entry: &NbtEntry) -> Result<()> {
    let mut bytes = Vec::new();
    for value in entry.values.values() {
        bytes.extend(nbt::encode(
            "",
            &Nbt::Compound(value.clone()),
            Encoding::LittleEndian,
        )?);
    }
    batch.put(chunk_key(key.0, key.1, b'1', None), bytes);
    Ok(())
}

impl Inner {
    fn db(&self) -> Result<&Db> {
        self.db
            .as_ref()
            .ok_or_else(|| Error::invalid("世界缓存已经关闭"))
    }

    fn check(&self) -> Result<()> {
        self.db()?;
        if let Some(error) = &self.background_error {
            return Err(Error::Other(format!(
                "周期刷新失败，请调用 flush 重试: {error}"
            )));
        }
        Ok(())
    }

    fn tick(&mut self) -> u64 {
        self.clock = self.clock.saturating_add(1);
        self.clock
    }

    fn evict_subchunk(&mut self) -> Result<()> {
        if self.subchunks.len() < self.limits.subchunks {
            return Ok(());
        }
        let key = *self
            .subchunks
            .iter()
            .min_by_key(|(_, entry)| entry.touched)
            .map(|(key, _)| key)
            .ok_or_else(|| Error::invalid("子区块缓存为空"))?;
        let entry = &self.subchunks[&key];
        if entry.dirty {
            let mut batch = WriteBatch::default();
            add_subchunk(&mut batch, key, entry)?;
            self.db()?
                .write(batch, WriteOptions::default())
                .map_err(db_error)?;
            self.db_dirty = true;
        }
        self.subchunks.remove(&key);
        Ok(())
    }

    fn subchunk(&mut self, key: SubchunkKey) -> Result<&mut SubchunkEntry> {
        let touched = self.tick();
        if !self.subchunks.contains_key(&key) {
            self.evict_subchunk()?;
            let bytes = self
                .db()?
                .get(&chunk_key(key.x, key.z, b'/', Some(key.y)))
                .map_err(db_error)?;
            let layers = if let Some(bytes) = bytes {
                let (stored_y, layers) = decode_subchunk(&bytes, key.y)?;
                if stored_y != i32::from(key.y) {
                    return Err(Error::invalid("子区块头部高度与键不一致"));
                }
                layers
            } else {
                vec![vec![BlockState::air(); 4096]]
            };
            self.subchunks.insert(
                key,
                SubchunkEntry {
                    layers,
                    dirty: false,
                    touched,
                },
            );
        }
        let entry = self
            .subchunks
            .get_mut(&key)
            .ok_or_else(|| Error::invalid("子区块缓存缺失"))?;
        entry.touched = touched;
        Ok(entry)
    }

    fn evict_nbt(&mut self) -> Result<()> {
        if self.nbt_chunks.len() < self.limits.nbt_chunks {
            return Ok(());
        }
        let key = *self
            .nbt_chunks
            .iter()
            .min_by_key(|(_, entry)| entry.touched)
            .map(|(key, _)| key)
            .ok_or_else(|| Error::invalid("NBT缓存为空"))?;
        let entry = &self.nbt_chunks[&key];
        if entry.dirty {
            let mut batch = WriteBatch::default();
            add_nbt(&mut batch, key, entry)?;
            self.db()?
                .write(batch, WriteOptions::default())
                .map_err(db_error)?;
            self.db_dirty = true;
        }
        self.nbt_chunks.remove(&key);
        Ok(())
    }

    fn nbt_chunk(&mut self, key: (i32, i32)) -> Result<&mut NbtEntry> {
        let touched = self.tick();
        if !self.nbt_chunks.contains_key(&key) {
            self.evict_nbt()?;
            let bytes = self
                .db()?
                .get(&chunk_key(key.0, key.1, b'1', None))
                .map_err(db_error)?;
            let mut values = BTreeMap::new();
            if let Some(bytes) = bytes {
                let mut reader = Reader::new(&bytes, Encoding::LittleEndian);
                while reader.pos < reader.data.len() {
                    let (_, root) = reader.named()?;
                    let value = root
                        .as_compound()
                        .cloned()
                        .ok_or_else(|| Error::invalid("方块NBT根不是Compound"))?;
                    let coordinate = |name: &str| {
                        value
                            .get(name)
                            .and_then(Nbt::as_i64)
                            .and_then(|v| i32::try_from(v).ok())
                            .ok_or_else(|| Error::invalid(format!("方块NBT缺少有效{name}坐标")))
                    };
                    let pos = Pos(coordinate("x")?, coordinate("y")?, coordinate("z")?);
                    if pos.chunk() != key {
                        return Err(Error::invalid("方块NBT位置与区块键不一致"));
                    }
                    values.insert(pos, value);
                }
            }
            self.nbt_chunks.insert(
                key,
                NbtEntry {
                    values,
                    dirty: false,
                    touched,
                },
            );
        }
        let entry = self
            .nbt_chunks
            .get_mut(&key)
            .ok_or_else(|| Error::invalid("NBT缓存缺失"))?;
        entry.touched = touched;
        Ok(entry)
    }

    fn flush(&mut self) -> Result<()> {
        self.db()?;
        let mut batch = WriteBatch::default();
        let mut dirty = false;
        for (key, entry) in &self.subchunks {
            if entry.dirty {
                add_subchunk(&mut batch, *key, entry)?;
                dirty = true;
            }
        }
        for (key, entry) in &self.nbt_chunks {
            if entry.dirty {
                add_nbt(&mut batch, *key, entry)?;
                dirty = true;
            }
        }
        if dirty {
            self.db()?
                .write(batch, WriteOptions::default())
                .map_err(db_error)?;
            self.db_dirty = true;
        }
        if self.db_dirty {
            self.db()?.flush().map_err(db_error)?;
            self.db_dirty = false;
        }
        // 写入及持久化全部成功后才清除脏标记，失败时可以幂等重试。
        for entry in self.subchunks.values_mut() {
            entry.dirty = false;
        }
        for entry in self.nbt_chunks.values_mut() {
            entry.dirty = false;
        }
        self.background_error = None;
        self.flush_generation = self.flush_generation.saturating_add(1);
        Ok(())
    }
}

impl Shared {
    fn lock(&self) -> Result<MutexGuard<'_, Inner>> {
        self.inner.lock().map_err(|_| poisoned())
    }

    fn background_flush(&self) {
        if let Ok(mut inner) = self.lock() {
            if inner.db.is_none() {
                return;
            }
            if let Err(error) = inner.flush() {
                inner.background_error = Some(error.to_string());
            }
            self.changed.notify_all();
        }
    }
}

impl CachedWorld {
    /// 打开世界根目录中的 db；目录不存在时创建数据库，不改写 level.dat。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_limits(path, CacheLimits::default())
    }

    pub fn open_with_limits(path: impl AsRef<Path>, limits: CacheLimits) -> Result<Self> {
        if limits.subchunks == 0 || limits.nbt_chunks == 0 {
            return Err(Error::invalid("缓存上限必须大于0"));
        }
        std::fs::create_dir_all(path.as_ref())?;
        let options = OpenOptions {
            compression_policy: CompressionPolicy::Snappy,
            ..OpenOptions::default()
        };
        let db = Db::open(path.as_ref().join("db"), options).map_err(db_error)?;
        Ok(Self {
            shared: Arc::new(Shared {
                inner: Mutex::new(Inner {
                    db: Some(db),
                    limits,
                    subchunks: BTreeMap::new(),
                    nbt_chunks: BTreeMap::new(),
                    clock: 0,
                    flush_generation: 0,
                    background_error: None,
                    db_dirty: false,
                }),
                changed: Condvar::new(),
            }),
            worker: Mutex::new(None),
        })
    }

    pub fn load_block(&self, pos: Pos, layer: u8) -> Result<BlockState> {
        let (key, index) = validate_position(pos, layer)?;
        let mut inner = self.shared.lock()?;
        inner.check()?;
        Ok(inner
            .subchunk(key)?
            .layers
            .get(usize::from(layer))
            .map_or_else(BlockState::air, |values| values[index].clone()))
    }

    pub fn set_block(&self, pos: Pos, layer: u8, state: BlockState) -> Result<()> {
        validate_position(pos, layer)?;
        // 在修改缓存之前校验属性，避免刷新时因非法NBT触发运行时哈希的断言。
        nbt::encode("", &super::state_nbt(&state), Encoding::LittleEndian)?;
        self.set_resolved_block(pos, layer, state)
    }

    /// 供已通过公共方块转换表解析的格式导入器使用，避免每次放置重复序列化属性。
    pub(crate) fn set_resolved_block(&self, pos: Pos, layer: u8, state: BlockState) -> Result<()> {
        let (key, index) = validate_position(pos, layer)?;
        let mut inner = self.shared.lock()?;
        inner.check()?;
        let entry = inner.subchunk(key)?;
        while entry.layers.len() <= usize::from(layer) {
            entry.layers.push(vec![BlockState::air(); 4096]);
        }
        if entry.layers[usize::from(layer)][index] != state {
            entry.layers[usize::from(layer)][index] = state;
            entry.dirty = true;
        }
        Ok(())
    }

    pub fn load_block_nbt(&self, pos: Pos) -> Result<Option<Compound>> {
        validate_position(pos, 0)?;
        let mut inner = self.shared.lock()?;
        inner.check()?;
        Ok(inner.nbt_chunk(pos.chunk())?.values.get(&pos).cloned())
    }

    pub fn set_block_nbt(&self, pos: Pos, mut value: Compound) -> Result<()> {
        validate_position(pos, 0)?;
        value.insert("x".into(), Nbt::Int(pos.0));
        value.insert("y".into(), Nbt::Int(pos.1));
        value.insert("z".into(), Nbt::Int(pos.2));
        nbt::encode("", &Nbt::Compound(value.clone()), Encoding::LittleEndian)?;
        let mut inner = self.shared.lock()?;
        inner.check()?;
        let entry = inner.nbt_chunk(pos.chunk())?;
        if entry.values.get(&pos) != Some(&value) {
            entry.values.insert(pos, value);
            entry.dirty = true;
        }
        Ok(())
    }

    pub fn flush(&self) -> Result<()> {
        let result = self.shared.lock()?.flush();
        self.shared.changed.notify_all();
        result
    }

    pub fn stats(&self) -> Result<CacheStats> {
        let inner = self.shared.lock()?;
        inner.check()?;
        Ok(CacheStats {
            subchunks: inner.subchunks.len(),
            nbt_chunks: inner.nbt_chunks.len(),
            dirty_subchunks: inner.subchunks.values().filter(|e| e.dirty).count(),
            dirty_nbt_chunks: inner.nbt_chunks.values().filter(|e| e.dirty).count(),
        })
    }

    pub fn flush_generation(&self) -> Result<u64> {
        let inner = self.shared.lock()?;
        inner.check()?;
        Ok(inner.flush_generation)
    }

    /// 等待一次明确的刷新完成事件；超时仅用于避免调用者无限等待。
    pub fn wait_for_flush(&self, after: u64, timeout: Duration) -> Result<u64> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| Error::invalid("等待超时值过大"))?;
        let mut inner = self.shared.lock()?;
        loop {
            inner.check()?;
            if inner.flush_generation > after {
                return Ok(inner.flush_generation);
            }
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| Error::Other("等待世界刷新超时".into()))?;
            let (guard, result) = self
                .shared
                .changed
                .wait_timeout(inner, remaining)
                .map_err(|_| poisoned())?;
            inner = guard;
            if result.timed_out() && inner.flush_generation <= after {
                return Err(Error::Other("等待世界刷新超时".into()));
            }
        }
    }

    pub fn last_auto_flush_error(&self) -> Result<Option<String>> {
        Ok(self.shared.lock()?.background_error.clone())
    }

    pub fn auto_flush(&self, interval: Duration) -> Result<()> {
        if interval.is_zero() {
            return Err(Error::invalid("周期刷新间隔必须大于0"));
        }
        let mut worker = self.worker.lock().map_err(|_| poisoned())?;
        if let Some(previous) = worker.take() {
            previous.stop()?;
        }
        self.shared.lock()?.check()?;
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let signal = Arc::clone(&stop);
        let shared = Arc::clone(&self.shared);
        let handle = thread::Builder::new()
            .name("world-auto-flush".into())
            .spawn(move || {
                let (cancelled, wake) = &*signal;
                let mut cancelled = cancelled.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    let (guard, _) = wake
                        .wait_timeout_while(cancelled, interval, |cancelled| !*cancelled)
                        .unwrap_or_else(|e| e.into_inner());
                    cancelled = guard;
                    if *cancelled {
                        return;
                    }
                    drop(cancelled);
                    shared.background_flush();
                    cancelled = signal.0.lock().unwrap_or_else(|e| e.into_inner());
                }
            })?;
        *worker = Some(Worker { stop, handle });
        Ok(())
    }

    pub fn stop_auto_flush(&self) -> Result<()> {
        if let Some(worker) = self.worker.lock().map_err(|_| poisoned())?.take() {
            worker.stop()?;
        }
        Ok(())
    }

    /// 停止周期任务，刷新后释放数据库锁；失败时保留数据库和脏缓存供重试。
    pub fn close(&self) -> Result<()> {
        // 持有 worker 锁直到数据库关闭，避免并发 auto_flush 在停止后重新启动线程。
        let mut worker = self.worker.lock().map_err(|_| poisoned())?;
        if let Some(previous) = worker.take() {
            previous.stop()?;
        }
        let mut inner = self.shared.lock()?;
        if inner.db.is_none() {
            return Ok(());
        }
        inner.flush()?;
        inner.subchunks.clear();
        inner.nbt_chunks.clear();
        inner.db.take();
        self.shared.changed.notify_all();
        Ok(())
    }
}

impl Worker {
    fn stop(self) -> Result<()> {
        let (stop, wake) = &*self.stop;
        *stop.lock().unwrap_or_else(|e| e.into_inner()) = true;
        wake.notify_all();
        self.handle
            .join()
            .map_err(|_| Error::Other("周期刷新线程异常退出".into()))
    }
}

impl Drop for CachedWorld {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("释放世界缓存失败，请使用显式 close 处理错误: {error}");
        }
    }
}
