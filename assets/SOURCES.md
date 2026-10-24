# 固定资源来源

资源于 2026-10-17 从 Fatalder 实际依赖提取，不在运行时联网更新。每个文件的字节长度和 SHA256 记录在 `manifest.json`。

| 资源 | 来源 | 基线/许可证 |
| --- | --- | --- |
| `nemc.br`、三个 `*translate.br` | Fatalder 本地 `modules/blocks` 同名资源 | 主项目本地 replace，AGPL-3.0；不是失效的远程 revision |
| `schematic_names.json` | 本地 blocks `schematic.go` 的 `schematicBlockStrings` 原始顺序 | AGPL-3.0，完整保留数字 ID 顺序 |
| `block_states.bin` | `bedrock-world-operator@v1.7.0/block/block_states.bin` | MIT；原生 RuntimeID 基线 |
| `bdx_runtimeIds_117.json` | 同级 WaterStructure `structure/bdx_runtimeIds_117.json` | 原工程 AGPL-3.0，BDX 117 池 |
| `colors.json` | 同级 WaterStructure `utils/map_art/colors.json` | 原工程 AGPL-3.0，地图画配色基线 |
| `block_upgrade_schemas/` | `worldupgrader v1.4.0` 全部 35 份原始规则 | 算法 MIT / schema CC0，子目录另附完整来源、版本和逐文件 SHA256 |

`licenses/blocks-AGPL-3.0.txt`、项目根 `LICENSE`、`licenses/bedrock-world-operator-MIT.txt` 保留对应许可正文。Rust 映射表处理仅标准化必要的类型和索引，不按新版 Minecraft 数据静默替换基线。

主项目 Go 基线还使用 `merry-memory v0.2.0`；它仅参与开发阶段对照，Rust 运行时没有该依赖。`Cargo.lock` 固定 Rust 依赖，`vendor/bedrock-leveldb/PATCHES.md` 记录同版本本地兼容补丁。
