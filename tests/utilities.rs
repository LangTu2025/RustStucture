use rust_water_structure::{color, netease};

#[test]
fn netease_encryption_keeps_all_file_bytes_after_decrypt() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("CURRENT"), b"MANIFEST-000001\n").unwrap();
    std::fs::write(temp.path().join("MANIFEST-000001"), b"hello manifest").unwrap();
    let payload = (0..=255u8).cycle().take(10003).collect::<Vec<_>>();
    std::fs::write(temp.path().join("000002.ldb"), &payload).unwrap();
    netease::encrypt(temp.path(), 17).unwrap();
    assert_ne!(
        std::fs::read(temp.path().join("000002.ldb")).unwrap(),
        payload
    );
    netease::decrypt(temp.path(), 13).unwrap();
    assert_eq!(
        std::fs::read(temp.path().join("000002.ldb")).unwrap(),
        payload
    );
    assert_eq!(
        std::fs::read(temp.path().join("CURRENT")).unwrap(),
        b"MANIFEST-000001\n"
    );
}

#[test]
fn rgba_preserves_signed_alpha() {
    let encoded = color::encode_rgba([12, 34, 56, 255]);
    assert!(encoded < 0);
    assert_eq!(color::decode_rgba(encoded), [12, 34, 56, 255]);
}

fn recovery_world() -> (tempfile::TempDir, Vec<u8>) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("CURRENT"), b"MANIFEST-000001\n").unwrap();
    std::fs::write(
        directory.path().join("MANIFEST-000001"),
        b"manifest original",
    )
    .unwrap();
    let payload = b"first database table payload".to_vec();
    std::fs::write(directory.path().join("000002.ldb"), &payload).unwrap();
    std::fs::write(directory.path().join("000003.log"), b"second original log").unwrap();
    (directory, payload)
}

#[test]
fn netease_encrypt_recovers_after_a_real_file_has_been_replaced() {
    let (directory, payload) = recovery_world();
    let error = netease::encrypt_with_callback(directory.path(), 7, |_| {
        Err(std::io::Error::other("模拟转换后的I/O失败").into())
    })
    .unwrap_err();
    assert!(error.to_string().contains("I/O失败"));
    assert!(directory.path().join(netease::RECOVERY_FILE).is_file());
    assert_ne!(
        std::fs::read(directory.path().join("000002.ldb")).unwrap(),
        payload
    );
    assert_eq!(
        std::fs::read(directory.path().join("000003.log")).unwrap(),
        b"second original log"
    );
    assert_eq!(
        std::fs::read(directory.path().join("CURRENT")).unwrap(),
        b"MANIFEST-000001\n"
    );
    let recovery = std::fs::read(directory.path().join(netease::RECOVERY_FILE)).unwrap();
    netease::encrypt(directory.path(), 3).unwrap();
    assert_eq!(
        std::fs::read(directory.path().join(netease::RECOVERY_FILE)).unwrap(),
        recovery
    );
    netease::decrypt(directory.path(), 11).unwrap();
    assert_eq!(
        std::fs::read(directory.path().join("000002.ldb")).unwrap(),
        payload
    );
    assert_eq!(
        std::fs::read(directory.path().join("000003.log")).unwrap(),
        b"second original log"
    );
    assert_eq!(
        std::fs::read(directory.path().join(netease::RECOVERY_FILE)).unwrap(),
        recovery
    );
}

#[test]
fn netease_decrypt_recovers_with_plain_current_and_skips_completed_files() {
    let (directory, payload) = recovery_world();
    netease::encrypt(directory.path(), 5).unwrap();
    netease::decrypt_with_callback(directory.path(), 9, |_| {
        Err(std::io::Error::other("模拟解密中断").into())
    })
    .unwrap_err();
    assert_eq!(
        std::fs::read(directory.path().join("000002.ldb")).unwrap(),
        payload
    );
    assert!(
        std::fs::read(directory.path().join("CURRENT"))
            .unwrap()
            .starts_with(&[0x80, 0x1d, 0x30, 0x01])
    );
    // 模拟旧实现已经将CURRENT变成明文；恢复必须能够从记录取回密钥。
    std::fs::write(directory.path().join("CURRENT"), b"MANIFEST-000001\n").unwrap();
    netease::decrypt(directory.path(), 2).unwrap();
    assert_eq!(
        std::fs::read(directory.path().join("000002.ldb")).unwrap(),
        payload
    );
    assert_eq!(
        std::fs::read(directory.path().join("000003.log")).unwrap(),
        b"second original log"
    );
    assert_eq!(
        std::fs::read(directory.path().join("MANIFEST-000001")).unwrap(),
        b"manifest original"
    );
}

