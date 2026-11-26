use super::*;
use crate::platform::paths::{Platform, Roots};

#[test]
fn bundled_database_finds_steam_game_without_ufs() {
    let tmp = tempfile::tempdir().unwrap();
    let save = tmp.path().join("steamapps/compatdata/12210/pfx/drive_c/users/steamuser/Documents/Rockstar Games/GTA IV/Profiles/123/SGTA400");
    std::fs::create_dir_all(save.parent().unwrap()).unwrap();
    std::fs::write(&save, b"save").unwrap();
    let mut db = ManifestDb::default();
    add_matching(&mut db, &HashSet::from([12210]), &HashSet::new());
    let roots = Roots {
        steam_libraries: vec![tmp.path().into()],
        ..Roots::default()
    };
    let found = crate::catalog::scanner::scan(&db, Platform::Linux, &roots);
    assert!(found
        .iter()
        .any(|game| game.locations.iter().any(|location| location
            .hits
            .iter()
            .any(|hit| crate::platform::paths::canonical_path(hit)
                == crate::platform::paths::canonical_path(&save)))));
    let before = db.get("grand-theft-auto-iv").unwrap().paths.len();
    add_matching(&mut db, &HashSet::from([12210]), &HashSet::new());
    assert_eq!(before, db.get("grand-theft-auto-iv").unwrap().paths.len());
}
