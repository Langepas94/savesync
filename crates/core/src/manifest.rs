//! Манифесты `savesync-manifests` (schema v1): разбор YAML и выбор путей
//! под платформу. Нормативное описание формата — docs/manifests-schema.md.
//!
//! Один файл = одна игра. Пример:
//! ```yaml
//! schema: 1
//! name: Elden Ring
//! slug: elden-ring
//! ids:
//!   steam: ["1245620"]
//! paths:
//!   - pattern: "<winAppData>/EldenRing"
//!     os: windows
//!     tags: [save]
//!   - pattern: "<steamCompat>/1245620/pfx/drive_c/users/steamuser/AppData/Roaming/EldenRing"
//!     os: linux
//!     tags: [save]
//! portability:
//!   status: warn
//!   reason: "Сейв привязан к SteamID64"
//! ```

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::paths::Platform;

/// Актуальная версия схемы манифестов.
pub const SCHEMA_VERSION: u32 = 1;

/// Один файл манифеста = одна игра.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Версия схемы; сейчас принимается только `1`.
    pub schema: u32,
    /// Человекочитаемое имя игры (для UI).
    pub name: String,
    /// Каноничный slug = `game_key`; обязан совпадать с именем файла.
    pub slug: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Ids::is_empty")]
    pub ids: Ids,
    #[serde(default)]
    pub paths: Vec<PathRule>,
    /// Пути реестра (Windows / Proton-префикс). Экспорт веток `*.reg` —
    /// отдельная задача (см. research); поле парсится, но не синкается.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry: Vec<PathRule>,
    #[serde(default, skip_serializing_if = "Portability::is_default")]
    pub portability: Portability,
}

/// Магазинные идентификаторы (для матчинга со сканером appinfo и подсказок UI).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ids {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steam: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gog: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub epic: Vec<String>,
}

impl Ids {
    pub fn is_empty(&self) -> bool {
        self.steam.is_empty() && self.gog.is_empty() && self.epic.is_empty()
    }
}

/// Один путь сейвов из манифеста.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathRule {
    /// Паттерн с плейсхолдером (`<winAppData>/...`, `<steamCompat>/...`),
    /// glob-символы `*`, `**`, `?` допустимы.
    pub pattern: String,
    /// Целевая ОС; `any` — правило применяется везде.
    #[serde(default)]
    pub os: Os,
    /// `save` (по умолчанию) и/или `config`.
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Windows,
    Linux,
    #[default]
    Any,
}

impl Os {
    pub fn matches(&self, platform: Platform) -> bool {
        match (self, platform) {
            (Os::Any, _) => true,
            (Os::Windows, Platform::Windows) => true,
            // Proton-игры на Deck резолвятся через <steamCompat> (os: linux),
            // нативные linux-пути — тоже os: linux.
            (Os::Linux, Platform::Linux) => true,
            _ => false,
        }
    }
}

