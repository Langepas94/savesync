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

/// Обратная совместимость с ранним кодом: `normalize` = `slugify`.
pub fn normalize(name: &str) -> String {
    slugify(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_names() {
        assert_eq!(slugify("Elden Ring"), "elden-ring");
        assert_eq!(slugify("Hades"), "hades");
        assert_eq!(slugify("Hollow Knight"), "hollow-knight");
    }

    #[test]
    fn punctuation_and_case_collapse() {
        assert_eq!(slugify("ELDEN RING"), "elden-ring");
        assert_eq!(slugify("Baldur's Gate 3"), "baldur-s-gate-3");
        assert_eq!(slugify("Doom (2016)"), "doom-2016");
        assert_eq!(
            slugify("The Witcher 3: Wild Hunt"),
            "the-witcher-3-wild-hunt"
        );
        assert_eq!(slugify("  No Man's Sky  "), "no-man-s-sky");
    }

    #[test]
    fn same_game_same_key() {
        assert_eq!(slugify("Factorio"), slugify("FACTORIO"));
        assert_eq!(slugify("Slay the Spire"), slugify("Slay  the   Spire"));
    }

    #[test]
    fn cyrillic_is_transliterated() {
        assert_eq!(slugify("Ведьмак"), "vedmak");
        assert_eq!(slugify("Atomic Heart"), "atomic-heart");
        assert_eq!(slugify("Космические Рейнджеры"), "kosmicheskie-reyndzhery");
    }

    #[test]
    fn slug_validation() {
        assert!(is_valid_slug("elden-ring"));
        assert!(is_valid_slug("2077"));
        assert!(is_valid_slug("a"));
        assert!(!is_valid_slug(""));
        assert!(!is_valid_slug("-elden-ring"));
        assert!(!is_valid_slug("elden--ring"));
        assert!(!is_valid_slug("Elden-Ring"));
        assert!(!is_valid_slug("elden ring"));
        assert!(!is_valid_slug("elden_ring"));
        assert!(!is_valid_slug(&"a".repeat(SLUG_MAX_LEN + 1)));
    }

    #[test]
    fn slugify_output_is_always_valid_or_empty() {
        for name in ["Elden Ring", "Ведьмак 3", "!!!", "A".repeat(200).as_str()] {
            let s = slugify(name);
            assert!(
                s.is_empty() || is_valid_slug(&s),
                "invalid slug {s:?} from {name:?}"
            );
        }
    }

    #[test]
    fn custom_keys_are_prefixed() {
        assert_eq!(custom_key("My Indie Game"), "custom:my-indie-game");
        assert!(custom_key("x").starts_with("custom:"));
    }
}
