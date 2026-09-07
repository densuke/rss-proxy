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

/// 検索フィードの channel title は検索クエリそのままで、購読すると読みづらい。
/// 表示名を設定してあれば、それを配信する title にも使う。
#[tokio::test]
async fn the_label_replaces_the_channel_title() {
    let (url, _up) = common::serve(FIXTURE).await;
    let store = Store::open_in_memory().unwrap();
    store
        .add_feed(&NewFeed {
            slug: Some("news".into()),
            label: Some("主要ニュース".into()),
            url: url.clone(),
            interval_secs: 900,
        })
        .unwrap();
    let feed = store.feed_by_slug("news").unwrap().unwrap();

    refresh(&store, &rss_proxy::fetch::client(), &feed)
        .await
        .unwrap();

    let xml = store.output("news").unwrap().unwrap();
    let parsed = rss_proxy::parse::parse(xml.as_bytes()).unwrap();
    assert_eq!(parsed.title, "主要ニュース");

    // 上流のタイトルは記録として残す
    let after = store.feed_by_slug("news").unwrap().unwrap();
    assert_eq!(
        after.title.as_deref(),
        Some("ヘッドライン - 最新 - Google ニュース")
    );
}

#[tokio::test]
async fn without_a_label_the_upstream_title_is_used() {
    let (url, _up) = common::serve(FIXTURE).await;
    let (store, _id) = store_with_feed(&url);
    let feed = store.feed_by_slug("news").unwrap().unwrap();

    refresh(&store, &rss_proxy::fetch::client(), &feed)
        .await
        .unwrap();

    let xml = store.output("news").unwrap().unwrap();
    let parsed = rss_proxy::parse::parse(xml.as_bytes()).unwrap();
    assert_eq!(parsed.title, "ヘッドライン - 最新 - Google ニュース");
}

/// 設定を変えたのに 304 で処理がスキップされると、配信内容が古いままになる。
/// 検証子を消してあれば次の取得で必ず作り直される。
#[tokio::test]
async fn clearing_the_validators_forces_a_full_fetch() {
    let (url, upstream) = common::serve(FIXTURE).await;
    let (store, id) = store_with_feed(&url);
    let client = rss_proxy::fetch::client();

    let feed = store.feed_by_slug("news").unwrap().unwrap();
    refresh(&store, &client, &feed).await.unwrap();
    assert!(store.feed_by_slug("news").unwrap().unwrap().etag.is_some());

    // そのままだと 304 になり、処理は走らない
    let feed = store.feed_by_slug("news").unwrap().unwrap();
    refresh(&store, &client, &feed).await.unwrap();
    // 表示名を変えたうえで検証子を消す
    store.rename(id, "news", Some("新しい表示名")).unwrap();
    store.clear_validators(id).unwrap();
    let feed = store.feed_by_slug("news").unwrap().unwrap();
    assert!(feed.etag.is_none() && feed.last_modified.is_none());

    refresh(&store, &client, &feed).await.unwrap();
    assert_eq!(upstream.hits(), 3);

    let xml = store.output("news").unwrap().unwrap();
    assert_eq!(
        rss_proxy::parse::parse(xml.as_bytes()).unwrap().title,
        "新しい表示名",
        "設定変更が配信内容に反映される"
    );
}
