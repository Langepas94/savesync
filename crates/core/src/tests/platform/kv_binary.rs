use super::*;

fn cstr(s: &str) -> Vec<u8> {
    [s.as_bytes(), &[0]].concat()
}

/// Собирает бинарное KV-дерево (v40-стиль, inline-ключи):
/// { "appinfo": { "common": { "name": "Elden Ring" }, "appid": 1245620 } }
fn sample_tree_v40() -> Vec<u8> {
    let mut b = Vec::new();
    b.push(0u8); // map "appinfo"
    b.extend(cstr("appinfo"));
    {
        b.push(0); // map "common"
        b.extend(cstr("common"));
        {
            b.push(1); // string "name"
            b.extend(cstr("name"));
            b.extend(cstr("Elden Ring"));
        }
        b.push(8); // end common
        b.push(2); // i32 "appid"
        b.extend(cstr("appid"));
        b.extend(1245620i32.to_le_bytes());
        b.push(7); // u64 "size"
        b.extend(cstr("size"));
        b.extend(9_999_999_999u64.to_le_bytes());
    }
    b.push(8); // end appinfo
    b.push(8); // end root
    b
}

#[test]
fn parses_inline_keys() {
    let data = sample_tree_v40();
    let mut p = Parser::new(&data, None);
    let tree = Value::Map(p.map().unwrap());
    let name = tree.get(&["appinfo", "common", "name"]).unwrap();
    assert_eq!(name.as_str(), Some("Elden Ring"));
    assert!(matches!(
        tree.get(&["appinfo", "appid"]).unwrap(),
        Value::I32(1245620)
    ));
    assert!(matches!(
        tree.get(&["appinfo", "size"]).unwrap(),
        Value::U64(9_999_999_999)
    ));
}

#[test]
fn case_insensitive_lookup() {
    let data = sample_tree_v40();
    let mut p = Parser::new(&data, None);
    let tree = Value::Map(p.map().unwrap());
    assert!(tree.get(&["APPINFO", "Common", "NAME"]).is_some());
}

#[test]
fn parses_string_table_keys_v41() {
    // v41: ключи — u32-индексы в таблице строк.
    let table = vec!["root".to_string(), "answer".to_string()];
    let mut b = Vec::new();
    b.push(0u8); // map, ключ = индекс 0
    b.extend(0u32.to_le_bytes());
    b.push(2); // i32, ключ = индекс 1
    b.extend(1u32.to_le_bytes());
    b.extend(42i32.to_le_bytes());
    b.push(8);
    b.push(8);

    let mut p = Parser::new(&b, Some(&table));
    let tree = Value::Map(p.map().unwrap());
    assert!(matches!(
        tree.get(&["root", "answer"]).unwrap(),
        Value::I32(42)
    ));
}

#[test]
fn rejects_truncated_data() {
    let data = &sample_tree_v40()[..5];
    let mut p = Parser::new(data, None);
    assert!(p.map().is_err());
}

#[test]
fn rejects_unknown_type() {
    let mut b = Vec::new();
    b.push(0x42u8);
    b.extend(cstr("key"));
    let mut p = Parser::new(&b, None);
    assert!(matches!(p.map(), Err(KvError::UnknownType(0x42, 0))));
}
