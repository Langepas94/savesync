use super::*;

#[test]
fn shard_rules() {
    assert_eq!(shard_of("elden-ring"), "e");
    assert_eq!(shard_of("7-days-to-die"), "0-9");
}

// --- синтетический appinfo.vdf v40 с одной игрой с UFS ---

fn cstr(s: &str) -> Vec<u8> {
    [s.as_bytes(), &[0]].concat()
}

fn kv_map_inline(key: &str, children: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8];
    b.extend(cstr(key));
    b.extend(children);
    b.push(8);
    b
}

fn kv_str(key: &str, val: &str) -> Vec<u8> {
    let mut b = vec![1u8];
    b.extend(cstr(key));
    b.extend(cstr(val));
    b
}

fn game_tree(appid: u32, name: &str) -> Vec<u8> {
    let _ = appid;
    let mut common = Vec::new();
    common.extend(kv_str("name", name));
    common.extend(kv_str("type", "Game"));
    let common = kv_map_inline("common", &common);

    let mut sf0 = Vec::new();
    sf0.extend(kv_str("root", "WinAppDataRoaming"));
    sf0.extend(kv_str("path", "TestGame"));
    sf0.extend(kv_map_inline("patterns", &kv_str("0", "*.sav")));
    let sf0 = kv_map_inline("0", &sf0);
    let savefiles = kv_map_inline("savefiles", &sf0);
    let ufs = kv_map_inline("ufs", &savefiles);

    let mut appinfo_inner = Vec::new();
    appinfo_inner.extend(common);
    appinfo_inner.extend(ufs);
    let appinfo = kv_map_inline("appinfo", &appinfo_inner);

    let mut root = Vec::new();
    root.extend(appinfo);
    root.push(8);
    root
}

fn appinfo_v40_with_game(appid: u32, name: &str) -> Vec<u8> {
    let tree = game_tree(appid, name);
    let mut record = Vec::new();
    let size = (4 + 4 + 8 + 20 + 4 + 20 + tree.len()) as u32;
    record.extend(appid.to_le_bytes());
    record.extend(size.to_le_bytes());
    record.extend(2u32.to_le_bytes());
    record.extend(1_700_000_000u32.to_le_bytes());
    record.extend(0u64.to_le_bytes());
    record.extend([0xAA; 20]);
    record.extend(1u32.to_le_bytes());
    record.extend([0xBB; 20]);
    record.extend(tree);

    let mut b = Vec::new();
    b.extend(0x07564428u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(record);
    b.extend(0u32.to_le_bytes());
    b
}

#[test]
fn generates_manifest_from_appinfo() {
    let dir = std::env::temp_dir().join(format!("savesync-test-gen-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let appinfo = dir.join("appinfo.vdf");
    std::fs::write(&appinfo, appinfo_v40_with_game(777001, "Test Game")).unwrap();

    let out = dir.join("repo");
    let stats = generate(&appinfo, &out, None, false).unwrap();
    assert_eq!(stats.games_seen, 1);
    assert_eq!(stats.generated, 1);

    let file = out.join("manifests/t/test-game.yaml");
    let yaml = std::fs::read_to_string(&file).unwrap();
    assert!(yaml.contains("slug: test-game"));
    assert!(yaml.contains("<winAppData>/TestGame/*.sav"));
    assert!(yaml.contains("<steamCompat>/777001/pfx/"));

    // Повторный прогон не перезаписывает (комьюнити-правки важнее).
    let stats2 = generate(&appinfo, &out, None, false).unwrap();
    assert_eq!(stats2.generated, 0);
    assert_eq!(stats2.skipped_existing, 1);

    // Фильтр по имени.
    let stats3 = generate(&appinfo, &out, Some("несуществующая"), true).unwrap();
    assert_eq!(stats3.games_seen, 0);

    std::fs::remove_dir_all(&dir).ok();
}
