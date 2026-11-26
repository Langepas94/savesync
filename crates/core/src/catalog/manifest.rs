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

use crate::platform::paths::Platform;

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
    /// The catalog identity previously confirmed for a local custom rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_key: Option<String>,
    /// Installation detected on this device. Never stored in shared manifests.
    #[serde(skip)]
    pub installed_at: Option<std::path::PathBuf>,
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
    /// Stable logical location shared by equivalent Windows/Linux rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
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
            slug: crate::catalog::game_key::custom_key(name),
            catalog_key: None,
            installed_at: None,
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

    /// Publish a new local rule without replacing an existing rule with the same slug.
    pub fn save_new_custom(&self, directory: &std::path::Path) -> std::io::Result<()> {
        self.write_custom(directory, false)
    }

    pub fn write_custom(&self, directory: &std::path::Path, replace: bool) -> std::io::Result<()> {
        use std::io::{self, Write};

        let slug = self
            .slug
            .strip_prefix("custom:")
            .filter(|slug| crate::catalog::game_key::is_valid_slug(slug))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid custom slug"))?;
        let yaml = self.to_yaml().map_err(io::Error::other)?;
        std::fs::create_dir_all(directory)?;
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        file.write_all(yaml.as_bytes())?;
        file.as_file().sync_all()?;
        let target = directory.join(format!("{slug}.yaml"));
        if replace {
            file.persist(target)
        } else {
            file.persist_noclobber(target)
        }
        .map_err(|error| error.error)?;
        Ok(())
    }

    /// Валидация записи (правила schema §6): версия, slug, причина для warn/blocked.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.schema != SCHEMA_VERSION {
            errors.push(format!("unsupported schema {}", self.schema));
        }
        // Пользовательские игры: `custom:<slug>` (schema §3) — валиден slug
        // после префикса. В репозитории такие записи не публикуются.
        let slug_ok = crate::catalog::game_key::is_valid_slug(&self.slug)
            || self
                .slug
                .strip_prefix("custom:")
                .is_some_and(crate::catalog::game_key::is_valid_slug);
        if !slug_ok {
            errors.push(format!("invalid slug {:?}", self.slug));
        }
        if self.catalog_key.as_ref().is_some_and(|key| {
            !self.slug.starts_with("custom:") || !crate::catalog::game_key::is_valid_slug(key)
        }) {
            errors.push("invalid custom catalog identity".into());
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
        for rule in &self.paths {
            if let Err(error) = glob::Pattern::new(&rule.pattern) {
                errors.push(format!("invalid path pattern: {error}"));
            }
        }
        if self.portability.status != PortabilityStatus::Ok && self.portability.reason.is_none() {
            errors.push("portability.warn/blocked requires reason".into());
        }
        errors
    }
}

/// Загруженная база манифестов, ключ — slug.
#[derive(Debug, Default, Clone)]
pub struct ManifestDb {
    entries: Vec<Manifest>,
}

impl ManifestDb {
    pub fn from_entries(entries: Vec<Manifest>) -> Self {
        ManifestDb { entries }
    }

    /// Добавляет манифест, заменяя существующий с тем же slug.
    pub fn insert(&mut self, m: Manifest) {
        self.remove(&m.slug);
        self.entries.push(m);
    }

    pub fn remove(&mut self, slug: &str) {
        self.entries.retain(|entry| entry.slug != slug);
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
        if entry.file_type().is_ok_and(|kind| kind.is_symlink()) {
            errors.push(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "symlinks are not allowed in manifest databases",
            ));
            continue;
        }
        if path.is_dir() {
            load_dir_recursive(&path, db, errors);
        } else if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
            match std::fs::read_to_string(&path) {
                Ok(text) => match parse_manifest_str(&text) {
                    Ok(m) if m.validate().is_empty() => db.entries.push(m),
                    Ok(m) => errors.push(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        m.validate().join("; "),
                    )),
                    Err(e) => errors.push(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
                },
                Err(e) => errors.push(e),
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/catalog/manifest.rs"]
mod tests;
