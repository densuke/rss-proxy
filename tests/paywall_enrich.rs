mod common;

use rss_proxy::paywall::Access;
use rss_proxy::scheduler::classify_items;
use rss_proxy::store::Store;

fn feed_with(links: &[&str]) -> rss_proxy::model::Feed {
    rss_proxy::model::Feed {
        title: "t".into(),
        link: None,
        description: None,
        updated: None,
        items: links
            .iter()
            .map(|l| rss_proxy::model::Item {
                id: None,
                title: Some("t".into()),
                link: Some((*l).to_string()),
                description: None,
                published: None,
                authors: vec![],
                categories: vec![],
                paywalled: None,
            })
            .collect(),
    }
}

/// ルールのない媒体は取りに行かない。無関係なフィードに負荷をかけないため。
#[tokio::test]
async fn feeds_without_a_known_publisher_cost_nothing() {
    let store = Store::open_in_memory().unwrap();
    let (url, upstream) = common::serve("<html/>").await;

    let feed = feed_with(&[&url, "https://www.publickey1.jp/blog/x.html"]);
    let out = classify_items(&store, &rss_proxy::fetch::client(), feed, 10).await;

    assert_eq!(upstream.hits(), 0, "ルールのない URL を取りに行っている");
    assert!(out.items.iter().all(|i| i.paywalled.is_none()));
}

/// 一度判定した記事は覚えておき、次からは取りに行かない。
#[tokio::test]
async fn a_cached_result_is_reused() {
    let store = Store::open_in_memory().unwrap();
    let url = "https://www.yomiuri.co.jp/national/20260908-XYZ/";
    store.remember_paywall(url, Access::Paid).unwrap();

    let out = classify_items(&store, &rss_proxy::fetch::client(), feed_with(&[url]), 10).await;
    assert_eq!(out.items[0].paywalled, Some(true));
}

/// 1 回の巡回で取りに行く数に上限を置く。新着が大量にあっても媒体を叩き続けない。
#[tokio::test]
async fn the_number_of_lookups_per_refresh_is_capped() {
    let store = Store::open_in_memory().unwrap();
    let urls: Vec<String> = (0..5)
        .map(|i| format!("https://www.yomiuri.co.jp/national/2026090{i}-X/"))
        .collect();
    let refs: Vec<&str> = urls.iter().map(String::as_str).collect();

    // 上限 0 なら 1 件も判定されない (取得は発生しない)
    let out = classify_items(&store, &rss_proxy::fetch::client(), feed_with(&refs), 0).await;
    assert!(out.items.iter().all(|i| i.paywalled.is_none()));
    for u in &urls {
        assert_eq!(store.paywall_cached(u).unwrap(), None);
    }
}
