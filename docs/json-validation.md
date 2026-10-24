# JSON 家族格式验证记录

## 范围与证据等级

`src/formats/json.rs` 及其四个子模块实现下表 20 种格式的原生 Rust 读取和写入。实现逐项参照同级 WaterStructure 的对应 Go 源文件；Rust 编译、发布和运行均不依赖 Go。

外部验证工具是 `tools/oracle/main.go` 编译的 `work/oracle.exe`，实际调用原 Go 格式读取器、`GetChunks` 和 `GetChunksNBT`。这里的“外部读取通过”表示已测试夹具被接受，不代表所有方块状态、所有 NBT 和所有历史导出器均无损兼容。NBT 计数相同也不等于字段值或数字类型相同。

本组最近一次完整执行 `tests/json_formats.rs` 为 17 个测试通过，包含真实样例、独立手写布局、外部 Go 读取，以及 Rust 自身往返。需要外部工具的测试在工具不存在时直接返回；单独看到测试绿色不足以证明执行过 Go 对照，应先确认 `work/oracle.exe`、`work/json-probe.exe` 存在。本次两者均存在，外部断言实际执行。

## 逐版本覆盖

“小输出”是石头 `(0,0,0)`、泥土 `(2,1,3)`，预期尺寸 `3×2×4`、非空气数 `2`，20 种 Rust 输出都由原 Go 读取器确认。“命令输出”是带中文、`+`、`%` 的命令方块，15 种有 NBT 能力格式都由 Go 确认非空气数 `1`、方块 NBT 数 `1`；命令文本另有 Rust 解码字段断言，不能将 Go 的 NBT 数量断言扩大成所有命令字段精确相等。

“真实重导出”是原 Go 分别读取外部真实输入和 Rust 重写输出，逐项比较 `size`、`non_air`、`blocks_sha256`、`nbt_count`、`nbt_sha256`，五项全部一致。样例位于 `../WaterStructure/test/测试文件`，北欧样例文件名前缀均为 `北欧之家_`。

| ID | 格式 | 小输出 Go 读取 | 命令输出 Go 读取 | 外部真实输入及重导出证据 |
| ---: | --- | --- | --- | --- |
| 12 | MianYangV1 | 通过 | 通过 | `MianYang_V1.json`，真实重导出五项一致，NBT 0 |
| 13 | MianYangV2 | 通过 | 通过 | `MianYang_V2.json`，真实重导出五项一致，NBT 23 |
| 14 | MianYangV3 | 通过 | 通过 | `MianYang_V3.building`，真实重导出五项一致，NBT 23 |
| 15 | MianYangV4 | 通过 | 通过 | `我的世界_475.3万方块.buildingX` 真实读取通过；该大文件重导出 Go 摘要未经验证 |
| 16 | GangBanV1 | 通过 | 通过 | `GangBan_V1.json`，真实重导出五项一致，NBT 0 |
| 17 | GangBanV2 | 通过 | 通过 | `GangBan_V2.json` Rust 读取通过；原 Go 拒读该无范围尾部样例，真实重导出 Go 对照未经验证 |
| 18 | GangBanV3 | 通过 | 通过 | `GangBan_V3.json`，真实重导出五项一致，NBT 23 |
| 19 | GangBanV4 | 通过 | 通过 | `GangBan_V4.json`，真实重导出五项一致，NBT 23 |
| 20 | GangBanV5 | 通过 | 通过 | `GangBan_V5.json`，真实重导出五项一致，NBT 20 |
| 21 | GangBanV6 | 通过 | 通过 | `GangBan_V6.json`，真实重导出五项一致，NBT 20 |
| 22 | GangBanV7 | 通过 | 通过 | `GangBan_V7.reb`，真实重导出五项一致，NBT 20 |
| 23 | RunAway | 通过 | 不支持 NBT | `RunAway.json`，真实重导出五项一致，NBT 0 |
| 24 | QingXuV1 | 通过 | 不支持 NBT | `QingXu_V1.json`，真实重导出五项一致，NBT 0 |
| 25 | TimeBuilderV1 | 通过 | 不支持 NBT | `TimeBuilder_V1.json`，真实重导出五项一致，NBT 0 |
| 26 | FuHongV1 | 通过 | 不支持 NBT | 无仓库真实样例；独立手写布局通过，真实样例兼容未经验证 |
| 27 | FuHongV2 | 通过 | 通过 | 无仓库真实样例；独立 SNBT/状态数组夹具通过，真实样例兼容未经验证 |
| 28 | FuHongV3 | 通过 | 通过 | `FuHong_V3.json`，真实重导出五项一致，NBT 23 |
| 29 | FuHongV4 | 通过 | 通过 | `FuHong_V4.json`，真实重导出五项一致，NBT 23 |
| 30 | FuHongV5 | 通过 | 通过 | `FuHong_V5.fhbuild`，真实重导出五项一致，NBT 23 |
| 35 | CovStructure | 通过 | 不支持 NBT | 无仓库真实样例；独立 X/Z/Y 扁平索引及空气边界夹具通过，真实样例兼容未经验证 |

