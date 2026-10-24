//! 与 Go MapArt 相同的 Lanczos、色板匹配与高度步进算法。
use crate::nbt::Compound;
use crate::{Block, Error, FormatId, Pos, Result, StructureData, blocks, color};
use image::{DynamicImage, Rgba, RgbaImage};
use std::collections::{BTreeMap, HashMap};
use std::f64::consts::PI;
use std::path::Path;
use std::sync::OnceLock;

pub const HEIGHT_HIGHER: u8 = 1;
pub const HEIGHT_MIDDLE: u8 = 2;
pub const HEIGHT_LOWER: u8 = 3;

#[derive(Debug, Clone)]
pub struct Options {
    pub start_sub_chunk_pos: Pos,
    pub map_width: u32,
    pub map_height: u32,
    pub disable_reference_column: bool,
    pub force_2d: bool,
    pub max_3d_height: i32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            start_sub_chunk_pos: Pos(0, -4, 0),
            map_width: 1,
            map_height: 1,
            disable_reference_column: false,
            force_2d: false,
            max_3d_height: 0,
        }
    }
}

pub struct MapArt {
    pub structure: StructureData,
    pub resized: RgbaImage,
    pub height_modes: Vec<u8>,
    pub heights: Vec<i32>,
}

pub struct PixelModes {
    pub runtime_ids: Vec<u32>,
    pub height_modes: Vec<u8>,
}

fn pixel_count(width: u32, height: u32) -> Result<usize> {
    if width == 0 || height == 0 {
        return Err(Error::invalid("图像尺寸必须大于零"));
    }
    usize::try_from(u64::from(width) * u64::from(height))
        .map_err(|_| Error::invalid("图像像素数量溢出"))
}

fn blank_image(width: u32, height: u32) -> Result<RgbaImage> {
    let count = pixel_count(width, height)?
        .checked_mul(4)
        .ok_or_else(|| Error::invalid("图像缓冲区大小溢出"))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(count)
        .map_err(|_| Error::invalid("无法分配图像缓冲区"))?;
    bytes.resize(count, 0);
    RgbaImage::from_raw(width, height, bytes).ok_or_else(|| Error::invalid("图像缓冲区尺寸错误"))
}

fn sinc(value: f64) -> f64 {
    if value == 0.0 {
        1.0
    } else {
        (PI * value).sin() / (PI * value)
    }
}

fn weights(destination: u32, source: u32) -> Vec<Vec<(u32, f64)>> {
    let ratio = f64::from(source) / f64::from(destination);
    let scale = ratio.max(1.0);
    let radius = (scale * 3.0).ceil();
    (0..destination)
        .map(|position| {
            let center = (f64::from(position) + 0.5) * ratio - 0.5;
            let begin = (center - radius).ceil().max(0.0) as u32;
            let end = (center + radius).floor().min(f64::from(source - 1)) as u32;
            let mut values = Vec::new();
            let mut sum = 0.0;
            for source_position in begin..=end {
                let distance = ((f64::from(source_position) - center) / scale).abs();
                let weight = if distance < 3.0 {
                    sinc(distance) * sinc(distance / 3.0)
                } else {
                    0.0
                };
                if weight != 0.0 {
                    sum += weight;
                    values.push((source_position, weight));
                }
            }
            if sum != 0.0 {
                for (_, weight) in &mut values {
                    *weight /= sum;
                }
            }
            values
        })
        .collect()
}

fn clamp(value: f64) -> u8 {
    (value + 0.5).trunc().clamp(0.0, 255.0) as u8
}

fn resample_pixel(values: &[(u32, f64)], sample: impl Fn(u32) -> Rgba<u8>) -> Rgba<u8> {
    let mut channels = [0.0; 4];
    for (index, weight) in values {
        let pixel = sample(*index);
        let alpha_weight = f64::from(pixel[3]) * weight;
        for channel in 0..3 {
            channels[channel] += f64::from(pixel[channel]) * alpha_weight;
        }
        channels[3] += alpha_weight;
    }
    if channels[3] == 0.0 {
        return Rgba([0; 4]);
    }
    let inverse = 1.0 / channels[3];
    Rgba([
        clamp(channels[0] * inverse),
        clamp(channels[1] * inverse),
        clamp(channels[2] * inverse),
        clamp(channels[3]),
    ])
}

