# 格式模块共享契约

所有位置均为 `Pos(i32,i32,i32)`，结构方块键使用从 0 开始的结构坐标；`origin` 保存原始最小世界坐标。`size=Size(u32,u32,u32)`，`offset` 初始为 0。对于无声明尺寸的稀疏格式，插入原始坐标后调用 `normalize()?`。有声明尺寸则保持尺寸（包括空气边界），自行转换相对坐标。

格式模块接口：

```rust
pub fn decode(format: crate::FormatId, data: &[u8]) -> crate::Result<crate::StructureData>;
pub fn encode(format: crate::FormatId, data: &crate::StructureData) -> crate::Result<Vec<u8>>;
```

`StructureData::new(format)`；字段 `format,size,origin,offset,blocks:BTreeMap<Pos,Block>,entities:Vec<Compound>,metadata:Compound`。

`Block { state:BlockState, extra:Option<BlockState>, nbt:Option<Compound> }`；`Block::new(state)`。第二层用 extra，第一层为空气且第二层非空气不得丢失。

`BlockState { name:String, properties:Compound }`；`BlockState::new("stone")` 自动添加 minecraft 命名空间，`air(),is_air(),string()` 可用。

`nbt::Compound=BTreeMap<String,Nbt>`；Nbt 枚举 Byte(i8),Short(i16),Int(i32),Long(i64),Float(f32),Double(f64),ByteArray(Vec<i8>),String(String),List(Vec<Nbt>),Compound(Compound),IntArray(Vec<i32>),LongArray(Vec<i64>)。

辅助：`Nbt::as_i64/as_str/as_compound/as_list/scalar_string/from_json/to_json`；`nbt::parse_snbt(&str)->Result<Nbt>`、`to_snbt(&Nbt)->String`；`nbt::decode(bytes,Encoding)->Result<(String,Nbt)>`，`encode(name,&Nbt,Encoding)->Result<Vec<u8>>`；Encoding 四种变体 BigEndian/LittleEndian/NetworkBigEndian/NetworkLittleEndian。`nbt::Reader` 提供 new,take,byte,short,int,long,string,value(tag,depth),named 以及公开 pos 字段；Writer 提供 new,int,long,short,string,value,named 和 data。

方块映射由主代理实现（可先依接口编写）：

```rust
blocks::legacy(name:&str, data:u16)->BlockState
blocks::java(text:&str)->BlockState
blocks::bedrock(name:&str, properties:Compound)->BlockState
blocks::to_java(state:&BlockState)->String
blocks::to_legacy(state:&BlockState)->(String,u16)
blocks::to_schematic(state:&BlockState)->(u8,u8)
blocks::from_schematic(id:u8,data:u8)->BlockState
blocks::runtime_id(state:&BlockState)->u32
blocks::from_runtime_id(id:u32)->BlockState
```

错误：`Error::invalid(impl Into<String>)`、`Error::Unsupported(String)`，Io/Json 可 `?`。其他依赖错误转为 Error::Other(err.to_string())。

严禁用自定义通用文件替代格式；必须输出 Go 原读取器或格式参考读取器接受的字节。现有 Go 测试/真实样例为准。SIBI 单独排除。格式模块不修改 Cargo.toml、lib.rs、model.rs、nbt.rs、blocks.rs、formats/mod.rs；若需依赖/接口变更发消息给主代理。

完整源代码位于同级 ../WaterStructure，父主项目 ../..，现有样例 ../WaterStructure/test/测试文件。按测试先行方式验证真实输入及原Go读新Rust输出。所有代码注释和说明使用简体中文；不提交或创建分支。
