use super::*;

fn cstr(s: &str) -> Vec<u8> {
    [s.as_bytes(), &[0]].concat()
}

/// Минимальное KV-дерево записи: { "appinfo": { "common": { "name": .. } } }
fn kv_tree_v40(name: &str) -> Vec<u8> {
    let mut b = Vec::new();
    b.push(0u8);
    b.extend(cstr("appinfo"));
    b.push(0);
    b.extend(cstr("common"));
    b.push(1);
    b.extend(cstr("name"));
    b.extend(cstr(name));
    b.push(8); // end common
    b.push(8); // end appinfo
    b.push(8); // end root
    b
}

fn app_record_v40(appid: u32, name: &str) -> Vec<u8> {
    let tree = kv_tree_v40(name);
    let size = (4 + 4 + 8 + 20 + 4 + 20 + tree.len()) as u32; // от infoState до конца
    let mut b = Vec::new();
    b.extend(appid.to_le_bytes());
    b.extend(size.to_le_bytes());
    b.extend(2u32.to_le_bytes()); // info_state
    b.extend(1_700_000_000u32.to_le_bytes()); // last_updated
    b.extend(0u64.to_le_bytes()); // pics token
    b.extend([0xAA; 20]); // sha1
    b.extend(12345u32.to_le_bytes()); // change_number
    b.extend([0xBB; 20]); // binary sha1 (v40+)
    b.extend(tree);
    b
}

fn appinfo_v40(records: &[Vec<u8>]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend(0x07564428u32.to_le_bytes());
    b.extend(1u32.to_le_bytes()); // universe
    for r in records {
        b.extend(r);
    }
    b.extend(0u32.to_le_bytes()); // футер: appid 0
    b
}

#[test]
fn parses_v40_appinfo() {
    let data = appinfo_v40(&[
        app_record_v40(1245620, "ELDEN RING"),
        app_record_v40(413150, "Stardew Valley"),
    ]);
    let entries = parse(&data).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].appid, 1245620);
    assert_eq!(entries[0].name(), Some("ELDEN RING"));
    assert_eq!(entries[1].appid, 413150);
    assert_eq!(entries[1].name(), Some("Stardew Valley"));
    assert_eq!(entries[0].change_number, 12345);
}

#[test]
fn parses_v41_with_string_table() {
    // Таблица: 0="appinfo", 1="common", 2="name"
    let table: Vec<String> = ["appinfo", "common", "name"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    // KV-дерево с ключами-индексами; значение-строка inline.
    let mut tree = Vec::new();
    tree.push(0u8);
    tree.extend(0u32.to_le_bytes()); // "appinfo"
    tree.push(0);
    tree.extend(1u32.to_le_bytes()); // "common"
    tree.push(1);
    tree.extend(2u32.to_le_bytes()); // "name"
    tree.extend(cstr("Hades"));
    tree.extend([8, 8, 8]);

    let mut record = Vec::new();
    let size = (4 + 4 + 8 + 20 + 4 + 20 + tree.len()) as u32;
    record.extend(244850u32.to_le_bytes());
    record.extend(size.to_le_bytes());
    record.extend(2u32.to_le_bytes());
    record.extend(1_700_000_000u32.to_le_bytes());
    record.extend(0u64.to_le_bytes());
    record.extend([0xAA; 20]);
    record.extend(7u32.to_le_bytes());
    record.extend([0xBB; 20]);
    record.extend(&tree);

    let header_len = 4 + 4 + 8;
    let table_offset = header_len + record.len() + 4;

    let mut b = Vec::new();
    b.extend(0x07564429u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend((table_offset as u64).to_le_bytes());
    b.extend(&record);
    b.extend(0u32.to_le_bytes()); // футер
    b.extend((table.len() as u32).to_le_bytes());
    for s in &table {
        b.extend(cstr(s));
    }

    let entries = parse(&b).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].appid, 244850);
    assert_eq!(entries[0].name(), Some("Hades"));
}

#[test]
fn rejects_bad_magic_and_old_versions() {
    assert!(matches!(
        parse(&0xDEADBEEFu32.to_le_bytes()),
        Err(AppInfoError::BadMagic(_))
    ));
    let mut v36 = Vec::new();
    v36.extend(0x06564424u32.to_le_bytes()); // версия 36 — не поддерживаем
    v36.extend(1u32.to_le_bytes());
    v36.extend(0u32.to_le_bytes());
    assert!(matches!(
        parse(&v36),
        Err(AppInfoError::UnsupportedVersion(36))
    ));
}

#[test]
fn rejects_truncated() {
    let data = appinfo_v40(&[app_record_v40(1, "X")]);
    assert!(parse(&data[..data.len() - 5]).is_err());
}