/// 使用 Go imaging.Lanczos 的两遍处理顺序和逐遍 uint8 舍入。
pub fn resize_lanczos(image: &RgbaImage, width: u32, height: u32) -> Result<RgbaImage> {
    pixel_count(image.width(), image.height())?;
    pixel_count(width, height)?;
    let mut resized = if width == image.width() {
        image.clone()
    } else {
        let weights = weights(width, image.width());
        let mut output = blank_image(width, image.height())?;
        for y in 0..image.height() {
            for (x, weights) in weights.iter().enumerate() {
                output.put_pixel(
                    x as u32,
                    y,
                    resample_pixel(weights, |source| *image.get_pixel(source, y)),
                );
            }
        }
        output
    };
    if height != image.height() {
        let weights = weights(height, image.height());
        let mut output = blank_image(width, height)?;
        for x in 0..width {
            for (y, weights) in weights.iter().enumerate() {
                output.put_pixel(
                    x,
                    y as u32,
                    resample_pixel(weights, |source| *resized.get_pixel(x, source)),
                );
            }
        }
        resized = output;
    }
    Ok(resized)
}

#[derive(Clone, Copy)]
struct BlockMode {
    runtime_id: u32,
    height_mode: u8,
}

#[derive(Default)]
struct Palette {
    mapping: Vec<[u8; 3]>,
    modes: BTreeMap<[u8; 3], BlockMode>,
    mapping_2d: Vec<[u8; 3]>,
    modes_2d: BTreeMap<[u8; 3], BlockMode>,
}

type PaletteEntry = ([u8; 4], [u8; 4], [u8; 4], String);

fn palette() -> &'static Palette {
    static PALETTE: OnceLock<Palette> = OnceLock::new();
    PALETTE.get_or_init(|| {
        let entries: Vec<PaletteEntry> =
            serde_json::from_slice(include_bytes!("../assets/colors.json"))
                .expect("内置 MapArt 色板损坏");
        let mut palette = Palette::default();
        for (higher, middle, lower, name) in entries {
            let name = match name.as_str() {
                "minecraft:stained_hardened_clay" | "minecraft:hardened_clay" => {
                    "minecraft:terracotta"
                }
                _ => &name,
            };
            let state = blocks::bedrock(name, Compound::new());
            if state.name == "minecraft:unknown" {
                continue;
            }
            let runtime_id = blocks::runtime_id(&state);
            for (rgba, height_mode) in [
                (higher, HEIGHT_HIGHER),
                (middle, HEIGHT_MIDDLE),
                (lower, HEIGHT_LOWER),
            ] {
                let rgb = [rgba[0], rgba[1], rgba[2]];
                // 原色板保留输入次序；重复 RGB 的方块模式由后一个条目覆盖。
                palette.mapping.push(rgb);
                palette.modes.insert(
                    rgb,
                    BlockMode {
                        runtime_id,
                        height_mode,
                    },
                );
            }
            let rgb = [middle[0], middle[1], middle[2]];
            palette.mapping_2d.push(rgb);
            palette.modes_2d.insert(
                rgb,
                BlockMode {
                    runtime_id,
                    height_mode: HEIGHT_MIDDLE,
                },
            );
        }
        palette
    })
}

