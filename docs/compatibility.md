# 37 个格式版本的能力与验证边界

此表根据 `src/format.rs::FormatId` 和 `src/api.rs::capabilities` 整理，稳定 ID 为 0–36。当前实现 36 个版本的读取和写入；ID 32 SIBI 按项目约定排除，显式返回 Unsupported。实现覆盖、可表达能力和外部验证覆盖是三件不同的事。

“全部 NBT”表示格式可以承载任意方块 Compound，不等于原始文件字节、所有 metadata 或所有转换语义无损。“有限 NBT”字段规则见 [JSON 验证](json-validation.md) 和 [二进制验证](binary-validation.md)。“尺寸”表示写入器能表达声明尺寸和空气边界；“原点”表示能表达世界位置，具体格式仍可能量化或规范化。

SNBT/JSON 不能逐字节保存二进制 NBT 中的非法 UTF-8 `RawString`；“全部 NBT”列仍受此字符串编码限制，实际损失由公共接口报告。

| ID | 格式 | 读/写 | 第二层 | 方块 NBT | 普通实体 | 声明尺寸 | 世界原点 |
| ---: | --- | --- | --- | --- | --- | --- | --- |
| 0 | Schematic | 是/是 | 否 | 全部 | 是 | 是 | 是 |
| 1 | SchemV1 | 是/是 | 否 | 全部 | 是 | 是 | 是 |
| 2 | SchemV2 | 是/是 | 否 | 全部 | 是 | 是 | 是 |
| 3 | Litematic | 是/是 | 否 | 全部 | 是 | 是 | 是 |
| 4 | MCStructure | 是/是 | 是 | 全部 | 是 | 是 | 是 |
| 5 | MCWorld | 是/是 | 是 | 全部 | 否 | 是 | 否 |
| 6 | BDX | 是/是 | 否 | 全部 | 否 | 是 | 否 |
| 7 | Construction | 是/是 | 是 | 全部 | 是 | 是 | 是 |
| 8 | AxiomBP | 是/是 | 否 | 全部 | 是 | 是* | 是* |
| 9 | MCFunction | 是/是 | 否 | 无 | 否 | 否 | 是 |
| 10 | KBDX | 是/是 | 否 | 命令/容器/告示牌 | 否 | 否 | 是 |
| 11 | IBImport | 是/是 | 否 | 命令 | 否 | 否 | 是 |
| 12 | MianYangV1 | 是/是 | 否 | 全部 | 否 | 否 | 是 |
| 13 | MianYangV2 | 是/是 | 否 | 全部 | 否 | 否 | 是 |
| 14 | MianYangV3 | 是/是 | 否 | 全部 | 否 | 否 | 是 |
| 15 | MianYangV4 | 是/是 | 否 | 全部 | 否 | 是 | 是 |
| 16 | GangBanV1 | 是/是 | 否 | 命令 | 否 | 是 | 是 |
| 17 | GangBanV2 | 是/是 | 否 | 命令 | 否 | 是 | 是 |
| 18 | GangBanV3 | 是/是 | 否 | 全部 | 否 | 是 | 是 |
| 19 | GangBanV4 | 是/是 | 否 | 全部 | 否 | 否 | 是 |
| 20 | GangBanV5 | 是/是 | 否 | 命令/容器 | 否 | 否 | 是 |
| 21 | GangBanV6 | 是/是 | 否 | 命令/容器 | 否 | 否 | 是 |
| 22 | GangBanV7 | 是/是 | 否 | 命令/容器 | 否 | 否 | 是 |
| 23 | RunAway | 是/是 | 否 | 无 | 否 | 否 | 是 |
| 24 | QingXuV1 | 是/是 | 否 | 无 | 否 | 否 | 是 |
| 25 | TimeBuilderV1 | 是/是 | 否 | 无 | 否 | 否 | 是 |
| 26 | FuHongV1 | 是/是 | 否 | 无 | 否 | 否 | 是 |
| 27 | FuHongV2 | 是/是 | 否 | 全部* | 否 | 否 | 是 |
| 28 | FuHongV3 | 是/是 | 否 | 命令/容器/告示牌 | 否 | 否 | 是 |
| 29 | FuHongV4 | 是/是 | 否 | 命令/容器/告示牌 | 否 | 否 | 是 |
| 30 | FuHongV5 | 是/是 | 否 | 命令/容器/告示牌 | 否 | 否 | 是 |
| 31 | BDS | 是/是 | 否 | 无 | 否 | 否 | 是 |
| 32 | SIBI | 否/否 | 否 | 无 | 否 | 否 | 否 |
| 33 | BCF | 是/是 | 否 | 无 | 否 | 否 | 是 |
| 34 | TIBI | 是/是 | 否 | 无 | 否 | 否 | 是 |
| 35 | CovStructure | 是/是 | 否 | 无 | 否 | 是 | 否 |
| 36 | NexusNP | 是/是 | 否 | 无 | 否 | 否 | 是 |

