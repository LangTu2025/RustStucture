# 原 Go 升级器对照夹具

`tests/fixtures/block_upgrade_oracle.json` 的期望值由原始 `worldupgrader@v1.4.0` 直接生成，包含 7,383 条带 NBT 标量类型的输入/期望状态。涵盖全部 35 个 schema 的实际规则、已有字段不覆盖、严格类型匹配、复制字段、状态条件不匹配、版本边界前后、byte 无符号展平及 int 展平。`coverage.json` 保存分类统计。

`tests/fixtures/akko_block_upgrade.json` 来自实际 AKKO 世界 LevelDB 的完整 persistent palette：28 个状态，版本均为 17959425，其中 9 个状态发生升级。直接从 Go NBT decoder 提取类型，未用无类型 JSON 数字猜测 Byte/Int。

从 RustWaterStructure 根目录复现通用规则夹具（开发机器已存在原 Fatalder Go 模块依赖）：

```powershell
python "tests/oracles/block_upgrade/generate.py"
& "C:/Program Files (x86)/Go/bin/go.exe" run "./tests/oracles/block_upgrade/main.go" "./work/block-upgrade-oracle/input.json" | Set-Content "tests/fixtures/block_upgrade_oracle.json" -Encoding utf8
```

生成脚本写入 `work/block-upgrade-oracle`，该目录须已存在。Go JSON decoder 使用 `UseNumber`，保证超过 2^53 的 Long 原样保留。两个 `.go` 文件均是独立命令，须分别 `go run`，不能将此目录当成单个 Go package 构建。

AKKO 夹具由 `akko.go` 只读打开已解压世界的 db 目录生成；输出按主/次层遍历去重后的全部状态数组：

```powershell
& "C:/Program Files (x86)/Go/bin/go.exe" run "./tests/oracles/block_upgrade/akko.go" "./work/akko-raw-world2/db" | Set-Content "tests/fixtures/akko_block_upgrade.json" -Encoding utf8
```

Rust 回归测试直接读取已保存期望值，不依赖 Go、原世界文件或网络。全部升级仅改变 `BlockState` 名称/属性，最终运行时 ID 查找由调用方负责。