/// 返回按 x * height + z 排列的方块与明暗模式。
pub fn map_pixels_to_modes(image: &RgbaImage, force_2d: bool) -> Result<PixelModes> {
    let count = pixel_count(image.width(), image.height())?;
    let palette = palette();
    let (colors, modes) = if force_2d {
        (&palette.mapping_2d, &palette.modes_2d)
    } else {
        (&palette.mapping, &palette.modes)
    };
    if colors.is_empty() {
        return Err(Error::invalid("MapArt 色板为空"));
    }
    let mut output = PixelModes {
        runtime_ids: vec![0; count],
        height_modes: vec![0; count],
    };
    let mut cache = HashMap::<[u8; 3], BlockMode>::new();
    for y in 0..image.height() {
        for x in 0..image.width() {
            let pixel = image.get_pixel(x, y);
            let rgb = [pixel[0], pixel[1], pixel[2]];
            let mode = *cache.entry(rgb).or_insert_with(|| {
                let closest = color::nearest(rgb, colors).expect("非空 MapArt 色板");
                modes[&closest]
            });
            let index = (u64::from(x) * u64::from(image.height()) + u64::from(y)) as usize;
            output.runtime_ids[index] = mode.runtime_id;
            output.height_modes[index] = mode.height_mode;
        }
    }
    Ok(output)
}

/// 计算包含北侧参考列的相对高度；关闭参考列时仅用实际像素求最小值。
pub fn compute_heights(width: u32, height: u32, modes: &[u8], reference: bool) -> Result<Vec<i32>> {
    let count = pixel_count(width, height)?;
    if modes.len() != count
        || modes
            .iter()
            .any(|mode| !(HEIGHT_HIGHER..=HEIGHT_LOWER).contains(mode))
    {
        return Err(Error::invalid("MapArt 高度模式或数组长度无效"));
    }
    let stride = height as usize + 1;
    let mut output = vec![
        0_i32;
        (width as usize)
            .checked_mul(stride)
            .ok_or_else(|| Error::invalid("MapArt 高度缓冲区溢出"))?
    ];
    for x in 0..width as usize {
        let even_x = x % 2 == 0;
        output[x * stride] = match modes[x * height as usize] {
            HEIGHT_HIGHER => {
                if even_x {
                    -2
                } else {
                    -1
                }
            }
            HEIGHT_MIDDLE => 0,
            HEIGHT_LOWER => {
                if even_x {
                    1
                } else {
                    2
                }
            }
            _ => unreachable!(),
        };
        for z in 2..stride {
            let previous_mode = modes[x * height as usize + z - 2];
            let current_mode = modes[x * height as usize + z - 1];
            let last = output[x * stride + z - 1];
            let previous = output[x * stride + z - 2];
            let same_parity = even_x == (z % 2 == 0);
            let delta = match current_mode {
                HEIGHT_MIDDLE => 0,
                HEIGHT_HIGHER if previous_mode == HEIGHT_HIGHER => {
                    if last - previous >= 2 {
                        1
                    } else {
                        2
                    }
                }
                HEIGHT_HIGHER => {
                    if same_parity {
                        1
                    } else {
                        2
                    }
                }
                HEIGHT_LOWER if previous_mode == HEIGHT_LOWER => {
                    if previous - last == 1 {
                        -2
                    } else {
                        -1
                    }
                }
                HEIGHT_LOWER => {
                    if same_parity {
                        -2
                    } else {
                        -1
                    }
                }
                _ => unreachable!(),
            };
            output[x * stride + z] = last
                .checked_add(delta)
                .ok_or_else(|| Error::invalid("MapArt 高度溢出"))?;
        }
    }
    let begin = usize::from(!reference);
    let minimum = (0..width as usize)
        .flat_map(|x| &output[x * stride + begin..(x + 1) * stride])
        .copied()
        .min()
        .unwrap_or_default();
    for value in &mut output {
        *value = value
            .checked_sub(minimum)
            .ok_or_else(|| Error::invalid("MapArt 高度归一化溢出"))?;
    }
    Ok(output)
}

