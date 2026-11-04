mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rss_proxy::fetch::{client, fetch, is_internal_host};
use rss_proxy::scheduler::refresh;
use rss_proxy::store::{NewFeed, Store};

/// 上流フィードはリダイレクト先を自由に指定できる。
/// 内部アドレスへ誘導されると、公開されていないサービスの内容を取りに行ってしまう。
#[test]
fn internal_destinations_are_recognised() {
    for host in [
        "localhost",
        "127.0.0.1",
        "127.1.2.3",
        "::1",
        "0.0.0.0",
        "169.254.169.254", // クラウドのメタデータ
        "10.0.0.1",
        "192.168.1.1",
        "172.16.0.1",
        "172.31.255.255",
        "[::1]",
        "fd00::1",
    ] {
        assert!(is_internal_host(host), "{host} を内部と判定していない");
    }
}

#[test]
fn public_destinations_are_allowed() {
    for host in [
        "news.google.com",
        "www.publickey1.jp",
        "8.8.8.8",
        "172.32.0.1", // 172.16/12 の外
        "11.0.0.1",
        "192.169.0.1", // 192.168/16 の外
    ] {
        assert!(!is_internal_host(host), "{host} を誤って内部と判定している");
    }
}

/// IPv4 を別の形で書いた宛先も内部として扱う。埋め込まれた先が同じサービスだから。
#[test]
fn internal_addresses_in_other_forms_are_recognised() {
    for host in [
        "[::ffff:127.0.0.1]",
        "[::ffff:169.254.169.254]",
        "[64:ff9b::7f00:1]",
        "[2002:7f00:1::]",
        "0.1.2.3",
        "100.64.0.1", // CGNAT
        "224.0.0.1",  // マルチキャスト
    ] {
        assert!(is_internal_host(host), "{host} を内部と判定していない");
    }
}

/// 名前解決の結果が内部アドレスなら接続しない。ホスト名の字面では判別できない手口。
/// `localhost` はテストで外部 DNS に頼らずループバックへ解決できる名前として使う。
#[tokio::test]
async fn hostnames_resolving_to_internal_addresses_are_not_contacted() {
    let (url, upstream) = common::serve("<rss/>").await;
    let url = url.replace("127.0.0.1", "localhost");

    assert!(fetch(&client(), &url, None, None).await.is_err());
    assert_eq!(upstream.hits(), 0, "内部アドレスへ接続している");
}

/// 登録時に IP アドレスで書いた宛先は、管理者の意思として従来どおり取得する。
#[tokio::test]
async fn registered_ip_addresses_are_still_fetched() {
    let (url, upstream) = common::serve("<rss/>").await;

    assert!(fetch(&client(), &url, None, None).await.is_ok());
    assert_eq!(upstream.hits(), 1);
}

/// item の link は上流が自由に書ける。IP アドレスで内部を指していても取りに行かない。
#[tokio::test]
async fn item_links_pointing_at_internal_addresses_are_not_fetched() {
    let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = target.local_addr().unwrap();
    let contacted = Arc::new(AtomicUsize::new(0));
    let counter = contacted.clone();
    tokio::spawn(async move {
        while target.accept().await.is_ok() {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });

    let rss = format!(
        r#"<rss version="2.0"><channel><title>t</title><item><title>x</title><link>http://{addr}/0_VPWW53_280000.xml</link></item></channel></rss>"#
    );
    let (url, _up) = common::serve(&rss).await;
    let store = Store::open_in_memory().unwrap();
    store
        .add_feed(&NewFeed {
            slug: Some("jma".into()),
            label: None,
            url,
            interval_secs: 900,
        })
        .unwrap();
    let feed = store.feed_by_slug("jma").unwrap().unwrap();
    store
        .set_processors(
            feed.id,
            &[("jma_warning".into(), r#"{"areas":["神戸市"]}"#.into())],
        )
        .unwrap();

    refresh(&store, &client(), &feed).await.unwrap();

    assert_eq!(
        contacted.load(Ordering::SeqCst),
        0,
        "内部アドレスへ接続している"
    );
}

/// 媒体の判定と URL の解釈がずれる書き方でも、内部アドレスへは接続しない。
/// `\` は接続側では `/` と同じ扱いなので、接続先は `@` の前のアドレスになる。
#[tokio::test]
async fn paywall_lookups_do_not_reach_internal_addresses() {
    let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = target.local_addr().unwrap();
    let contacted = Arc::new(AtomicUsize::new(0));
    let counter = contacted.clone();
    tokio::spawn(async move {
        while target.accept().await.is_ok() {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });

    let store = Store::open_in_memory().unwrap();
    let feed = rss_proxy::model::Feed {
        title: "t".into(),
        link: None,
        description: None,
        updated: None,
        items: vec![rss_proxy::model::Item {
            id: None,
            title: Some("t".into()),
            link: Some(format!("http://{addr}\\@www.yomiuri.co.jp/x")),
            description: None,
            published: None,
            authors: vec![],
            categories: vec![],
            paywalled: None,
        }],
    };

    rss_proxy::scheduler::classify_items(&store, &client(), feed, 10).await;

    assert_eq!(
        contacted.load(Ordering::SeqCst),
        0,
        "内部アドレスへ接続している"
    );
}
