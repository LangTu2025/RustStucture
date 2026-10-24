use image::{DynamicImage, Rgba, RgbaImage};
use rust_water_structure::map_art::{self, Options};
use rust_water_structure::{Pos, Size};
use serde_json::{Value, json};

#[test]
fn manual_height_modes_match_reference_steps_and_normalization() {
    let modes = [1, 1, 1, 3];
    let heights = map_art::compute_heights(1, 4, &modes, true).unwrap();
    assert_eq!(heights, vec![0, 2, 3, 5, 3]);
    assert_eq!(
        map_art::compute_heights(1, 4, &modes, false).unwrap(),
        vec![-2, 0, 1, 3, 1]
    );
    let mut limited = heights;
    map_art::limit_heights(&mut limited, 1, 4, true, 3).unwrap();
    assert_eq!(limited, vec![0, 0, 1, 2, 1]);
    assert!(map_art::compute_heights(1, 4, &[0, 1, 2, 3], true).is_err());
    assert!(map_art::compute_heights(2, 4, &modes, true).is_err());
}

#[test]
fn lanczos_keeps_pixels_when_dimensions_are_unchanged() {
    let image = RgbaImage::from_raw(2, 1, vec![17, 42, 93, 0, 201, 77, 32, 128]).unwrap();
    assert_eq!(map_art::resize_lanczos(&image, 2, 1).unwrap(), image);
    assert!(map_art::resize_lanczos(&image, 0, 1).is_err());
}

#[test]
fn lanczos_alpha_rounding_matches_go_golden_pixels() {
    let image = RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
    let resized = map_art::resize_lanczos(&image, 3, 1).unwrap();
    // 来自原 Go imaging.Lanczos；负瓣使末端 alpha 降到 112。
    assert_eq!(
        resized.as_raw(),
        &[255, 0, 0, 255, 170, 0, 85, 192, 0, 0, 255, 112]
    );
    let modes = map_art::map_pixels_to_modes(&resized, false).unwrap();
    assert_eq!(modes.runtime_ids, vec![512666773, 240216761, 3444948044]);
    assert_eq!(modes.height_modes, vec![1, 2, 1]);
    assert_eq!(
        map_art::compute_heights(3, 1, &modes.height_modes, true).unwrap(),
        vec![0, 2, 2, 2, 0, 2]
    );
}

#[test]
fn force_2d_generates_map_pixels_and_reference_at_requested_coordinates() {
    let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([75, 79, 79, 255])));
    let options = Options {
        start_sub_chunk_pos: Pos(-1, -4, 2),
        force_2d: true,
        ..Options::default()
    };
    let generated = map_art::generate(&image, &options).unwrap();
    assert_eq!(generated.resized.dimensions(), (128, 128));
    assert_eq!(generated.structure.origin, Pos(-16, -64, 31));
    assert_eq!(generated.structure.size, Size(128, 1, 129));
    assert_eq!(generated.structure.blocks.len(), 128 * 129);
    assert_eq!(
        generated.structure.blocks[&Pos(0, 0, 0)].state.name,
        "minecraft:emerald_block"
    );
    assert!(generated.structure.blocks.keys().all(|pos| pos.1 == 0));
    assert!(generated.height_modes.iter().all(|mode| *mode == 2));
}

#[test]
fn map_art_resize_modes_and_heights_match_original_go_algorithm() {
    use std::io::Write;
    use std::process::Stdio;
    let oracle = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("work/map-art-oracle.exe");
    if !oracle.exists() {
        return;
    }
    for (width, height, target_width, target_height) in
        [(3, 2, 7, 5), (7, 5, 3, 2), (4, 3, 4, 3), (3, 4, 128, 128)]
    {
        let mut image = RgbaImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                image.put_pixel(
                    x,
                    y,
                    Rgba([
                        ((x * 37 + y * 13) % 255) as u8,
                        ((y * 71 + x * 17) % 255) as u8,
                        (((x + y) * 53) % 255) as u8,
                        ((x * 91 + y * 137) % 256) as u8,
                    ]),
                );
            }
        }
        for (flat, reference, max_height) in
            [(false, true, 384), (false, false, 5), (true, true, 1)]
        {
            let pixels: Vec<[u8; 4]> = image.pixels().map(|pixel| pixel.0).collect();
            let request = json!({"Width":width,"Height":height,"TargetWidth":target_width,"TargetHeight":target_height,
                "Pixels":pixels,"Force2D":flat,"Reference":reference,"MaxHeight":max_height});
            let mut child = std::process::Command::new(&oracle)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(&serde_json::to_vec(&request).unwrap())
                .unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(result.status.success());
            let expected: Value = serde_json::from_slice(&result.stdout).unwrap();
            let resized = map_art::resize_lanczos(&image, target_width, target_height).unwrap();
            assert_eq!(
                json!(resized.as_raw()),
                expected["pixels"],
                "Lanczos {width}x{height}->{target_width}x{target_height}"
            );
            let modes = map_art::map_pixels_to_modes(&resized, flat).unwrap();
            assert_eq!(json!(modes.height_modes), expected["modes"], "height mode");
            assert_eq!(
                json!(modes.runtime_ids),
                expected["runtime_ids"],
                "runtime id"
            );
            let mut heights = map_art::compute_heights(
                target_width,
                target_height,
                &modes.height_modes,
                reference,
            )
            .unwrap();
            if flat {
                heights.fill(0);
            } else {
                map_art::limit_heights(
                    &mut heights,
                    target_width,
                    target_height,
                    reference,
                    max_height,
                )
                .unwrap();
            }
            assert_eq!(json!(heights), expected["heights"], "height layout");
        }
    }
}

#[test]
fn generated_3d_block_positions_follow_go_height_layout() {
    let image = RgbaImage::from_fn(3, 4, |x, y| {
        Rgba([
            ((x * 79 + y * 31) % 256) as u8,
            ((x * 17 + y * 97) % 256) as u8,
            ((x * 43 + y * 13) % 256) as u8,
            255,
        ])
    });
    for reference in [true, false] {
        let options = Options {
            start_sub_chunk_pos: Pos(-2, -4, 3),
            disable_reference_column: !reference,
            max_3d_height: 5,
            ..Options::default()
        };
        let generated =
            map_art::generate(&DynamicImage::ImageRgba8(image.clone()), &options).unwrap();
        let modes = map_art::map_pixels_to_modes(&generated.resized, false).unwrap();
        assert_eq!(
            generated.structure.origin,
            Pos(-32, -64, if reference { 47 } else { 48 })
        );
        assert!(generated.structure.size.1 <= 5);
        assert_eq!(
            generated.structure.blocks.len(),
            128 * if reference { 129 } else { 128 }
        );
        for x in 0..128 {
            for z_index in usize::from(!reference)..=128 {
                let y = generated.heights[x * 129 + z_index];
                let pos = Pos(x as i32, y, z_index as i32 - i32::from(!reference));
                let block = &generated.structure.blocks[&pos];
                if z_index == 0 {
                    assert_eq!(block.state.name, "minecraft:emerald_block");
                } else {
                    assert_eq!(
                        rust_water_structure::blocks::runtime_id(&block.state),
                        modes.runtime_ids[x * 128 + z_index - 1]
                    );
                }
            }
        }
    }
}
