use rss_proxy::auth::{Admin, hash_password, verify_password};

#[test]
fn a_hashed_password_verifies_only_against_itself() {
    let hash = hash_password("正しいパスワード").unwrap();
    assert!(hash.starts_with("$argon2"), "argon2 で保存する: {hash}");

    assert!(verify_password(&hash, "正しいパスワード"));
    assert!(!verify_password(&hash, "違うパスワード"));
    assert!(!verify_password(&hash, ""));
}

#[test]
fn the_same_password_hashes_differently_each_time() {
    let a = hash_password("同じ").unwrap();
    let b = hash_password("同じ").unwrap();
    assert_ne!(a, b, "ソルトが効いている");
    assert!(verify_password(&a, "同じ") && verify_password(&b, "同じ"));
}

#[test]
fn a_broken_hash_never_authenticates() {
    assert!(!verify_password("ハッシュではない", "何か"));
    assert!(!verify_password("", ""));
}

#[test]
fn admin_is_configured_only_when_both_values_are_present() {
    let hash = hash_password("pw").unwrap();

    assert!(Admin::new(Some("admin".into()), Some(hash.clone())).is_some());
    assert!(Admin::new(None, Some(hash.clone())).is_none());
    assert!(Admin::new(Some("admin".into()), None).is_none());
    assert!(Admin::new(Some("admin".into()), Some("  ".into())).is_none());
}

#[test]
fn credentials_must_match_both_user_and_password() {
    let admin = Admin::new(Some("admin".into()), Some(hash_password("pw").unwrap())).unwrap();

    assert!(admin.authenticates("admin", "pw"));
    assert!(!admin.authenticates("admin", "違う"));
    assert!(!admin.authenticates("別人", "pw"));
}
