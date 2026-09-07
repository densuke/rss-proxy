use rss_proxy::slug::{generate, is_valid};

#[test]
fn generated_slugs_are_valid_and_unpredictable() {
    let a = generate();
    let b = generate();

    assert!(is_valid(&a), "生成した slug が形式を満たさない: {a}");
    assert_ne!(a, b, "毎回異なる");
    // 128 ビットを base64url にすると 22 文字。総当たりは非現実的
    assert_eq!(a.len(), 22);
}

#[test]
fn accepts_url_safe_identifiers() {
    for s in ["nhk", "gnews-headline", "a_b-9", &"x".repeat(64)] {
        assert!(is_valid(s), "{s} は使えるはず");
    }
}

#[test]
fn rejects_anything_that_would_need_url_encoding() {
    for s in [
        "",
        "ab",               // 3 文字未満
        &"x".repeat(65),    // 64 文字超
        "NHK 主要ニュース", // 空白と非 ASCII
        "a/b",
        "a.b",
        "a%20b",
        "日本語",
    ] {
        assert!(!is_valid(s), "{s} は拒否されるはず");
    }
}
