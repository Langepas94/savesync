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
