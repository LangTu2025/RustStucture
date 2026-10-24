use rust_water_structure::nbt::{Compound, Nbt};
use rust_water_structure::world::cache::{CacheLimits, CachedWorld};
use rust_water_structure::{BlockState, Pos};
use std::time::Duration;

#[test]
fn cached_blocks_layers_and_nbt_survive_close_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let world = CachedWorld::open(directory.path()).unwrap();
    let first = Pos(-17, -64, -1);
    let second = Pos(16, 319, 32);
    let stone = rust_water_structure::blocks::legacy("stone", 0);
    let water = rust_water_structure::blocks::legacy("water", 0);
    world.set_block(first, 0, stone.clone()).unwrap();
    world.set_block(first, 1, water.clone()).unwrap();
    world.set_block(second, 0, stone.clone()).unwrap();
    let nbt = Compound::from([("id".into(), Nbt::String("Chest".into()))]);
    world.set_block_nbt(first, nbt).unwrap();
    assert_eq!(world.load_block(first, 1).unwrap(), water);
    world.close().unwrap();
    world.close().unwrap();
    assert!(world.load_block(first, 0).is_err());

    let reopened = CachedWorld::open(directory.path()).unwrap();
    assert_eq!(reopened.load_block(first, 0).unwrap(), stone);
    assert_eq!(reopened.load_block(first, 1).unwrap(), water);
    assert_eq!(reopened.load_block(second, 0).unwrap(), stone);
    let nbt = reopened.load_block_nbt(first).unwrap().unwrap();
    assert_eq!(nbt["x"].as_i64(), Some(-17));
    assert_eq!(nbt["y"].as_i64(), Some(-64));
    assert_eq!(nbt["z"].as_i64(), Some(-1));
    reopened.close().unwrap();
}

#[test]
fn bounded_cache_evicts_dirty_entries_without_losing_edits() {
    let directory = tempfile::tempdir().unwrap();
    let world = CachedWorld::open_with_limits(
        directory.path(),
        CacheLimits {
            subchunks: 2,
            nbt_chunks: 2,
        },
    )
    .unwrap();
    let stone = rust_water_structure::blocks::legacy("stone", 0);
    for chunk in 0..8 {
        let pos = Pos(chunk * 16, chunk * 16, -chunk * 16);
        world.set_block(pos, 0, stone.clone()).unwrap();
        world
            .set_block_nbt(
                pos,
                Compound::from([("id".into(), Nbt::String("Chest".into()))]),
            )
            .unwrap();
        let stats = world.stats().unwrap();
        assert!(stats.subchunks <= 2);
        assert!(stats.nbt_chunks <= 2);
    }
    world.close().unwrap();
    let reopened = CachedWorld::open(directory.path()).unwrap();
    for chunk in 0..8 {
        let pos = Pos(chunk * 16, chunk * 16, -chunk * 16);
        assert_eq!(reopened.load_block(pos, 0).unwrap(), stone);
        assert_eq!(
            reopened.load_block_nbt(pos).unwrap().unwrap()["id"].as_str(),
            Some("Chest")
        );
    }
    reopened.close().unwrap();
}

#[test]
fn editing_existing_chunk_preserves_other_blocks_entities_and_extra_only_layer() {
    let directory = tempfile::tempdir().unwrap();
    let first = Pos(1, 3, 2);
    let second = Pos(2, 3, 2);
    let extra = Pos(3, 3, 2);
    let stone = rust_water_structure::blocks::legacy("stone", 0);
    let water = rust_water_structure::blocks::legacy("water", 0);
    let world = CachedWorld::open(directory.path()).unwrap();
    for pos in [first, second] {
        world.set_block(pos, 0, stone.clone()).unwrap();
        world
            .set_block_nbt(
                pos,
                Compound::from([("id".into(), Nbt::String("Chest".into()))]),
            )
            .unwrap();
    }
    world.set_block(extra, 1, water.clone()).unwrap();
    world.close().unwrap();
    let world = CachedWorld::open(directory.path()).unwrap();
    let glass = rust_water_structure::blocks::legacy("glass", 0);
    world.set_block(first, 0, glass.clone()).unwrap();
    world
        .set_block_nbt(
            first,
            Compound::from([("id".into(), Nbt::String("Barrel".into()))]),
        )
        .unwrap();
    world.close().unwrap();
    let world = CachedWorld::open(directory.path()).unwrap();
    assert_eq!(world.load_block(first, 0).unwrap(), glass);
    assert_eq!(world.load_block(second, 0).unwrap(), stone);
    assert_eq!(
        world.load_block_nbt(first).unwrap().unwrap()["id"].as_str(),
        Some("Barrel")
    );
    assert_eq!(
        world.load_block_nbt(second).unwrap().unwrap()["id"].as_str(),
        Some("Chest")
    );
    assert!(world.load_block(extra, 0).unwrap().is_air());
    assert_eq!(world.load_block(extra, 1).unwrap(), water);
    world.close().unwrap();
}

