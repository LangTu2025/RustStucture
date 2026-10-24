# 本地兼容修复

来源：crates.io `bedrock-leveldb 0.3.1`，保留 MIT/Apache-2.0 双许可证。

`src/table.rs::write_native_table` 原来将 SST footer 的 metaindex handle 写为 `(0,0)`。
本库读取器忽略该 handle，但原生 Go LevelDB 会读取它，因而把首数据块的头部当作
空元数据块的压缩类型与校验值，报 meta-block checksum mismatch。

本地修复在 index 前写入标准空 metaindex block（单个 restart=0，restart_count=1），
写入原有 CRC trailer，并将实际 offset/size 记录到 footer。不改变 CRC 算法，
不关闭校验，不升级依赖。项目回归测试：`tests/leveldb_interop.rs`；外部 Go 重开
验证见项目兼容文档。

`src/db.rs::load_existing_or_initialize` 原来只回放 manifest 当前编号的一个 WAL，
没有恢复编号更大的日志；`src/manifest.rs` 同时丢弃了 prevLogNumber（版本编辑 tag 9）。
原 Go LevelDB 在有多个未刷表 WAL 的合法世界中，会因此被本库少读尾部数据。

本地修复遵循原生 LevelDB 的恢复条件：按文件编号顺序回放所有编号不小于
logNumber 或等于 prevLogNumber 的 WAL，保持全部 CRC 和批次解码校验。
恢复后的写入追加到最新日志，文件编号分配避开现存的新日志；刷表后清除
prevLogNumber，避免过期日志使删除的键复活。只读打开不会修改原数据库。

回归测试在 `src/db.rs`：跨 SST 与多 WAL 的覆盖/删除、恢复后再写入和刷表重开、
previous WAL 与过期 WAL 的区分，以及较新日志的校验和损坏拒绝。
实际 Go 主城世界的 14,394 个可见键，由 Go 与本库分别
枚举后逐键比较值长度及 SHA-256，零缺失、零不同；本库只读扫描前后数据库
全部文件 SHA-256 保持不变。开发期对照工具位于项目 `work/`，不属于运行时依赖。

`src/db.rs::flush_locked` 原来在新 manifest 发布前删除旧 SST、修改内存 manifest、
清空 dirty overlay；`src/manifest.rs` 先删除原 manifest 再 rename，CURRENT 也直接覆盖。
这些步骤遇到 I/O 错误时可能使旧数据库和待重试数据失效。

本地修复先构造候选 manifest，写入并 sync 新 SST 和新 WAL，再 sync 临时 manifest
并原子替换；需要改变 CURRENT 时通过临时文件 sync 后 rename 发布。
CURRENT 已指向固定 manifest 时不重复重写指针，确保没有发布成功后的第二个失败点。
发布成功后才提交内存状态、清除 dirty overlay；过期文件清理失败仅记录警告并保留文件，
不将已经提交的刷表误报为失败。SST 替换同样不再先删除原文件。

新增真实文件系统故障回归：目录阻塞 manifest 临时文件、Windows 共享句柄拒绝原子替换、
CURRENT 指向旧 manifest 时阻塞指针发布，均验证失败后旧数据库可重开、dirty 保留且可重试；
锁定旧 SST 阻止删除，验证已经提交的刷表仍成功并可重开。42 个库单元测试全部通过，
`clippy --lib -- -D warnings` 通过。测试未模拟断电或磁盘控制器故障；sync 与原子替换
改善文件系统发布顺序，不将其描述为已经通过断电试验。

另外将 `src/coding.rs` 的 CRC32C 逐位循环改为运行时检测的 x64 SSE4.2 路径，
其他 CPU 使用编译期生成的256项查表。多项式、初值、分段更新、最终取反和
LevelDB mask 均未变化；原生指令只在 CPU 明确支持时调用。8组 Go 标准库
Castagnoli 固定向量、空输入、短尾、分段边界同时核对自动路径与纯软件路径。

未更改依赖版本，也没有禁用任何校验。除本文列出的兼容、持久化和CRC优化外保持 0.3.1 原版。
