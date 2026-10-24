# 二进制、指令格式和世界缓存验证

目标为除 SIBI 外的36种格式。本模块负责其中8种：BDX、MCFunction、KBDX、IBImport、BDS、BCF、TIBI、NexusNP。没有使用通用私有容器代替真实格式。Go源码作为只读参考，外部读取器为 `work/oracle.exe`。

## 外部验证证据

`tests/binary_formats.rs::original_go_readers_accept_every_binary_encoder` 将同一组石头、玻璃、定向楼梯、红羊毛、定向箱子分别导出成8种格式，然后由原Go读取器扫描。8种输出的尺寸、非空气数量和方块 SHA-256 全部相同。

三个真实输入另外完成了“真实输入 → Rust解码及导出 → 原Go读取器”的逐项对照：尺寸、扫描非空气数量、NBT数量、方块 SHA-256、类型化NBT SHA-256 均精确相等。

| 格式 | 真实样例 | 尺寸 | 非空气方块 | 方块实体 |
|---|---|---|---:|---:|
| BDX | 七天签到(1).bdx | 41×25×2 | 73 | 70 |
| KBDX | 初见黑市.kbdx | 65×24×33 | 5222 | 206 |
| IBImport | 二级雪球菜单(含仓库-公益-by虹芡).ibi | 14×30×10 | 874 | 288 |

BDX样例的原Go方块摘要为 `19f672ae81ad9e1de6a3f85098037f41e6a0241fa8111cd3b237ae06bf6f0017`，原始BDX的NBT摘要为 `2fc29e6348978bd7c15dc46983e928d1209715eb4c1242be6b9a7647ee2168c1`。

其他独立证据：

- BDX：手写wire夹具覆盖源Go声明的全部移动、放置、命令方块、容器、任意NBT、runtime pool和辅助opcode，117池来自原始JSON表；额外读取A空岛指令、二维地皮、nor实验室地图。二维地皮由Go给出的基线为76×19×59、21484非空气、92方块实体。
- MCFunction：手写fill、相对/负坐标、覆盖空气夹具；螺旋跑酷真实样例专项测试与Go尺寸105×203×105及1326819非空气一致。专项测试默认忽略，已经显式运行成功；这不是Release性能基准。
- KBDX：手写20字节小端记录及JSON调色板，覆盖字符串/浮点整数和容器物品类型。
- IBImport：独立手写长度varint、单字节XOR、分段JSON及带/不带padding的规范Base64识别。
- BDS、NexusNP：直接手写MessagePack字节；Go读取Rust导出的真实数组布局成功。
- TIBI：Python标准库hashlib/zlib独立生成固定raw-deflate、MD5派生XOR夹具；Go读取Rust导出成功。
- BCF：按Go测试独立构造版本1、2、3布局；负四万/正四万世界坐标使用子区块origin分组，原Go读取器验证尺寸与方块数成功。

## BDX直接导入世界的实际Go对照

在修复底层SST空metaindex校验和问题并统一使用Snappy tag1后，对小样例七天签到分别执行：

1. CLI `convert ... --to MCWorld`，使用 `bdx-stream` 直接导入CachedWorld。
2. CLI `convert ... --to MCWorld --offset "0,0,0"`，使用普通解码/世界导出路径。
3. 分别用原Go `oracle.exe <输出.mcworld> MCWorld` 重开并扫描。

两条路径的原Go结果完全相同：41×25×2、73非空气、70方块实体；方块摘要仍为上述 `19f672...0017`；两份世界的类型化NBT摘要均为 `7edd3bb61d83f9b9cc5f5c5885888f61e552e424ba6be90247fbb02e6e1c4eec`。世界中的NBT增加真实世界x/y/z，故不要求其摘要与原BDX缺坐标的NBT摘要相同。

原Go MCWorld读取器的 `reported_count` 在该样例为2050，即选区体积；本验证采用实际逐格扫描的 `non_air=73` 与方块摘要，避免将该计数当作真实非空气数量。

## 缓存与快速路径

2026-10-17对Release语义报告作了额外逐字段核查。读取现有Go/Rust世界的原始方块实体记录，比较完整嵌套NBT的标签类型、字段和值，结果如下；表中的差异均为Rust此前漏保留的 `LPCommandMode:Int`，共同坐标未发现其他字段或类型差异。

