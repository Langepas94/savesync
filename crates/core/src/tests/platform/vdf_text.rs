use super::*;

#[test]
fn parses_appmanifest() {
    let acf = r#"
"AppState"
{
    "appid"     "1245620"
    "name"      "ELDEN RING"
    "installdir"        "ELDEN RING"
    "SizeOnDisk"        "51234567890"
}
"#;
    let doc = parse(acf).unwrap();
    let root = Node::Map(doc);
    assert_eq!(root.get_str(&["AppState", "appid"]), Some("1245620"));
    assert_eq!(root.get_str(&["AppState", "name"]), Some("ELDEN RING"));
    assert_eq!(
        root.get_str(&["AppState", "installdir"]),
        Some("ELDEN RING")
    );
}

#[test]
fn parses_libraryfolders() {
    let vdf = r#"
"libraryfolders"
{
    "0"
    {
        "path"      "C:\\Program Files (x86)\\Steam"
        "apps"
        {
            "1245620"       "51234567890"
        }
    }
    "1"
    {
        "path"      "D:\\SteamLibrary"
        "apps"
        {
            "413150"        "1000"
        }
    }
}
"#;
    let doc = parse(vdf).unwrap();
    let root = Node::Map(doc);
    let folders = root.get(&["libraryfolders"]).unwrap().as_map().unwrap();
    assert_eq!(folders.len(), 2);
    // Backslash в путях Windows сохраняется как литерал.
    assert_eq!(
        root.get_str(&["libraryfolders", "1", "path"]),
        Some("D:\\SteamLibrary")
    );
}

#[test]
fn skips_comments() {
    let vdf = "// comment\n\"a\"\n{\n// inner\n\"b\" \"c\"\n}\n";
    let doc = parse(vdf).unwrap();
    assert_eq!(Node::Map(doc).get_str(&["a", "b"]), Some("c"));
}

#[test]
fn windows_paths_stay_literal() {
    let doc = parse("\"k\" \"C:\\Games\\New Folder\"").unwrap();
    assert_eq!(
        Node::Map(doc).get_str(&["k"]),
        Some("C:\\Games\\New Folder")
    );
}

#[test]
fn rejects_garbage() {
    assert!(parse("not a vdf").is_err());
    assert!(parse("\"unterminated").is_err());
    assert!(parse("\"k\" }").is_err());
}
