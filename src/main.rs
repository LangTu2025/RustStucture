use clap::{Parser, Subcommand};
use rust_water_structure::api::{self, LossPolicy, Reader};
use rust_water_structure::formats::java::dense::DenseSource;
use rust_water_structure::{Error, FormatId, Pos, Result, StructureData, map_art, netease, world};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser)]
#[command(version, about = "WaterStructure 兼容的 Minecraft 建筑格式转换工具")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 查询全部格式 ID 和可表达能力。
    Formats,
    /// 读取元数据并生成方块、NBT 对照摘要。
    Inspect {
        input: PathBuf,
        #[arg(long)]
        format: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        bounds: Option<String>,
    },
    /// 转换建筑文件或世界目录。
    Convert {
        input: PathBuf,
        output: PathBuf,
        #[arg(long)]
        to: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        bounds: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        offset: Option<String>,
        #[arg(long)]
        strict: bool,
        /// 紧凑索引内存阈值；超出后使用临时磁盘分页（不含调色板/NBT/数据库缓存）。
        #[arg(long, default_value_t = 256)]
        memory_budget_mib: u64,
    },
    /// 从图片生成地图画并导出建筑或世界。
    MapArt {
        input: PathBuf,
        output: PathBuf,
        #[arg(long, default_value_t = 1)]
        map_width: u32,
        #[arg(long, default_value_t = 1)]
        map_height: u32,
        #[arg(long)]
        flat: bool,
        #[arg(long)]
        no_reference_column: bool,
        #[arg(long, default_value_t = 384)]
        max_height: i32,
        #[arg(long, default_value = "0,0,0", allow_hyphen_values = true)]
        start: String,
        #[arg(long, default_value = "MCWorld")]
        to: String,
    },
    /// 网易 LevelDB 目录加解密（原地处理）。
    Netease {
        #[command(subcommand)]
        action: NeteaseAction,
    },
}

#[derive(Subcommand)]
enum NeteaseAction {
    Encrypt {
        path: PathBuf,
        #[arg(long, default_value_t = 1048576)]
        buffer_size: usize,
    },
    Decrypt {
        path: PathBuf,
        #[arg(long, default_value_t = 1048576)]
        buffer_size: usize,
    },
}

fn parse_position(value: &str) -> Result<Pos> {
    let parts = value
        .split(',')
        .map(str::trim)
        .map(str::parse::<i32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::invalid("坐标应为 x,y,z 三个整数"))?;
    if parts.len() != 3 {
        return Err(Error::invalid("坐标必须包含三个整数"));
    }
    Ok(Pos(parts[0], parts[1], parts[2]))
}

fn parse_bounds(value: Option<String>) -> Result<Option<(Pos, Pos)>> {
    value
        .map(|value| {
            let (first, second) = value
                .split_once(':')
                .ok_or_else(|| Error::invalid("选区应为 x,y,z:x,y,z"))?;
            Ok((parse_position(first)?, parse_position(second)?))
        })
        .transpose()
}

fn canonical_nbt(value: &rust_water_structure::nbt::Nbt) -> serde_json::Value {
    use rust_water_structure::nbt::Nbt;
    match value {
        Nbt::RawString(value) => json!({"type":"String","value":String::from_utf8_lossy(value)}),
        Nbt::Compound(value) => {
            json!({"type":"Compound","value":value.iter().map(|(key,value)|(key.clone(),canonical_nbt(value))).collect::<std::collections::BTreeMap<_,_>>()})
        }
        Nbt::List(value) => {
            json!({"type":"List","value":value.iter().map(canonical_nbt).collect::<Vec<_>>()})
        }
        _ => serde_json::to_value(value).expect("NBT 序列化"),
    }
}