| 源BDX | Go/Rust共有NBT | 缺少模式字段 |
| --- | ---: | ---: |
| 飞蛾地铁/抽奖 | 17 | 13 |
| 飞蛾地铁/地铁冰河 | 46 | 30 |
| 飞蛾地铁/对峙 | 1476 | 124 |
| 飞蛾地铁/黑市 | 404 | 253 |
| 飞蛾地铁/荒岛 | 331 | 151 |
| 飞蛾地铁/旧封 | 306 | 60 |
| 飞蛾地铁/指令 | 1442 | 1150 |
| 飞蛾地铁/主城 | 130 | 21 |
| test/主城 | 4457（初始只读扫描结果） | 242 |
| test/主城黑市(1) | 732 | 113 |

原因是opcode41携带的完整NBT也经过了清除临时模式字段的逻辑。现已区分原始完整NBT与专用命令方块指令：前者保留字段，后者仍只用模式解析方块类型。独立手写LE NBT夹具先重现失败，再验证普通解码和快导入关闭重开的字段完整性。修复后小样例“抽奖”重新导入，17个NBT与原Go产物完整类型化内容逐项相同。

初始 `world_digest-v1` 对同一Go“test/主城”产物报告2377344非空气、4457NBT；但原Go `oracle.exe` 重开该现有产物实际得到2912386非空气、4527NBT，与Rust产物数量一致。因此这535042方块和70NBT的初始差距属于Rust只读数据库扫描问题，需要底层读取修复后重新生成语义报告，不能据此删除Rust正确导入的方块。源指令中opcode5有2909072条、opcode41有4527条，不含opcode33；全部所用常量状态组合在Go/Rust解析后的RuntimeID一致。调查只扫描已有大产物与源指令，未重复执行大建筑转换。

Go快速转换的 `makeFastBDXJob` 另有未覆盖 `PlaceRuntimeBlockWithUint32RuntimeID`（opcode33）的代码缺口，但上述主城没有该指令，故不是本次差距原因；Rust继续完整支持该合法指令。开发期详细类型差异记录保存在 `work/semantic-nbt-typed-diff.json`。

`tests/world_cache.rs` 验证真实临时LevelDB目录关闭重开、两层方块、第一层空气而第二层水、保留既有区块其他方块/NBT、容量2的LRU脏淘汰、坏子区块错误传播、周期刷新完成事件以及长周期线程被close唤醒退出。线程测试使用Condvar和channel同步，不使用任意sleep。

快速BDX入口为 `formats::binary::import_bdx_to_world`：解压一次，第一遍计算含初始笔刷的轨迹范围，第二遍按指令写入有界缓存；提供取消、进度、越界错误，并保留普通覆盖不清除已有NBT的Go行为。测试同时比较真实BDX的普通解析结果和直接导入后重开的世界。

当前验证命令：

```powershell
& ".\tools\cargo.ps1" test --test binary_formats --test world_cache --test api
& ".\tools\cargo.ps1" -CargoArguments @("clippy", "--tests", "--", "-D", "warnings")
```

二进制测试20通过、1专项默认忽略但已显式通过；世界缓存测试8通过。API交接时15测试通过，涵盖offset、有限NBT字段损失、legacy降级、目录导出和二进制自动检测。后续根模块变更应以实际最新测试输出为准。

## 明确限制

- BDX任意Compound通过opcode41保留；IBImport仅表达命令方块字段；KBDX的Go读取器对命令/容器/告示牌只接受有限字段。MCFunction、BDS、BCF、TIBI、NexusNP没有通用方块NBT或第二层语义。API用损失报告和Strict拒绝处理这些差异。
- BDX导出采用原Go的局部坐标语义；Axiom量化、MCWorld归档重定位及其他格式能力由API单独报告。
- 新式RSA证书链校验已实现，签名尾部边界及旧签名兼容已测试；尚无可确认有效的新式签名样例作为成功基线。
- 未注入磁盘损坏或磁盘写满来验证后台写失败；实现返回/保留刷新错误，坏输入读取错误已覆盖。
- 本文不宣称36种格式全部无损，也不宣称已全面覆盖各格式metadata语义；其他格式证据见对应验证文档。