/// Переносимость сейвов между устройствами (свой варн/блок-лист,
/// см. research: FromSoftware SteamID64, GameMaker secure_save).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Portability {
    #[serde(default)]
    pub status: PortabilityStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Portability {
    pub fn is_default(&self) -> bool {
        self.status == PortabilityStatus::Ok && self.reason.is_none()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortabilityStatus {
    /// Переносится без оговорок.
    #[default]
    Ok,
    /// Переносится с оговорками (например, привязка к SteamID64 того же аккаунта).
    Warn,
    /// Не переносится: синхронизация заблокирована, показываем пояснение.
    Blocked,
}

impl Manifest {
    /// Пути, применимые к целевой платформе.
    pub fn paths_for(&self, platform: Platform) -> impl Iterator<Item = &PathRule> {
        self.paths.iter().filter(move |p| p.os.matches(platform))
    }

    /// Custom-правило пользователя (schema §3): slug получает префикс
    /// `custom:` и никогда не публикуется в репозитории манифестов.
    pub fn custom(name: &str, paths: Vec<PathRule>) -> Result<Manifest, String> {
        let m = Manifest {
            schema: SCHEMA_VERSION,
            name: name.to_string(),
            slug: crate::game_key::custom_key(name),
            aliases: Vec::new(),
            ids: Ids::default(),
            paths,
            registry: Vec::new(),
            portability: Portability::default(),
        };
        let errors = m.validate();
        if errors.is_empty() {
            Ok(m)
        } else {
            Err(errors.join("; "))
        }
    }

    /// Сериализация в YAML (запись custom-правил, экспорт).
    pub fn to_yaml(&self) -> Result<String, serde_yaml::Error> {
        serde_yaml::to_string(self)
    }

    /// Валидация записи (правила schema §6): версия, slug, причина для warn/blocked.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.schema != SCHEMA_VERSION {
            errors.push(format!("unsupported schema {}", self.schema));
        }
        // Пользовательские игры: `custom:<slug>` (schema §3) — валиден slug
        // после префикса. В репозитории такие записи не публикуются.
        let slug_ok = crate::game_key::is_valid_slug(&self.slug)
            || self
                .slug
                .strip_prefix("custom:")
                .is_some_and(crate::game_key::is_valid_slug);
        if !slug_ok {
            errors.push(format!("invalid slug {:?}", self.slug));
        }
        if self.name.trim().is_empty() {
            errors.push("empty name".into());
        }
        if self.paths.is_empty() && self.registry.is_empty() {
            errors.push("no paths".into());
        }
        if self
            .paths
            .iter()
            .chain(&self.registry)
            .any(|p| p.pattern.trim().is_empty())
        {
            errors.push("empty pattern".into());
        }
        if self.portability.status != PortabilityStatus::Ok && self.portability.reason.is_none() {
            errors.push("portability.warn/blocked requires reason".into());
        }
        errors
    }
}

/// Загруженная база манифестов, ключ — slug.
#[derive(Debug, Default)]
pub struct ManifestDb {
    entries: Vec<Manifest>,
}

impl ManifestDb {
    pub fn from_entries(entries: Vec<Manifest>) -> Self {
        ManifestDb { entries }
    }

