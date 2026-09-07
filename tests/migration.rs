//! 既存 DB (name をそのまま URL に使っていた形) からの移行。

use rss_proxy::slug::is_valid;
use rss_proxy::store::Store;
use rusqlite::Connection;

/// 移行前のスキーマでフィードを 2 件入れた DB を作る。
fn legacy_db(path: &std::path::Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE feeds (
            id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, url TEXT NOT NULL,
            title TEXT, interval_secs INTEGER NOT NULL DEFAULT 900,
            enabled INTEGER NOT NULL DEFAULT 1, etag TEXT, last_modified TEXT,
            next_fetch_at INTEGER NOT NULL DEFAULT 0, last_success_at INTEGER,
            last_error TEXT, fail_count INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE processors (
            id INTEGER PRIMARY KEY, feed_id INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
            position INTEGER NOT NULL, kind TEXT NOT NULL, params TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1, UNIQUE(feed_id, position)
        );
        CREATE TABLE outputs (
            feed_id INTEGER PRIMARY KEY REFERENCES feeds(id) ON DELETE CASCADE,
            xml TEXT NOT NULL, generated_at INTEGER NOT NULL
        );
        INSERT INTO feeds (name, url, created_at) VALUES ('gnews-headline', 'https://a', 0);
        INSERT INTO feeds (name, url, created_at) VALUES ('NHK 主要ニュース', 'https://b', 0);
        INSERT INTO processors (feed_id, position, kind, params) VALUES (1, 0, 'dedupe', '{}');
        INSERT INTO outputs (feed_id, xml, generated_at) VALUES (1, '<rss/>', 0);
        "#,
    )
    .unwrap();
}

#[test]
fn url_safe_names_keep_their_address_and_others_get_a_random_one() {
    let dir = std::env::temp_dir().join(format!("rss-proxy-migration-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("legacy.db");
    let _ = std::fs::remove_file(&path);
    legacy_db(&path);

    let store = Store::open(&path).unwrap();
    let feeds = store.list_feeds().unwrap();
    assert_eq!(feeds.len(), 2);

    // すでに URL に使えていた識別子は変えない。購読中の URL を壊さないため
    let kept = &feeds[0];
    assert_eq!(kept.slug, "gnews-headline");
    assert_eq!(
        kept.label.as_deref(),
        None,
        "元から URL 安全なら表示名は付けない"
    );

    // URL にできない名前は識別子をランダムに振り直し、元の名前は表示名として残す
    let renamed = &feeds[1];
    assert!(
        is_valid(&renamed.slug),
        "移行後の slug が不正: {}",
        renamed.slug
    );
    assert_ne!(renamed.slug, "NHK 主要ニュース");
    assert_eq!(renamed.label.as_deref(), Some("NHK 主要ニュース"));

    // 連鎖と出力は失われない
    assert_eq!(store.processors(kept.id).unwrap().len(), 1);
    assert_eq!(
        store.output("gnews-headline").unwrap().as_deref(),
        Some("<rss/>")
    );

    // 2 回目の起動でも壊れない
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.list_feeds().unwrap()[1].slug, renamed.slug);

    std::fs::remove_file(&path).ok();
}
