use rss_proxy::store::{NewFeed, Store};

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

/// 登録して id を返す。
fn feed_id(s: &Store, slug: &str) -> i64 {
    s.add_feed(&new_feed(slug)).unwrap();
    s.feed_by_slug(slug).unwrap().unwrap().id
}

fn new_feed(slug: &str) -> NewFeed {
    NewFeed {
        slug: Some(slug.to_string()),
        label: None,
        url: format!("https://example.com/{slug}.xml"),
        interval_secs: 900,
    }
}

#[test]
fn adds_and_reads_back_a_feed() {
    let s = store();
    let id = feed_id(&s, "news");

    let feed = s
        .feed_by_slug("news")
        .unwrap()
        .expect("登録した feed が読める");
    assert_eq!(feed.id, id);
    assert_eq!(feed.url, "https://example.com/news.xml");
    assert_eq!(feed.interval_secs, 900);
    assert_eq!(feed.fail_count, 0);
    assert!(feed.last_success_at.is_none());

    assert!(s.feed_by_slug("missing").unwrap().is_none());
}

#[test]
fn rejects_duplicate_names() {
    let s = store();
    s.add_feed(&new_feed("news")).unwrap();
    assert!(s.add_feed(&new_feed("news")).is_err());
}

#[test]
fn lists_and_removes_feeds() {
    let s = store();
    s.add_feed(&new_feed("a")).unwrap();
    s.add_feed(&new_feed("b")).unwrap();

    let names: Vec<String> = s
        .list_feeds()
        .unwrap()
        .into_iter()
        .map(|f| f.slug)
        .collect();
    assert_eq!(names, vec!["a", "b"]);

    assert!(s.remove_feed("a").unwrap());
    assert!(
        !s.remove_feed("a").unwrap(),
        "存在しない feed の削除は false"
    );
    assert_eq!(s.list_feeds().unwrap().len(), 1);
}

#[test]
fn due_feeds_respects_next_fetch_at_and_enabled() {
    let s = store();
    let due = feed_id(&s, "due");
    let later = feed_id(&s, "later");
    let disabled = feed_id(&s, "disabled");

    s.mark_success(later, None, None, 5_000).unwrap();
    s.set_enabled(disabled, false).unwrap();

    let ids: Vec<i64> = s
        .due_feeds(1_000)
        .unwrap()
        .into_iter()
        .map(|f| f.id)
        .collect();
    assert_eq!(ids, vec![due]);

    let ids: Vec<i64> = s
        .due_feeds(9_000)
        .unwrap()
        .into_iter()
        .map(|f| f.id)
        .collect();
    assert_eq!(ids, vec![due, later]);
}

#[test]
fn success_records_validators_and_clears_errors() {
    let s = store();
    let id = feed_id(&s, "news");

    s.mark_failure(id, "接続失敗", 100).unwrap();
    s.mark_failure(id, "接続失敗", 200).unwrap();
    let f = s.feed_by_slug("news").unwrap().unwrap();
    assert_eq!(f.fail_count, 2);
    assert_eq!(f.last_error.as_deref(), Some("接続失敗"));
    assert_eq!(f.next_fetch_at, 200);

    s.mark_success(
        id,
        Some("W/\"abc\""),
        Some("Mon, 07 Sep 2026 02:53:53 GMT"),
        1_100,
    )
    .unwrap();
    let f = s.feed_by_slug("news").unwrap().unwrap();
    assert_eq!(f.fail_count, 0);
    assert!(f.last_error.is_none());
    assert_eq!(f.etag.as_deref(), Some("W/\"abc\""));
    assert_eq!(
        f.last_modified.as_deref(),
        Some("Mon, 07 Sep 2026 02:53:53 GMT")
    );
    assert!(f.last_success_at.is_some());
}

#[test]
fn stores_and_replaces_the_processor_chain() {
    let s = store();
    let id = feed_id(&s, "news");
    assert!(s.processors(id).unwrap().is_empty());

    s.set_processors(
        id,
        &[
            ("google_news_cluster".into(), "{}".into()),
            ("dedupe".into(), r#"{"key":"link"}"#.into()),
        ],
    )
    .unwrap();

    let chain = s.processors(id).unwrap();
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0].0, "google_news_cluster");
    assert_eq!(chain[1].1, r#"{"key":"link"}"#);

    // 一括更新は古い連鎖を置き換える
    s.set_processors(id, &[("dedupe".into(), "{}".into())])
        .unwrap();
    assert_eq!(s.processors(id).unwrap().len(), 1);
}

#[test]
fn output_round_trips_and_is_removed_with_the_feed() {
    let s = store();
    let id = feed_id(&s, "news");
    assert!(s.output("news").unwrap().is_none());

    s.set_output(id, "<rss>1</rss>").unwrap();
    assert_eq!(s.output("news").unwrap().as_deref(), Some("<rss>1</rss>"));

    s.set_output(id, "<rss>2</rss>").unwrap();
    assert_eq!(s.output("news").unwrap().as_deref(), Some("<rss>2</rss>"));

    s.remove_feed("news").unwrap();
    assert!(
        s.output("news").unwrap().is_none(),
        "feed 削除で output も消える"
    );
}

#[test]
fn migration_is_idempotent() {
    let s = store();
    s.add_feed(&new_feed("news")).unwrap();
    s.migrate().unwrap();
    assert_eq!(s.list_feeds().unwrap().len(), 1);
}