fn summarize(data: &StructureData, elapsed_seconds: f64) -> Result<serde_json::Value> {
    let mut block_rows = Vec::new();
    let mut nbt_rows = Vec::new();
    let size = data.get_size();
    let mut non_air = 0;
    for (position, block) in &data.blocks {
        if position.0 < 0
            || position.0 >= size.0 as i32
            || position.1 < 0
            || position.1 >= size.1.min(384) as i32
            || position.2 < 0
            || position.2 >= size.2 as i32
        {
            continue;
        }
        let chunk = position.chunk();
        for (layer, state) in std::iter::once(&block.state)
            .chain(block.extra.iter())
            .enumerate()
        {
            if state.is_air() {
                continue;
            }
            block_rows.push((
                chunk,
                *position,
                layer,
                rust_water_structure::blocks::runtime_id(state),
            ));
            if layer == 0 {
                non_air += 1;
            }
        }
        if let Some(nbt) = &block.nbt {
            nbt_rows.push((
                chunk,
                *position,
                canonical_nbt(&rust_water_structure::nbt::Nbt::Compound(nbt.clone())),
            ));
        }
    }
    block_rows.sort_by_key(|(chunk, position, layer, _)| (*chunk, *position, *layer));
    nbt_rows.sort_by_key(|(chunk, position, _)| (*chunk, *position));
    let mut blocks_hash = Sha256::new();
    for (_, position, layer, runtime_id) in block_rows {
        blocks_hash.update(format!(
            "{},{},{},{},{}\n",
            position.0, position.1, position.2, layer, runtime_id
        ));
    }
    let mut nbt_hash = Sha256::new();
    for (_, position, value) in &nbt_rows {
        nbt_hash.update(format!(
            "{},{},{}:",
            position.0,
            position.1 - 64,
            position.2
        ));
        nbt_hash.update(serde_json::to_vec(value)?);
        nbt_hash.update(b"\n");
    }
    Ok(json!({
        "format":data.format.name(), "id":data.format as u8,
        "size":[size.0,size.1,size.2], "origin":data.origin, "offset":data.offset,
        "non_air":non_air, "reported_count":data.non_air_count(),
        "blocks_sha256":hex::encode(blocks_hash.finalize()),
        "nbt_sha256":hex::encode(nbt_hash.finalize()), "nbt_count":nbt_rows.len(),
        "elapsed_seconds":elapsed_seconds,
    }))
}

fn write_output(
    path: &Path,
    format: FormatId,
    data: &StructureData,
    policy: LossPolicy,
) -> Result<api::LossReport> {
    if format == FormatId::MCWorld && path.extension().is_none() {
        return api::encode_world_directory(path, data, policy);
    }
    let encoded = api::encode(format, data, policy)?;
    persist_output(path, &encoded.bytes)?;
    Ok(encoded.report)
}

fn persist_output(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut output = tempfile::NamedTempFile::new_in(parent)?;
    output.write_all(bytes)?;
    output.flush()?;
    output
        .persist(path)
        .map_err(|error| Error::Io(error.error))?;
    Ok(())
}

fn persist_world_output(path: &Path, directory: &Path) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut output = tempfile::NamedTempFile::new_in(parent)?;
    world::write_directory_archive(directory, output.as_file_mut())?;
    output.as_file_mut().flush()?;
    output
        .persist(path)
        .map_err(|error| Error::Io(error.error))?;
    Ok(())
}

fn reject_input_output_alias(input: &Path, output: &Path) -> Result<()> {
    let input = std::fs::canonicalize(input)?;
    if !output.exists() {
        return Ok(());
    }
    let output = std::fs::canonicalize(output)?;
    #[cfg(windows)]
    let same = input
        .to_string_lossy()
        .eq_ignore_ascii_case(&output.to_string_lossy());
    #[cfg(not(windows))]
    let same = input == output;
    if same {
        return Err(Error::invalid("输入输出路径指向同一文件或目录"));
    }
    Ok(())
}

