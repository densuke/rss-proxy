use rss_proxy::store::{NewFeed, Store};

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

#[test]
fn a_fresh_database_starts_with_sensible_defaults() {
    let s = store();
    let chain = s.global_processors().unwrap();
    assert!(!chain.is_empty(), "既定のグローバル連鎖がある");

    let kinds: Vec<&str> = chain.iter().map(|(k, _)| k.as_str()).collect();
    assert!(kinds.contains(&"normalize_width"));
    assert!(kinds.contains(&"exclude"));

    // 正規化が先。［PR］ が [PR] になってから除外判定にかかる
    let normalize = kinds.iter().position(|k| *k == "normalize_width").unwrap();
    let exclude = kinds.iter().position(|k| *k == "exclude").unwrap();
    assert!(normalize < exclude, "正規化を先に適用する");

    // 既定の連鎖はそのまま組み立てられる
    for (kind, params) in &chain {
        rss_proxy::proc::build(kind, params).expect(kind);
    }
}

#[test]
fn the_global_chain_can_be_replaced() {
    let s = store();
    s.set_global_processors(&[("dedupe".into(), r#"{"key":"guid"}"#.into())])
        .unwrap();

    let chain = s.global_processors().unwrap();
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].0, "dedupe");

    // 空にもできる (既定を使いたくない場合)
    s.set_global_processors(&[]).unwrap();
    assert!(s.global_processors().unwrap().is_empty());
}

#[test]
fn the_global_chain_is_independent_of_any_feed() {
    let s = store();
    s.add_feed(&NewFeed {
        slug: Some("news".into()),
        label: None,
        url: "https://example.com/f.xml".into(),
        interval_secs: 900,
    })
    .unwrap();
    let id = s.feed_by_slug("news").unwrap().unwrap().id;

    s.set_global_processors(&[("dedupe".into(), "{}".into())])
        .unwrap();
    s.set_processors(id, &[("max_age".into(), "{}".into())])
        .unwrap();

    assert_eq!(s.global_processors().unwrap().len(), 1);
    assert_eq!(s.processors(id).unwrap().len(), 1);

    // フィードを消してもグローバルは残る
    s.remove_feed("news").unwrap();
    assert_eq!(s.global_processors().unwrap().len(), 1);
}

/// 既定のグローバル連鎖を入れる移行は、全フィードを作り直しの対象にする。
/// そうしないと 304 で処理がスキップされ、新しい連鎖が反映されない。
#[test]
fn seeding_the_defaults_reschedules_existing_feeds() {
    use rusqlite::Connection;

    let dir = std::env::temp_dir().join(format!("rss-proxy-seed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("seed.db");
    let _ = std::fs::remove_file(&path);

    // グローバル連鎖を持たない時点の DB を作り、検証子と先の予定を入れておく
    {
        let s = Store::open(&path).unwrap();
        s.add_feed(&NewFeed {
            slug: Some("news".into()),
            label: None,
            url: "https://example.com/f.xml".into(),
            interval_secs: 900,
        })
        .unwrap();
        let id = s.feed_by_slug("news").unwrap().unwrap().id;
        s.mark_success(id, Some("W/\"1\""), Some("Mon"), 9_999_999_999)
            .unwrap();
    }
    Connection::open(&path)
        .unwrap()
        .execute_batch("DELETE FROM global_processors; PRAGMA user_version = 2")
        .unwrap();

    let s = Store::open(&path).unwrap();
    assert!(!s.global_processors().unwrap().is_empty(), "既定が入る");

    let f = s.feed_by_slug("news").unwrap().unwrap();
    assert!(
        f.etag.is_none() && f.last_modified.is_none() && f.next_fetch_at == 0,
        "移行後に作り直しの対象になっていない"
    );

    std::fs::remove_file(&path).ok();
}