#[test]
fn periodic_flush_has_observable_completion_and_close_wakes_worker() {
    let directory = tempfile::tempdir().unwrap();
    let world = CachedWorld::open(directory.path()).unwrap();
    let generation = world.flush_generation().unwrap();
    world
        .set_block(Pos(0, 0, 0), 0, BlockState::new("stone"))
        .unwrap();
    world.auto_flush(Duration::from_millis(5)).unwrap();
    assert!(
        world
            .wait_for_flush(generation, Duration::from_secs(5))
            .unwrap()
            > generation
    );
    assert_eq!(world.stats().unwrap().dirty_subchunks, 0);
    world.auto_flush(Duration::from_secs(86_400)).unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || sender.send(world.close()).unwrap());
    receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("关闭必须唤醒周期刷新线程")
        .unwrap();
    handle.join().unwrap();
    CachedWorld::open(directory.path())
        .unwrap()
        .close()
        .unwrap();
}

#[test]
fn cache_rejects_invalid_bounds_and_propagates_corrupt_subchunks() {
    let directory = tempfile::tempdir().unwrap();
    assert!(
        CachedWorld::open_with_limits(
            directory.path(),
            CacheLimits {
                subchunks: 0,
                nbt_chunks: 1
            }
        )
        .is_err()
    );
    let world = CachedWorld::open(directory.path()).unwrap();
    assert!(
        world
            .set_block(Pos(0, -65, 0), 0, BlockState::air())
            .is_err()
    );
    assert!(world.load_block(Pos(0, 320, 0), 0).is_err());
    assert!(world.load_block(Pos(0, 0, 0), 2).is_err());
    assert!(world.auto_flush(Duration::ZERO).is_err());
    world.close().unwrap();
    let db = bedrock_leveldb::Db::open(directory.path().join("db"), Default::default()).unwrap();
    let mut key = vec![0; 8];
    key.extend_from_slice(&[b'/', 0]);
    db.put(key, vec![255], Default::default()).unwrap();
    db.flush().unwrap();
    drop(db);
    let world = CachedWorld::open(directory.path()).unwrap();
    assert!(world.load_block(Pos(0, 0, 0), 0).is_err());
    world.close().unwrap();
}

#[test]
fn direct_bdx_import_matches_canonical_decode_and_keeps_overwritten_nbt() {
    use rust_water_structure::FormatId;
    use rust_water_structure::bdump::{BlockReference, Command, write_command};
    use rust_water_structure::formats::binary::{self, import_bdx_to_world};
    use std::io::Write;
    let mut body = b"BDX\0fixture\0".to_vec();
    let nbt = Compound::from([
        ("id".into(), Nbt::String("Chest".into())),
        ("CustomName".into(), Nbt::String("keep".into())),
    ]);
    for command in [
        Command::Constant("chest".into()),
        Command::Constant("[]".into()),
        Command::Constant("stone".into()),
        Command::Move { axis: 0, delta: -2 },
        Command::Place {
            block: BlockReference::States { name: 0, states: 1 },
            nbt: Some(nbt),
        },
        Command::Place {
            block: BlockReference::Legacy { name: 2, data: 0 },
            nbt: None,
        },
        Command::Move { axis: 0, delta: 4 },
        Command::Place {
            block: BlockReference::Legacy { name: 2, data: 0 },
            nbt: None,
        },
        Command::Terminate,
    ] {
        write_command(&mut body, &command).unwrap();
    }
    body.push(b'E');
    let mut bytes = b"BD@".to_vec();
    {
        let mut writer = brotli::CompressorWriter::new(&mut bytes, 4096, 4, 22);
        writer.write_all(&body).unwrap();
    }
    let expected = binary::decode(FormatId::BDX, &bytes).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let cached = CachedWorld::open(directory.path()).unwrap();
    let start = Pos(-100, -64, 50);
    let result = import_bdx_to_world(&bytes, &cached, start, None).unwrap();
    assert_eq!(result.min, Pos(-2, 0, 0));
    assert_eq!(result.max, Pos(2, 0, 0));
    assert_eq!(result.blocks, 3);
    assert_eq!(result.size().unwrap(), expected.size);
    cached.close().unwrap();
    let reopened = CachedWorld::open(directory.path()).unwrap();
    for (pos, block) in expected.blocks {
        let world_pos = Pos(start.0 + pos.0, start.1 + pos.1, start.2 + pos.2);
        assert_eq!(reopened.load_block(world_pos, 0).unwrap(), block.state);
        let actual_nbt = reopened.load_block_nbt(world_pos).unwrap();
        if let Some(expected_nbt) = block.nbt {
            let actual_nbt = actual_nbt.unwrap();
            for (name, value) in expected_nbt {
                assert_eq!(actual_nbt[&name], value);
            }
        } else {
            assert!(actual_nbt.is_none());
        }
    }
    reopened.close().unwrap();
}