#[test]
fn netease_refuses_mismatched_or_foreign_recovery_records_before_changes() {
    let (directory, _) = recovery_world();
    std::fs::write(
        directory.path().join(netease::RECOVERY_FILE),
        b"existing user file",
    )
    .unwrap();
    let before = std::fs::read(directory.path().join("000002.ldb")).unwrap();
    assert!(netease::encrypt(directory.path(), 3).is_err());
    assert_eq!(
        std::fs::read(directory.path().join("000002.ldb")).unwrap(),
        before
    );
    assert_eq!(
        std::fs::read(directory.path().join(netease::RECOVERY_FILE)).unwrap(),
        b"existing user file"
    );

    let (directory, _) = recovery_world();
    netease::encrypt(directory.path(), 3).unwrap();
    let path = directory.path().join(netease::RECOVERY_FILE);
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["key"][0] = serde_json::json!(record["key"][0].as_u64().unwrap() ^ 1);
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let before = std::fs::read(directory.path().join("000002.ldb")).unwrap();
    assert!(netease::decrypt(directory.path(), 3).is_err());
    assert_eq!(
        std::fs::read(directory.path().join("000002.ldb")).unwrap(),
        before
    );
}

#[test]
fn netease_decrypts_existing_encrypted_directory_without_recovery_record() {
    let directory = tempfile::tempdir().unwrap();
    // 独立夹具：密钥为0..15，按旧格式XOR后附加固定文件头。
    let current = [
        0x80, 0x1d, 0x30, 0x01, 0x4d, 0x40, 0x4c, 0x4a, 0x42, 0x40, 0x55, 0x53, 0x25, 0x39, 0x3a,
        0x3b, 0x3c, 0x3d, 0x3f, 0x05,
    ];
    let table = [
        0x80, 0x1d, 0x30, 0x01, 0x65, 0x79, 0x76, 0x66, 0x76, 0x6b, 0x67, 0x6b, 0x28, 0x7d, 0x6b,
        0x69, 0x60, 0x68, 0x2e, 0x6c, 0x6f, 0x6f, 0x76, 0x66, 0x6a, 0x71,
    ];
    std::fs::write(directory.path().join("CURRENT"), current).unwrap();
    std::fs::write(
        directory.path().join("MANIFEST-000001"),
        b"manifest original",
    )
    .unwrap();
    std::fs::write(directory.path().join("000002.ldb"), table).unwrap();

    netease::decrypt(directory.path(), 3).unwrap();

    assert_eq!(
        std::fs::read(directory.path().join("000002.ldb")).unwrap(),
        b"external table content"
    );
    assert_eq!(
        std::fs::read(directory.path().join("CURRENT")).unwrap(),
        b"MANIFEST-000001\n"
    );
    assert!(directory.path().join(netease::RECOVERY_FILE).is_file());
}

#[test]
fn netease_reencrypts_edited_world_after_manifest_rotation() {
    for manifest in ["MANIFEST-000002", "MANIFEST-1000000"] {
        let (directory, payload) = recovery_world();
        netease::encrypt(directory.path(), 3).unwrap();
        netease::decrypt(directory.path(), 5).unwrap();
        let record_path = directory.path().join(netease::RECOVERY_FILE);
        let old: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();

        std::fs::remove_file(directory.path().join("MANIFEST-000001")).unwrap();
        std::fs::write(directory.path().join(manifest), b"new manifest content").unwrap();
        std::fs::write(directory.path().join("CURRENT"), format!("{manifest}\n")).unwrap();
        netease::encrypt_with_callback(directory.path(), 7, |_| {
            let record: serde_json::Value = serde_json::from_slice(&std::fs::read(&record_path)?)?;
            assert_eq!(record["manifest"], manifest);
            assert_eq!(record["key"].as_array().unwrap().len(), manifest.len() + 1);
            if manifest.len() == "MANIFEST-000001".len() {
                assert_eq!(record["key"], old["key"]);
            }
            Ok(())
        })
        .unwrap();
        netease::decrypt(directory.path(), 2).unwrap();
        assert_eq!(
            std::fs::read(directory.path().join("000002.ldb")).unwrap(),
            payload
        );
        assert_eq!(
            std::fs::read(directory.path().join(manifest)).unwrap(),
            b"new manifest content"
        );
        assert_eq!(
            std::fs::read(directory.path().join("CURRENT")).unwrap(),
            format!("{manifest}\n").as_bytes()
        );
    }
}

#[test]
fn netease_does_not_refresh_record_for_a_mixed_encrypted_directory() {
    let (directory, _) = recovery_world();
    netease::encrypt(directory.path(), 3).unwrap();
    netease::decrypt_with_callback(directory.path(), 3, |_| {
        Err(std::io::Error::other("模拟解密中断").into())
    })
    .unwrap_err();
    std::fs::write(
        directory.path().join("MANIFEST-000002"),
        b"rotated manifest",
    )
    .unwrap();
    std::fs::write(directory.path().join("CURRENT"), b"MANIFEST-000002\n").unwrap();
    let record_path = directory.path().join(netease::RECOVERY_FILE);
    let record = std::fs::read(&record_path).unwrap();
    let encrypted_log = std::fs::read(directory.path().join("000003.log")).unwrap();

    assert!(netease::encrypt(directory.path(), 3).is_err());

    assert_eq!(std::fs::read(&record_path).unwrap(), record);
    assert_eq!(
        std::fs::read(directory.path().join("000003.log")).unwrap(),
        encrypted_log
    );
}
