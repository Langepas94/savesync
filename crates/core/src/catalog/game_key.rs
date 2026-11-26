//! Логический ключ игры (`game_key`) — slug из репозитория
//! `savesync-manifests`. Store-агностично: одна игра = один ключ на обоих
//! устройствах, независимо от магазина (Steam / GOG / Epic / standalone).
//!
//! Нормативные правила slug'ов — docs/manifests-schema.md §3. Этот модуль —
//! их эталонная реализация: `slugify` используется генератором манифестов
//! и как fallback-матчинг для имён, введённых пользователем.

/// Таблица транслитерации кириллицы (ГОСТ-подобная, упрощённая).
fn translit(ch: char) -> Option<&'static str> {
    let lower = ch.to_lowercase().next()?;
    Some(match lower {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'д' => "d",
        'е' | 'ё' => "e",
        'ж' => "zh",
        'з' => "z",
        'и' => "i",
        'й' => "y",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ф' => "f",
        'х' => "h",
        'ц' => "ts",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "sch",
        'ъ' | 'ь' => "",
        'ы' => "y",
        'э' => "e",
        'ю' => "yu",
        'я' => "ya",
        _ => return None,
    })
}

/// Максимальная длина slug'а (см. schema §3).
pub const SLUG_MAX_LEN: usize = 64;

/// Нормализует имя игры в slug.
///
/// Правила: lowercase ASCII `[a-z0-9]`, кириллица транслитерируется,
/// знаки препинания и `™®©` отбрасываются/схлопываются в один дефис,
/// дефисы по краям обрезаются, длина ограничена `SLUG_MAX_LEN`.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_was_dash = true; // не даём дефису оказаться первым символом
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if let Some(t) = translit(ch) {
            if !t.is_empty() {
                out.push_str(t);
                last_was_dash = false;
            }
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
        if out.len() >= SLUG_MAX_LEN {
            break;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Проверяет, что строка — валидный slug по правилам schema §3.
pub fn is_valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= SLUG_MAX_LEN
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Ключ для вручную добавленной игры вне базы `savesync-manifests`.
/// Префикс `custom:` гарантирует отсутствие коллизий с опубликованными slug'ами.
pub fn custom_key(name: &str) -> String {
    format!("custom:{}", slugify(name))
}

pub(crate) fn title_initials(slug: &str) -> Option<String> {
    if !slug.contains('-') {
        return None;
    }
    let mut initials = String::new();
    for word in slug.split('-') {
        if word.bytes().all(|byte| byte.is_ascii_digit()) {
            initials.push_str(word);
        } else if let Some(initial) = word.chars().next() {
            initials.push(initial);
        }
    }
    (initials.len() >= 3).then_some(initials)
}

/// Обратная совместимость с ранним кодом: `normalize` = `slugify`.
pub fn normalize(name: &str) -> String {
    slugify(name)
}

#[cfg(test)]
#[path = "../tests/catalog/game_key.rs"]
mod tests;
