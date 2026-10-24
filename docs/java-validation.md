# NBT 结构格式验证记录

## 实现范围

`src/formats/java.rs` 及其子模块提供 Schematic、SchemV1、SchemV2、Litematic、MCStructure、AxiomBP、Construction 的原生 Rust 读取与写入，没有 Go 运行时依赖。

实现逐项对照了同级 WaterStructure 的对应 Go 源码，另外参考了 [Amulet Construction 元数据规范](https://github.com/Amulet-Team/construction-specification/blob/master/specifications/version_0/metadata.md)和[区段规范](https://github.com/Amulet-Team/construction-specification/blob/master/specifications/version_0/section_data_table.md)。Construction 的 `extra_blocks` 是调色板索引列表；区段与元数据使用 gzip。

## 自动验证

运行命令：

```powershell
& "./tools/cargo.ps1" test --test java_formats
& "./tools/cargo.ps1" clippy --test java_formats
```

本组最近一次完整结果为 12 个测试通过、1 个需要本地大型样例的探测测试忽略。本组源代码和测试没有 Clippy 警告。覆盖范围包括：

- 手工 NBT 夹具：Schem Varint 128、MCStructure 仅第二层存在水、Construction 二进制区段索引与 `extra_blocks`、Schematic 标准字节数组。
- Litematic 五位索引跨越 long 边界、负尺寸区域；Axiom 五位索引在每个 long 内填充。
- 七种写入器的真实文件签名，以及带类型的方块 NBT、非法 UTF-8 原始字符串、实体和原点的往返。
- 紧凑读取器与四类真实样例的普通读取结果逐方块、逐方块 NBT 比较。
- Axiom 非 16 对齐原点的绝对方块坐标保持；Construction 非对齐原点的区段划分与第二层保存。

## 原 Go 读取器差分

开发工具使用原 Go `StructureFromFile`、`GetChunks` 和 `GetChunksNBT`；没有改动原 Go 格式源码。批量调用 `GetChunks` 可以避免为每一个区块重复解压同一文件。

以下真实样例的尺寸、主层非空气数量，以及按区块和坐标排序后的原生 RuntimeID SHA-256 完全一致：

| 样例 | 尺寸 X×Y×Z | 非空气数量 | 主层 SHA-256 |
| --- | --- | ---: | --- |
| 123.construction | 17×16×17 | 333 | `61bb467081bf8ad4066a5e312a290debc1a565b7813c3a5a64a2146cf15fef1f` |
| 结构方块测试.mcstructure | 12×1×12 | 61 | `535d02a62f8e3801195e3a50d633c0989affcbc8fd428fb0c8d6c0b11a5455dc` |
| 影子门户.mcstructure | 51×38×23 | 6150 | `fec9a838faf9acfda2bd62353e16b6558563830fa0f79145ef0de83efc22d785` |
| 建筑37.schem | 136×70×135 | 185345 | `55bb4d3690fdc3827052a50cb05a2a610c273b1808b34af4145909358aea6033` |
| ziyounvshenxiang.schem | 123×207×115 | 261653 | `a79d19b0c2b40d523bcabc3fbaa98c51617c4e91b79c8c29b4defe9ea0808dc0` |
| 破碎之梦·落叶就此消逝于尘埃.litematic | 256×116×197 | 621933 | `f1616d43bd8db957ebc47920b2240fb82d0a3cb981cae2c4762a4e567ef80668` |
| 末至审判者.bp | 147×119×351 | 200100 | `9254669c2abe136ec77ff1c25e5674e7f645277f61d1da32ab40e68716d29278` |

上述七个 SHA 已固定到集成测试中。额外逐坐标检查的差异数均为零。

Rust 重写后的两个 MCStructure 样例及 Construction 样例也由原 Go 重新读取，主层 SHA 保持一致。Construction 的 53 个方块 NBT、结构方块测试的 60 个方块 NBT，在 Go 读取原文件与重写文件时的 NBT SHA 也一致。原 Go 没有完整暴露其他格式的方块实体与普通实体，因此这些内容另以独立夹具和 Rust 往返验证。

七种 Rust 写入器的石头夹具都能被原 Go 接受。除下面确认的 Schematic 原读取器缺陷外，其余六种的主层 SHA 一致。

## 原 Go Schematic 数组偏移缺陷

Go `schematic.go` 第 224、231 行记录 `Blocks`、`Data` 的值起点；该位置仍然包含 ByteArray 的四字节长度。后续 `GetChunks` 和 `CountNonAirBlocks` 从该位置直接读取方块字节，没有跳过长度字段。

例如体积为 24、只有一个石头的标准文件，会把长度字节 `00 00 00 18` 误当作前四个方块，导致额外方块和坐标偏移。这个问题也影响 Go 自身标准写入器输出。

Rust 保持标准 NBT ByteArray 编码，读取时从长度字段之后的 payload 开始；不会为了迎合错误偏移生成畸形文件。测试直接检查独立 NBT 夹具以及写出后的原始 `Blocks`、`Data` 内容。

## 大型文件紧凑读取（v1 内存接口，保留兼容）

公开接口：

```rust
formats::java::dense::from_file(path, format) -> Result<DenseStructure>
DenseStructure::from_file(path, format) -> Result<DenseStructure>
DenseStructure::state(x: u32, y: u32, z: u32, layer: u8)
    -> Option<&BlockState>
```

支持 Schematic、SchemV1、SchemV2、MCStructure。字段为 `size`、`origin`、`palette`、`indices`、`extra_indices`、`nbt`。统一索引顺序为 Y→Z→X，X 最快。第一遍流式读取头、调色板和方块 NBT并跳过大数组，第二遍直接写入紧凑索引；不构造每方块 BTreeMap，也不同时保留原始 Blocks/Data 数组副本。支持原始 NBT和 gzip，并保留方块实体的原始 NBT 类型及非法 UTF-8 字节。

主层索引需要每体素 4 字节；MCStructure 只有实际遇到非空气第二层时才分配额外的每体素 4 字节。`try_reserve_exact` 将索引分配失败转换为错误。此接口没有保存普通实体和额外元数据，调用端必须明确处理相应转换损失；高度限制由世界写入器检查。

以下是正式端到端基准之前的单独 Release 读取探测，包含两遍扫描、索引分配和非空气计数，不含世界写出：

| 样例 | 尺寸 | 体积 | 非空气 | 索引字节数 | 时间 |
| --- | --- | ---: | ---: | ---: | ---: |
| 乌托邦.schem | 2569×318×2562 | 2093005404 | 544964595 | 8372021616 | 8.421 秒 |
| 茜之地.schem | 1468×291×1521 | 649752948 | 96096311 | 2599011792 | 2.468 秒 |
| 建筑205废弃富人区.schem | 319×46×234 | 3433716 | 889583 | 13734864 | 0.029 秒 |

乌托邦的索引本身超过 8 GB，这是 v1 `from_file` 内存接口的实际限制，不能将其描述为恒定内存流式转换。该接口保留兼容；CLI 新增的有界读取使用下面的分页路径。

## 有界分页读取（v2）

`dense::from_file_bounded(path, format, limit_bytes)` 返回 `BoundedStructure::Memory` 或 `BoundedStructure::Paged`。主层及潜在第二层索引估算超过预算时，第一遍仍共用同一个头、调色板、方块 NBT、实体损失解析器；第二遍按原文件顺序将 u32 索引缓冲写入临时文件，不分配按体积增长的主层 Vec。MCStructure 的 XYZ 原始索引无需在内存中整体重排，第二层仅在发现非空气数据时创建临时文件。

原始临时索引随后以有界条带转置为 16³ 子区块页。YZX 格式读取连续 X 行，MCStructure 读取连续 Z 行；每个原始索引只转置一次。最终页采用 X→Z→Y 布局，每层每页 16 KiB。世界写出通过 `DenseSource::fill_subchunk` 每层一次读取完整页，没有逐体素 seek，也不重新创建全体积索引。大文件转换增加临时磁盘 I/O，转置期间须预留约两份索引的磁盘空间；文件句柄释放时自动删除临时索引。

预算约束的是索引及转置条带，不包含调色板、需要保留的方块实体 NBT、数据库缓存和运行时开销。条带至少容纳一个 16 KiB 页，流式解析另有固定大小的缓冲。测试中的 256 字节阈值用于强制选中分页路径，并不表示整个进程只占 256 字节。

旧 `DenseStructure` API 保留，内存与分页类型共同实现只读 `DenseSource`；原 `write_dense_directory` 委托给同一个公共世界写入器，无需克隆整个索引。

`tests/dense_paged.rs` 用 256 字节和 32 KiB 阈值验证四种格式在跨 16 边界、边缘空气、两层及带类型 NBT 情况下，分页/内存子区块逐索引相同；两者写出的实际 LevelDB 世界通过 `world_digest v1` 得到相同方块与 NBT 语义摘要。无实际第二层时不会创建额外分页索引。

## 亚特兰蒂斯.schematic 的损坏证据

文件位于本地基准输入目录，大小 229968078 字节，头部是未压缩 BigEndian NBT：`0A0009536368656D61746963`。原 Go Schematic 强制 gzip，会原生读取失败；Rust 支持 raw NBT，但进一步标准解析确认内容自身损坏：

- 声明尺寸为 770×354×841，体积 229239780。
- `Data` 标签位于偏移 83，数组长度字段位于偏移 90，声明 229239780 字节。按声明读完后，偏移 229239874 是 `TAG_End`，根复合标签已经结束，没有读到 `Blocks`。
- 在偏移 229905531 能搜索到 `Blocks` 标签字节模式，同样声明 229239780 字节；其后立即出现 `Entities`、`TileEntities`，剩余文件只有约 62 KB，不能容纳声明的方块数组。
- 两个完整的标准字节数组，仅 payload 就至少需要 458479560 字节，已超过整个文件大小。

Rust 明确拒绝该样例，不把 `Data` 猜作 `Blocks`，也不补造方块。原生 Go 失败和 Rust 内容校验失败应在正式测试中分别记录；不得用自动 gzip 预处理冒充原生 Go 兼容结果。

## 表示能力与边界

- MCStructure 和 Construction 原生保存第二层；Construction 的主层为空气、第二层为水也会保留。Construction 超过两层会返回明确的 Unsupported 错误。
- 其他五种格式当前写出主层；调用端应报告第二层损失。
- 七种普通格式编解码器均保存方块 NBT、普通实体和支持的原点字段。Schematic 使用 WEOriginXYZ，Schem 使用 Offset，Litematic 使用区域 Position，Construction 使用 selection_boxes。
- Construction 为匹配原 Go 的方块状态选择，将整数调色板属性标准化为 Int。原 Go 对部分含 Byte 属性的状态会回落默认状态；方块实体 NBT 不做这种有损类型降级。
- Axiom 按原 Go 行为保留最小区域边界，并裁掉正方向末端空气。非 16 对齐原点写入后，解码的结构原点变成区域网格边界，局部方块位置相应平移，绝对世界位置仍保持。声明空白尺寸和原点表示可能变化，严格转换应报告这一点。
- Litematic 多区域在内存中合并为一个包围盒，写出为单一区域；不承诺保留原区域划分和待处理 tick 列表。
