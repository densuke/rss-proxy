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

/// 誰も false にできない列だった。移行で落とす。
#[test]
fn the_unused_enabled_columns_are_dropped() {
    let dir = std::env::temp_dir().join(format!("rss-proxy-enabled-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("enabled.db");
    let _ = std::fs::remove_file(&path);
    legacy_db(&path);

    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.list_feeds().unwrap().len(),
        2,
        "移行でデータは失われない"
    );
    assert_eq!(store.processors(1).unwrap().len(), 1);

    let conn = Connection::open(&path).unwrap();
    for table in ["feeds", "processors"] {
        let remains: bool = conn
            .prepare(&format!(
                "SELECT 1 FROM pragma_table_info('{table}') WHERE name = 'enabled'"
            ))
            .unwrap()
            .exists([])
            .unwrap();
        assert!(!remains, "{table}.enabled が残っている");
    }

    // 2 回目の起動でも壊れない
    drop(store);
    assert_eq!(Store::open(&path).unwrap().list_feeds().unwrap().len(), 2);

    std::fs::remove_file(&path).ok();
}

/// 取り返しのつかない操作 (列の rename / drop) を含むため、適用前の状態を残す。
#[test]
fn a_snapshot_is_taken_before_the_schema_moves() {
    let dir = std::env::temp_dir().join(format!("rss-proxy-snapshot-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("legacy.db");
    let backup = dir.join("legacy.db.bak-v0");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&backup);
    legacy_db(&path);

    let store = Store::open(&path).unwrap();
    assert!(backup.exists(), "適用前のスナップショットがない");

    // 中身は適用前の状態。移行で消える列がそのまま残っている
    let old = Connection::open(&backup).unwrap();
    let has_name: bool = old
        .prepare("SELECT 1 FROM pragma_table_info('feeds') WHERE name = 'name'")
        .unwrap()
        .exists([])
        .unwrap();
    assert!(has_name, "スナップショットが適用後の形になっている");
    let feeds: i64 = old
        .query_row("SELECT count(*) FROM feeds", [], |r| r.get(0))
        .unwrap();
    assert_eq!(feeds, 2, "スナップショットにデータが入っていない");
    drop(old);

    // 進める段がなければ増やさない。起動のたびにファイルが増えては困る
    let taken: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .filter(|n| n.to_string_lossy().contains(".bak-v"))
        .collect();
    drop(store);
    Store::open(&path).unwrap();
    let after: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .filter(|n| n.to_string_lossy().contains(".bak-v"))
        .collect();
    assert_eq!(
        taken.len(),
        after.len(),
        "2 回目の起動でスナップショットが増えた"
    );

    std::fs::remove_file(&path).ok();
    std::fs::remove_file(&backup).ok();
}

/// インメモリ DB には保存先がない。テストと preview が通る道なので落ちては困る。
#[test]
fn an_in_memory_database_migrates_without_a_snapshot() {
    Store::open_in_memory().unwrap();
}

/// 新規インストールでは移行前の状態が存在しない。空のファイルを残さない。
#[test]
fn a_brand_new_database_leaves_no_snapshot() {
    let dir = std::env::temp_dir().join(format!("rss-proxy-fresh-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fresh.db");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(dir.join("fresh.db.bak-v0"));

    Store::open(&path).unwrap();
    assert!(
        !dir.join("fresh.db.bak-v0").exists(),
        "中身のない DB のスナップショットを作っている"
    );

    std::fs::remove_file(&path).ok();
}