    /// Добавляет манифест, заменяя существующий с тем же slug.
    pub fn insert(&mut self, m: Manifest) {
        self.entries.retain(|e| e.slug != m.slug);
        self.entries.push(m);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Manifest> {
        self.entries.iter()
    }

    pub fn get(&self, slug: &str) -> Option<&Manifest> {
        self.entries.iter().find(|m| m.slug == slug)
    }
}

/// Разбирает один YAML-файл манифеста (одна игра).
pub fn parse_manifest_str(yaml: &str) -> Result<Manifest, serde_yaml::Error> {
    serde_yaml::from_str(yaml)
}

/// Рекурсивно загружает все `*.yaml` из дерева `manifests/<shard>/<slug>.yaml`.
/// Битые файлы не роняют загрузку — попадают в вектор ошибок.
pub fn load_manifest_tree(dir: &Path) -> (ManifestDb, Vec<std::io::Error>) {
    let mut db = ManifestDb::default();
    let mut errors = Vec::new();
    load_dir_recursive(dir, &mut db, &mut errors);
    (db, errors)
}

fn load_dir_recursive(dir: &Path, db: &mut ManifestDb, errors: &mut Vec<std::io::Error>) {
    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            errors.push(e);
            return;
        }
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            load_dir_recursive(&path, db, errors);
        } else if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
            match std::fs::read_to_string(&path) {
                Ok(text) => match parse_manifest_str(&text) {
                    Ok(m) => db.entries.push(m),
                    Err(e) => errors.push(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
                },
                Err(e) => errors.push(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ELDEN_RING: &str = r#"
schema: 1
name: Elden Ring
slug: elden-ring
aliases: ["ELDEN RING"]
ids:
  steam: ["1245620"]
paths:
  - pattern: "<winAppData>/EldenRing"
    os: windows
    tags: [save]
  - pattern: "<steamCompat>/1245620/pfx/drive_c/users/steamuser/AppData/Roaming/EldenRing"
    os: linux
    tags: [save]
portability:
  status: warn
  reason: "Сейв зашифрован ключом от SteamID64: перенос только между устройствами одного аккаунта"
"#;

    const STARDEW: &str = r#"
schema: 1
name: Stardew Valley
slug: stardew-valley
ids:
  steam: ["413150"]
  gog: ["1453375253"]
paths:
  - pattern: "<winAppData>/StardewValley/Saves"
    os: windows
  - pattern: "<xdgConfig>/StardewValley/Saves"
    os: linux
"#;

    #[test]
    fn parses_full_manifest() {
        let m = parse_manifest_str(ELDEN_RING).unwrap();
        assert_eq!(m.schema, 1);
        assert_eq!(m.slug, "elden-ring");
        assert_eq!(m.ids.steam, vec!["1245620"]);
        assert_eq!(m.paths.len(), 2);
        assert_eq!(m.portability.status, PortabilityStatus::Warn);
        assert!(m.portability.reason.is_some());
        assert!(m.validate().is_empty());
    }

    #[test]
    fn os_defaults_to_any_and_tags_default_save_agnostic() {
        let m = parse_manifest_str(
            "schema: 1\nname: X\nslug: x\npaths:\n  - pattern: \"<home>/x/saves\"\n",
        )
        .unwrap();
        assert_eq!(m.paths[0].os, Os::Any);
        assert!(m.paths[0].tags.is_empty());
        assert!(m.validate().is_empty());
    }

    #[test]
    fn paths_filtered_by_platform() {
        let m = parse_manifest_str(ELDEN_RING).unwrap();
        let win: Vec<_> = m.paths_for(Platform::Windows).collect();
        let lin: Vec<_> = m.paths_for(Platform::Linux).collect();
        assert_eq!(win.len(), 1);
        assert_eq!(lin.len(), 1);
        assert!(win[0].pattern.contains("winAppData"));
        assert!(lin[0].pattern.contains("steamCompat"));
    }

    #[test]
    fn validation_catches_bad_entries() {
        let mut m = parse_manifest_str(STARDEW).unwrap();
        assert!(m.validate().is_empty());

        m.slug = "Not A Slug".into();
        assert!(m.validate().iter().any(|e| e.contains("invalid slug")));

        let mut m2 = parse_manifest_str(STARDEW).unwrap();
        m2.portability.status = PortabilityStatus::Blocked;
        assert!(m2.validate().iter().any(|e| e.contains("requires reason")));

        let mut m3 = parse_manifest_str(STARDEW).unwrap();
        m3.schema = 99;
        assert!(m3
            .validate()
            .iter()
            .any(|e| e.contains("unsupported schema")));
    }

    #[test]
    fn load_tree_tolerates_garbage() {
        let dir =
            std::env::temp_dir().join(format!("savesync-test-manifest-{}", std::process::id()));
        let shard = dir.join("manifests").join("el");
        std::fs::create_dir_all(&shard).unwrap();
        std::fs::write(shard.join("elden-ring.yaml"), ELDEN_RING).unwrap();
        std::fs::write(shard.join("bad.yaml"), "{{{not yaml").unwrap();
        std::fs::write(dir.join("ignored.txt"), "schema: 1\n").unwrap();

        let (db, errors) = load_manifest_tree(&dir);
        assert_eq!(db.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(db.get("elden-ring").is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn custom_manifest_roundtrip() {
        let m = Manifest::custom(
            "My Indie Game",
            vec![PathRule {
                pattern: "<documents>/My Indie Game/saves".into(),
                os: Os::Any,
                tags: vec!["save".into()],
            }],
        )
        .unwrap();
        assert_eq!(m.slug, "custom:my-indie-game");
        assert!(m.validate().is_empty());

        // YAML round-trip: запись на диск и загрузка обратно.
        let yaml = m.to_yaml().unwrap();
        let parsed = parse_manifest_str(&yaml).unwrap();
        assert_eq!(parsed.slug, m.slug);
        assert_eq!(parsed.paths.len(), 1);
        assert!(parsed.validate().is_empty());
    }

    #[test]
    fn custom_manifest_rejects_empty_input() {
        assert!(Manifest::custom("", vec![]).is_err());
        assert!(Manifest::custom(
            "X",
            vec![PathRule {
                pattern: "".into(),
                os: Os::Any,
                tags: vec![],
            }],
        )
        .is_err());
    }
}