#[test]
fn direct_bdx_import_reports_progress_and_honors_cancellation_and_height() {
    use rust_water_structure::formats::binary::{self, import_bdx_to_world};
    use rust_water_structure::{Block, FormatId, Size, StructureData};
    use std::sync::{Arc, Mutex, atomic::Ordering};
    let mut data = StructureData::new(FormatId::BDX);
    data.size = Size(1, 1, 1);
    data.blocks
        .insert(Pos(0, 0, 0), Block::new(BlockState::new("stone")));
    let bytes = binary::encode(FormatId::BDX, &data).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let cached = CachedWorld::open(directory.path()).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let observer = Arc::clone(&events);
    let control = rust_water_structure::world::Control {
        progress: Some(Arc::new(move |current, total| {
            observer.lock().unwrap().push((current, total))
        })),
        ..Default::default()
    };
    let result = import_bdx_to_world(&bytes, &cached, Pos(0, 0, 0), Some(&control)).unwrap();
    assert_eq!(events.lock().unwrap().first(), Some(&(0, result.commands)));
    assert_eq!(
        events.lock().unwrap().last(),
        Some(&(result.commands, result.commands))
    );
    control.cancelled.store(true, Ordering::Relaxed);
    assert!(matches!(
        import_bdx_to_world(&bytes, &cached, Pos(2, 0, 0), Some(&control)),
        Err(rust_water_structure::Error::Cancelled)
    ));
    assert!(cached.load_block(Pos(2, 0, 0), 0).unwrap().is_air());
    assert!(
        import_bdx_to_world(&bytes, &cached, Pos(0, 320, 0), None)
            .unwrap_err()
            .to_string()
            .contains("[-64,319]")
    );
    cached.close().unwrap();
}

#[test]
fn real_bdx_direct_import_matches_decoded_blocks_and_nbt_after_reopen() {
    use rust_water_structure::FormatId;
    use rust_water_structure::formats::binary::{self, import_bdx_to_world};
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../WaterStructure/test/测试文件/七天签到(1).bdx");
    if !source.exists() {
        return;
    }
    let bytes = std::fs::read(source).unwrap();
    let expected = binary::decode(FormatId::BDX, &bytes).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let cached = CachedWorld::open(directory.path()).unwrap();
    let start = Pos(-16, -64, 32);
    let result = import_bdx_to_world(&bytes, &cached, start, None).unwrap();
    assert_eq!(result.min, expected.origin);
    assert_eq!(result.size().unwrap(), expected.size);
    cached.close().unwrap();
    let reopened = CachedWorld::open(directory.path()).unwrap();
    for (pos, block) in expected.blocks {
        let pos = Pos(pos.0 + start.0, pos.1 + start.1, pos.2 + start.2);
        assert_eq!(reopened.load_block(pos, 0).unwrap(), block.state);
        if let Some(expected_nbt) = block.nbt {
            let actual = reopened.load_block_nbt(pos).unwrap().unwrap();
            for (name, value) in expected_nbt {
                assert_eq!(actual[&name], value);
            }
            assert_eq!(actual["x"].as_i64(), Some(i64::from(pos.0)));
            assert_eq!(actual["y"].as_i64(), Some(i64::from(pos.1)));
            assert_eq!(actual["z"].as_i64(), Some(i64::from(pos.2)));
        }
    }
    reopened.close().unwrap();
}