通常北欧样例尺寸为 `24×27×28`、非空气数 `3103`。GangBanV3 头部声明 `24×28×27`，实际稀疏坐标边界为 `24×27×28`；实现保留声明，原 Go 有界扫描为 `3095`，其 `reported_count=3103` 不能替代逐格非空气数。BuildingX 的真实断言为原点 `(362,16,-181)`、尺寸 `124×81×124`、实际非空气 `341459`；原 Go 的 `reported_count=397032` 同样不能当作实际扫描数。

北欧大部分 legacy 格式的 Go 方块摘要为 `999d6752fd64594ba39992b1471858f23fc75544a1b2e738a1e8315fa462c03b`；GangBanV3 为 `f9ef9de44d8a9a583e504e56b2fe02740d961eab4a59cf70fbd8b9b847a40d3f`。QingXu 没有同样的 aux 表达，其基线为 `4dabb1c2c972abc90926473b5c32a631fa869e6a79364f42567822c75185a2bf`，不要求不同格式的摘要相等。

上述 23 个 NBT 的北欧基线为 `5827f3632bda907beeea8396b7d814ed1c9b3745b5652b33c44c19b1abce113f`；GangBanV5/V6/V7 的 20 个 NBT 基线为 `ea14d85a804c5ca3e42585efd857f6d2ab711b7ba579f26813d65db76908fc9d`。这些是 oracle 的类型化文本摘要，不是文件字节 SHA-256，也不是 `tests/support/world_digest.rs` 的世界摘要算法。

## 精确 NBT 表达能力

这里描述写入后再按原格式语义读取的结果。任何不在可表达字段内的自定义字段均不能保证保存；公共 `LossReport` 对实际输入判断损失，Strict 模式拒绝有损导出。

SNBT/JSON 使用有效 Unicode 字符串。NBT 二进制中的非法 UTF-8 `RawString` 在本组写入时不能逐字节保存；即使下表标为“任意 Compound”，此类字符串仍属于损失，公共接口另行报告。

| 格式 | 可表达的方块 NBT |
| --- | --- |
| MianYangV1/V2/V3/V4 | 任意 Compound，经 SNBT/原格式 NBT 字段保存；百分号编码保护命令中的 `+`、`%` 与中文 |
| GangBanV3/V4 | 任意 Compound，经原格式 SNBT 字段保存 |
| FuHongV2 | 任意 SNBT 字段；有原 Go 规范化例外，见下文 |
| GangBanV1/V2 | 仅命令方块字段集合 A |
| GangBanV5/V6/V7 | 命令方块字段集合 B；容器物品字段集合 C |
| FuHongV3/V4/V5 | 命令方块字段集合 D、容器物品字段集合 C、告示牌正面文本 |
| FuHongV1、RunAway、QingXuV1、TimeBuilderV1、CovStructure | 不支持方块 NBT |

字段集合与重建规则：

