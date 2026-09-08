use rss_proxy::paywall::Access;
use rss_proxy::store::Store;

#[test]
fn results_are_remembered_so_each_article_is_fetched_once() {
    let s = Store::open_in_memory().unwrap();
    let url = "https://www.yomiuri.co.jp/national/20260908-XYZ/";

    assert_eq!(s.paywall_cached(url).unwrap(), None, "最初は未判定");

    s.remember_paywall(url, Access::Paid).unwrap();
    assert_eq!(s.paywall_cached(url).unwrap(), Some(Access::Paid));

    // 判定し直しても上書きできる。無料化された記事に追随するため
    s.remember_paywall(url, Access::Free).unwrap();
    assert_eq!(s.paywall_cached(url).unwrap(), Some(Access::Free));

    s.remember_paywall(url, Access::Unknown).unwrap();
    assert_eq!(s.paywall_cached(url).unwrap(), Some(Access::Unknown));
}

#[test]
fn old_entries_are_pruned() {
    let s = Store::open_in_memory().unwrap();
    s.remember_paywall("https://www.nikkei.com/article/A/", Access::Paid)
        .unwrap();
    s.remember_paywall("https://www.nikkei.com/article/B/", Access::Free)
        .unwrap();

    // 未来を基準にすれば、いま入れた分がすべて古い扱いになる
    let removed = s
        .prune_paywall_cache(chrono::Utc::now().timestamp() + 1)
        .unwrap();
    assert_eq!(removed, 2);
    assert_eq!(
        s.paywall_cached("https://www.nikkei.com/article/A/")
            .unwrap(),
        None
    );
}
