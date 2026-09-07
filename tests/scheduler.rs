mod common;

use rss_proxy::scheduler::{backoff_secs, refresh};
use rss_proxy::store::{NewFeed, Store};

const FIXTURE: &str = include_str!("fixtures/google_news_headline.xml");

fn store_with_feed(url: &str) -> (Store, i64) {
    let s = Store::open_in_memory().unwrap();
    s.add_feed(&NewFeed {
        slug: Some("news".into()),
        label: None,
        url: url.into(),
        interval_secs: 900,
    })
    .unwrap();
    let id = s.feed_by_slug("news").unwrap().unwrap().id;
    s.set_processors(id, &[("google_news_cluster".into(), "{}".into())])
        .unwrap();
    (s, id)
}

#[tokio::test]
async fn refresh_stores_processed_output() {
    let (url, _up) = common::serve(FIXTURE).await;
    let (store, _id) = store_with_feed(&url);
    let feed = store.feed_by_slug("news").unwrap().unwrap();

    refresh(&store, &rss_proxy::fetch::client(), &feed)
        .await
        .unwrap();

    let xml = store.output("news").unwrap().expect("出力が保存される");
    let parsed = rss_proxy::parse::parse(xml.as_bytes()).unwrap();
    assert_eq!(parsed.items.len(), 70);
    // Processor が適用済みであること (単一形式 7 件の description が空)
    let cleared = parsed
        .items
        .iter()
        .filter(|i| i.description.as_deref().unwrap_or("").is_empty())
        .count();
    assert_eq!(cleared, 7);

    let after = store.feed_by_slug("news").unwrap().unwrap();
    assert_eq!(after.fail_count, 0);
    assert_eq!(after.etag.as_deref(), Some(common::ETAG));
    assert_eq!(
        after.title.as_deref(),
        Some("ヘッドライン - 最新 - Google ニュース")
    );
    assert!(after.next_fetch_at > 0);
}

#[tokio::test]
async fn not_modified_skips_processing_but_reschedules() {
    let (url, upstream) = common::serve(FIXTURE).await;
    let (store, _id) = store_with_feed(&url);
    let client = rss_proxy::fetch::client();

    let feed = store.feed_by_slug("news").unwrap().unwrap();
    refresh(&store, &client, &feed).await.unwrap();
    let first = store.output("news").unwrap().unwrap();

    // 2 回目は ETag が付くので 304 が返り、本文は取り直されない
    let feed = store.feed_by_slug("news").unwrap().unwrap();
    let next_before = feed.next_fetch_at;
    refresh(&store, &client, &feed).await.unwrap();

    assert_eq!(upstream.hits(), 2);
    assert_eq!(store.output("news").unwrap().unwrap(), first);
    assert!(store.feed_by_slug("news").unwrap().unwrap().next_fetch_at >= next_before);
}

#[tokio::test]
async fn failure_keeps_the_previous_output() {
    let (url, upstream) = common::serve(FIXTURE).await;
    let (store, _id) = store_with_feed(&url);
    let client = rss_proxy::fetch::client();

    let feed = store.feed_by_slug("news").unwrap().unwrap();
    refresh(&store, &client, &feed).await.unwrap();
    let good = store.output("news").unwrap().unwrap();

    upstream.set_failing(true);
    let feed = store.feed_by_slug("news").unwrap().unwrap();
    assert!(refresh(&store, &client, &feed).await.is_err());

    assert_eq!(
        store.output("news").unwrap().unwrap(),
        good,
        "配信内容は維持される"
    );
    let after = store.feed_by_slug("news").unwrap().unwrap();
    assert_eq!(after.fail_count, 1);
    assert!(after.last_error.is_some());
}

#[test]
fn backoff_grows_then_caps_at_six_hours() {
    assert!(backoff_secs(0, 900) >= 900);
    assert!(backoff_secs(1, 900) > backoff_secs(0, 900));
    assert!(backoff_secs(3, 900) > backoff_secs(1, 900));
    assert_eq!(backoff_secs(100, 900), 6 * 3600);
    assert_eq!(backoff_secs(100, 900), backoff_secs(50, 900));
}