AxiomBP 的原点按子区块量化，并可能裁去正方向空气边缘；表中支持不能理解为任意原点和声明尺寸的原样保存。FuHongV2 对 `conditionalMode` 和空 `CustomName` 有原 Go 规范化规则。公共损失检查根据输入的实际字段、边界和编码结果报告这些限制，而不只检查表中布尔值。

## 转换时必须考虑的差异

- 第二层只由 MCStructure、MCWorld、Construction 写出；仅主层空气而第二层有水的方块也必须保留。
- 大部分格式通过 legacy 名称/aux 表达状态；现代状态不一定可逆。QingXu 使用完整状态字符串，FuHongV2 使用状态数组。状态转换损失会进入 `LossReport.block_states`。
- MCWorld 归档默认将结构放置到 CLI 指定的位置，不能把世界目录导出时的显式起点与归档内部原点原样保存混为一谈。普通世界实体当前未实现。
- BDX 保存局部方块轨迹，不承诺原世界 origin；空气边界通过轨迹和补空气表达。任意 Compound 使用 opcode41，命令和容器的特定 opcode 仍有自己的字段语义。
- Litematic 多区域合并到统一结构；不保留原来的区域划分及待执行 tick。Axiom、Construction 等格式的专用 metadata 不作全面无损承诺。
- Java 与 Bedrock 的方块命名、NBT 字段和游戏版本不同。能够携带 Compound，不意味着跨游戏版本自动转换了所有方块实体行为。
- 世界高度、坐标范围、调色板容量和目标格式数值宽度仍需满足目标约束。分页索引预算限制索引和转置缓存，不是整个进程的内存上限，见 NBT 验证文档。

`api::encode` 返回编码字节和 `LossReport`。Compatible 模式允许有损转换并返回报告；Strict 模式遇到实际损失时拒绝。CLI 的 `formats` 子命令输出当前可执行文件的能力 JSON，便于自动检查；使用旧的冻结基准二进制时，其能力输出可能早于最新源码。

## 外部验证入口

| 家族 | 证据文档 | 已完成的独立验证概要 |
| --- | --- | --- |
| NBT 结构格式（7） | [java-validation.md](java-validation.md) | 手写 NBT/位打包夹具、7 份真实 Go RuntimeID 基线、3 份真实重写后 Go 对照、7 种小输出外部读取；Schematic 原 Go ByteArray 偏移缺陷单列说明 |
| JSON 家族（20） | [json-validation.md](json-validation.md) | 20 种小输出 Go 读取、15 种命令 NBT 输出、15 份真实重导出五项 Go 摘要对照；缺真实样例和 Go 拒读情况逐格式列出 |
| 二进制/指令（8） | [binary-validation.md](binary-validation.md) | 8 种小输出 Go 读取、3 份真实完整方块/NBT 对照、手写 MessagePack/BDX opcode 等夹具、世界缓存重开 |
| MCWorld（1） | [binary-validation.md](binary-validation.md)及 `tests/world.rs` / `tests/world_cache.rs` | 原 Go 重开 Rust LevelDB/Snappy 产物、两层与缓存读写；实际大样本结果另见基准产物的文件与语义摘要 |
| SIBI（1） | 项目范围约定 | 缺少规范与真实样例，明确排除；没有伪造占位格式 |

正式性能报告另外记录单次 Go/Rust Release 转换耗时、CPU、进程峰值内存和文件字节 SHA-256。文件哈希不同不能直接判定语义不同；集成测试的世界方块/NBT 摘要实现位于 `tests/support/world_digest.rs`，遇到差异需要检查坐标、状态、NBT 类型和默认字段，而不能为了摘要相同删除原数据。