fn fast_world_conversion(
    input: &Path,
    output: &Path,
    source_format: Option<FormatId>,
    memory_budget: usize,
) -> Result<Option<(&'static str, api::LossReport)>> {
    if output.extension().is_none() {
        return Ok(None);
    }
    let extension = input
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let formats = match source_format {
        Some(format) => vec![format],
        None => match extension.as_str() {
            "bdx" => vec![FormatId::BDX],
            "schematic" => vec![FormatId::Schematic],
            "schem" => vec![FormatId::SchemV1, FormatId::SchemV2],
            "mcstructure" => vec![FormatId::MCStructure],
            _ => return Ok(None),
        },
    };
    let first = formats[0];
    if first == FormatId::BDX {
        let directory = tempfile::tempdir()?;
        let cached = world::cache::CachedWorld::open(directory.path())?;
        let bytes = std::fs::read(input)?;
        let result = rust_water_structure::formats::binary::import_bdx_to_world(
            &bytes,
            &cached,
            Pos(0, -64, 0),
            None,
        )?;
        cached.close()?;
        world::write_metadata(
            directory.path(),
            result.size()?,
            Pos(0, -64, 0),
            &Default::default(),
        )?;
        persist_world_output(output, directory.path())?;
        let loss = api::LossReport {
            bounds_or_origin: true,
            details: vec!["世界归档坐标从 (0,-64,0) 开始".into()],
            ..Default::default()
        };
        return Ok(Some(("bdx-stream", loss)));
    }
    if !matches!(
        first,
        FormatId::Schematic | FormatId::SchemV1 | FormatId::SchemV2 | FormatId::MCStructure
    ) {
        return Ok(None);
    }
    let mut last_error = None;
    for format in formats {
        match rust_water_structure::formats::java::dense::from_file_bounded(
            input,
            format,
            memory_budget,
        ) {
            Ok(data) => {
                let directory = tempfile::tempdir()?;
                world::write_dense_source_directory(directory.path(), &data, Pos(0, -64, 0), None)?;
                persist_world_output(output, directory.path())?;
                let loss = api::LossReport {
                    entities: data.loss().entities,
                    metadata: data.loss().metadata,
                    bounds_or_origin: data.origin() != Pos(0, -64, 0),
                    details: vec![
                        "世界归档坐标从 (0,-64,0) 开始；紧凑转换不输出普通实体和格式专有元数据"
                            .into(),
                    ],
                    ..Default::default()
                };
                return Ok(Some((
                    if data.is_paged() {
                        "paged-index"
                    } else {
                        "dense-index"
                    },
                    loss,
                )));
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| Error::invalid("找不到紧凑读取器")))
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Commands::Formats => {
            let formats = FormatId::ALL.map(|format| json!({"id":format as u8,"name":format.name(),"capabilities":api::capabilities(format)}));
            println!("{}", serde_json::to_string_pretty(formats.as_slice())?);
        }
        Commands::Inspect {
            input,
            format,
            bounds,
        } => {
            let start = Instant::now();
            let format = format.as_deref().map(FormatId::parse).transpose()?;
            let reader = Reader::open(input, format, parse_bounds(bounds)?)?;
            let value = summarize(reader.data(), start.elapsed().as_secs_f64())?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        Commands::Convert {
            input,
            output,
            to,
            from,
            bounds,
            offset,
            strict,
            memory_budget_mib,
        } => {
            reject_input_output_alias(&input, &output)?;
            let start = Instant::now();
            let from = from.as_deref().map(FormatId::parse).transpose()?;
            let format = FormatId::parse(&to)?;
            let memory_budget = memory_budget_mib
                .checked_mul(1024 * 1024)
                .and_then(|bytes| usize::try_from(bytes).ok())
                .filter(|bytes| *bytes > 0)
                .ok_or_else(|| Error::invalid("索引内存预算必须为正数且不能溢出"))?;
            if format == FormatId::MCWorld
                && !strict
                && offset.is_none()
                && let Some((engine, loss)) =
                    fast_world_conversion(&input, &output, from, memory_budget)?
            {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"output":output,"format":format.name(),"engine":engine,"loss":loss,"elapsed_seconds":start.elapsed().as_secs_f64()})
                    )?
                );
                return Ok(());
            }
            let mut reader = Reader::open(input, from, parse_bounds(bounds)?)?;
            if let Some(offset) = offset {
                reader.set_offset(parse_position(&offset)?)?;
            }
            let report = write_output(
                &output,
                format,
                reader.data(),
                if strict {
                    LossPolicy::Strict
                } else {
                    LossPolicy::Compatible
                },
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"output":output,"format":format.name(),"loss":report,"elapsed_seconds":start.elapsed().as_secs_f64()})
                )?
            );
        }
        Commands::MapArt {
            input,
            output,
            map_width,
            map_height,
            flat,
            no_reference_column,
            max_height,
            start,
            to,
        } => {
            reject_input_output_alias(&input, &output)?;
            let image = image::open(input).map_err(|error| Error::Other(error.to_string()))?;
            let options = map_art::Options {
                start_sub_chunk_pos: parse_position(&start)?,
                map_width,
                map_height,
                force_2d: flat,
                disable_reference_column: no_reference_column,
                max_3d_height: max_height,
            };
            let art = map_art::generate(&image, &options)?;
            let report = write_output(
                &output,
                FormatId::parse(&to)?,
                &art.structure,
                LossPolicy::Compatible,
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"output":output,"size":art.structure.size,"origin":art.structure.origin,"loss":report})
                )?
            );
        }
        Commands::Netease { action } => match action {
            NeteaseAction::Encrypt { path, buffer_size } => netease::encrypt(&path, buffer_size)?,
            NeteaseAction::Decrypt { path, buffer_size } => netease::decrypt(&path, buffer_size)?,
        },
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("错误: {error}");
        std::process::exit(1);
    }
}
