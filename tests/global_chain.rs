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