- A：`Command`、`CustomName`、`LastOutput`、`TickDelay`、`auto`、`TrackOutput`、`conditionalMode`、`Powered`、`LPCommandMode`。`id` 重建为 `CommandBlock`；`ExecuteOnFirstTick` 从 `auto` 重建，两者原始值不同时会损失。
- B：`Command`、`CustomName`、`TickDelay`、`auto`、`conditionalMode`。`id=CommandBlock`，`ExecuteOnFirstTick=auto`，`TrackOutput=0`、`Powered=0`、`LastOutput=""`；`LPCommandMode` 由方块类型重建。
- C：`Items` 中仅 `Name`、`Damage`、`Count`、`Slot`，分别重建为 String、Short、Byte、Byte；物品嵌套 `tag`、附魔和其他字段不能保留。容器 `id` 由方块名重建。GangBanV5/V6/V7 显式写出空数组，空容器仍保留方块实体。FuHongV3/V4/V5 另外重建 `Findable=0`、`IsOpened=0`、`isMovable=1`，物品 `Block` 重建为 `{name,states:{},val:0,version:17959425}`，不能保留原始物品方块状态和版本。
- D：`Command`、`TickDelay`、`auto`、`CustomName`。`id=CommandBlock`、`ExecuteOnFirstTick=1`、`TrackOutput=1`、`conditionalMode=0`；`Version` 根据 execute 命令形态重建为 19 或 38。

FuHongV2 解码遵循原 Go：方块状态中的 `conditional_bit` 会覆盖 NBT `conditionalMode`，空字符串 `CustomName` 被移除。因此其“任意 SNBT”能力并不表示原始 NBT 字节或这两个字段永远无损。

FuHongV3/V4/V5 告示牌只保留 `FrontText.Text`（或旧字段 `Text`）的文本；背面文本、文字样式、颜色、发光和其他自定义字段不保留。`BackText`/样式使用默认值，`IsWaxed=0`、`isMovable=1`，`id` 根据告示牌类型重建。

容器识别严格参照原 Go，包括 crafter，不自行增加原格式不识别的 brewing_stand。重复坐标的新记录没有 NBT 时保留旧 NBT；FuHong 分组另外遵循首个 NBT 优先规则。

## 状态、位置和压缩变体

20 种格式均不写出第二层 `extra` 和普通实体 `entities`。除 CovStructure 外，原点可以通过绝对坐标或显式头部保存；只有 MianYangV4、GangBanV1/V2/V3、CovStructure 保留声明尺寸及空气边界，其余根据实际方块范围重建。CovStructure 不保存世界原点。

FuHongV2 使用名称和状态数组；QingXu 写出精确 `BlockState::string()`。其余格式主要通过名称及 legacy aux 表达状态，不能保证保留所有现代状态。测试另由原 Go 批量检查真实北欧调色板的逆向映射及 QingXu 名称转换，避免只凭 Rust 自身往返判断。

| 变体 | 真实封装与独立证据 |
| --- | --- |
| MianYangV3 | zlib 包裹 MianYang JSON；外部 `.building` 真实输入、重写后原 Go 五项对照、小输出外部读取均通过 |
| MianYangV4 / BuildingX | gzip 包裹小端尺寸/原点、字符串调色板和坐标差分二进制记录；独立外部 `.buildingX` 读取及小输出原 Go 读取通过，大型重写对照未经验证 |
| GangBanV7 | zlib 包裹 GangBanV6 流式数组；外部 `.reb` 真实输入、重写后原 Go 五项对照、小输出外部读取均通过 |
| FuHongV5 | Unicode 字符按 `FuHongBuild` 循环异或后加 `index % 3`，再 zlib；按字符处理而非 UTF-8 字节；外部 `.fhbuild` 真实输入、重写后原 Go 五项对照、小输出外部读取均通过 |
| MessagePack | 不属于本组 JSON 20 格式；BDS、NexusNP 的独立手写 MessagePack 字节及外部 Go 读取证据见 [二进制验证](binary-validation.md) |

压缩验证包含外部导出器产生的真实输入，因而独立于本实现编码器；`compressed_formats_and_buildingx_roundtrip` 是额外的自身往返测试，不能单独充当外部兼容证据。尚未给这四种压缩变体建立另一套完全手写压缩字节 golden，不能宣称该项已覆盖。

复现：

```powershell
& "./tools/cargo.ps1" test --test json_formats
& "./tools/cargo.ps1" -CargoArguments @("clippy", "--test", "json_formats", "--", "-D", "warnings")
```

其他家族见 [NBT 格式验证](java-validation.md)、[二进制验证](binary-validation.md) 和 [全版本能力表](compatibility.md)。