pub fn limit_heights(
    heights: &mut [i32],
    width: u32,
    height: u32,
    reference: bool,
    max_height: i32,
) -> Result<()> {
    pixel_count(width, height)?;
    let stride = height as usize + 1;
    if heights.len() != width as usize * stride {
        return Err(Error::invalid("MapArt 高度数组长度错误"));
    }
    if max_height <= 0 {
        return Ok(());
    }
    let begin = usize::from(!reference);
    let maximum = (0..width as usize)
        .flat_map(|x| &heights[x * stride + begin..(x + 1) * stride])
        .copied()
        .max()
        .unwrap_or_default()
        .max(0);
    let allowed = max_height - 1;
    if maximum <= allowed || maximum == 0 {
        return Ok(());
    }
    for x in 0..width as usize {
        for z in begin..stride {
            let value = &mut heights[x * stride + z];
            *value = (i64::from(*value) * i64::from(allowed) / i64::from(maximum)) as i32;
        }
    }
    Ok(())
}

pub fn generate(image: &DynamicImage, options: &Options) -> Result<MapArt> {
    let width = options
        .map_width
        .checked_mul(128)
        .filter(|value| *value <= i32::MAX as u32)
        .ok_or_else(|| Error::invalid("MapArt 宽度溢出"))?;
    let height = options
        .map_height
        .checked_mul(128)
        .filter(|value| *value <= i32::MAX as u32)
        .ok_or_else(|| Error::invalid("MapArt 高度溢出"))?;
    let resized = resize_lanczos(&image.to_rgba8(), width, height)?;
    let modes = map_pixels_to_modes(&resized, options.force_2d)?;
    let reference = !options.disable_reference_column;
    let mut heights = if options.force_2d {
        vec![0; width as usize * (height as usize + 1)]
    } else {
        let mut heights = compute_heights(width, height, &modes.height_modes, reference)?;
        let max_height = if options.max_3d_height == 0 {
            384
        } else {
            options.max_3d_height
        };
        if max_height < 1 {
            return Err(Error::invalid("MapArt 最大高度必须为正"));
        }
        limit_heights(&mut heights, width, height, reference, max_height)?;
        heights
    };
    let mut structure = StructureData::new(FormatId::MCStructure);
    let base = Pos(
        options
            .start_sub_chunk_pos
            .0
            .checked_mul(16)
            .ok_or_else(|| Error::invalid("MapArt 起点 X 溢出"))?,
        options
            .start_sub_chunk_pos
            .1
            .checked_mul(16)
            .ok_or_else(|| Error::invalid("MapArt 起点 Y 溢出"))?,
        options
            .start_sub_chunk_pos
            .2
            .checked_mul(16)
            .ok_or_else(|| Error::invalid("MapArt 起点 Z 溢出"))?,
    );
    let emerald = blocks::bedrock("minecraft:emerald_block", Compound::new());
    for x in 0..width {
        let start_z = u32::from(!reference);
        for z_index in start_z..=height {
            let index = x as usize * (height as usize + 1) + z_index as usize;
            let y = base
                .1
                .checked_add(heights[index])
                .ok_or_else(|| Error::invalid("MapArt Y 溢出"))?;
            if !(i16::MIN as i32..=i16::MAX as i32).contains(&y) {
                return Err(Error::invalid("MapArt Y 超出 int16 范围"));
            }
            let state = if z_index == 0 {
                emerald.clone()
            } else {
                blocks::from_runtime_id(
                    modes.runtime_ids[x as usize * height as usize + z_index as usize - 1],
                )
            };
            let pos = Pos(
                base.0
                    .checked_add(x as i32)
                    .ok_or_else(|| Error::invalid("MapArt X 溢出"))?,
                y,
                base.2
                    .checked_add(z_index as i32 - 1)
                    .ok_or_else(|| Error::invalid("MapArt Z 溢出"))?,
            );
            structure.insert(pos, Block::new(state));
        }
    }
    structure.normalize()?;
    Ok(MapArt {
        structure,
        resized,
        height_modes: modes.height_modes,
        heights: std::mem::take(&mut heights),
    })
}

pub fn generate_to_directory(
    path: &Path,
    image: &DynamicImage,
    options: &Options,
) -> Result<MapArt> {
    let result = generate(image, options)?;
    crate::world::write_directory(path, &result.structure, result.structure.origin, None)?;
    Ok(result)
}
