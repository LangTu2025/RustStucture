# Bedrock 方块升级 schema

此目录保存 `github.com/Happy2018new/worldupgrader@v1.4.0/blockupgrader/schemas` 的全部 **35** 个 JSON 原始文件，未重排、压缩或修改内容。Rust 按原 Go `embed.FS.ReadDir` 的文件名字典序加载；不是按版本字段重新排序。

- 固定来源：<https://github.com/Happy2018new/worldupgrader/tree/dd5e48d4b058f1c71647629866a27fb43bb1c281/blockupgrader>
- 模块版本：`v1.4.0`，Go 模块校验：`h1:RUVMkKsqzEfoMLZzavB00iDN7UhcK9iqda5ca5tu3yg=`。
- 上游 schema 项目：<https://github.com/pmmp/BedrockBlockUpgradeSchema>。
- 算法来源文件：`upgrader.go`、`schema.go`、`model.go`、`registry.go`。算法的 MIT 许可证保存在 `LICENSE-worldupgrader-MIT`；schema 的 CC0 文本保存在 `LICENSE-schema-CC0`，取自上游项目的 `LICENSE`。
- `manifest.json` 记录每个原始 JSON 的长度和 SHA-256，以及两个许可证文本的 SHA-256。测试验证全部 schema 文件；未使用上游后续版本替换数据。

实现 `src/blocks/upgrade.rs` 保留字节、整数、字符串的类型区别，使用输入的原始版本筛选 schema。状态级 remap 优先于普通规则；普通规则顺序为名称变换、添加属性、移除属性、属性重命名并同步映射值、剩余属性值映射。展平 byte 时按 Go 的 `uint8` 解释 NBT 字节位模式。

测试用 Go oracle 仅用于开发验证，Rust 运行时不调用 Go。复现步骤和覆盖统计见 `tests/oracles/block_upgrade/README.md`。
