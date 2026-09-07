use rss_proxy::store::{NewFeed, Store};
use rss_proxy::web;

async fn serve(setup: impl FnOnce(&Store)) -> (String, reqwest::Client) {
    let store = Store::open_in_memory().unwrap();
    setup(&store);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, web::app(store)).await.unwrap() });
    (
        format!("http://{addr}"),
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
    )
}

fn feed(name: &str) -> NewFeed {
    NewFeed {
        name: name.into(),
        url: format!("https://example.com/{name}.xml"),
        interval_secs: 900,
    }
}

#[tokio::test]
async fn index_lists_registered_feeds() {
    let (base, c) = serve(|s| {
        s.add_feed(&feed("news")).unwrap();
    })
    .await;

    let body = c.get(&base).send().await.unwrap().text().await.unwrap();
    assert!(body.contains("news"));
    assert!(body.contains("/feeds/news"), "配信 URL が示される");
}

#[tokio::test]
async fn upstream_text_is_html_escaped() {
    let (base, c) = serve(|s| {
        let id = s.add_feed(&feed("news")).unwrap();
        // 上流フィード由来の文字列がそのまま管理画面に載らないこと
        s.set_title(id, "<script>alert(1)</script>").unwrap();
        s.mark_failure(id, "<img src=x onerror=alert(2)>", 0)
            .unwrap();
    })
    .await;

    let body = c.get(&base).send().await.unwrap().text().await.unwrap();
    assert!(!body.contains("<script>"), "title がエスケープされていない");
    assert!(
        !body.contains("<img "),
        "エラー文言がエスケープされていない"
    );
    assert!(body.contains("&lt;script&gt;"));
    assert!(body.contains("&lt;img "));
}

#[tokio::test]
async fn adds_and_deletes_a_feed_through_forms() {
    let (base, c) = serve(|_| {}).await;

    let res = c
        .post(format!("{base}/ui/feeds"))
        .form(&[
            ("name", "news"),
            ("url", "https://example.com/news.xml"),
            ("interval", "600"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 303);

    let body = c.get(&base).send().await.unwrap().text().await.unwrap();
    assert!(body.contains("news"));

    let res = c
        .post(format!("{base}/ui/feeds/news/delete"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 303);
    let body = c.get(&base).send().await.unwrap().text().await.unwrap();
    assert!(!body.contains("news.xml"));
}

#[tokio::test]
async fn edits_the_processor_chain_as_text() {
    let (base, c) = serve(|s| {
        s.add_feed(&feed("news")).unwrap();
    })
    .await;

    let res = c
        .post(format!("{base}/ui/feeds/news/processors"))
        .form(&[("chain", "google_news_cluster\ndedupe {\"key\":\"guid\"}\n")])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 303);

    let body = c
        .get(format!("{base}/ui/feeds/news"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("google_news_cluster"));
    assert!(body.contains("dedupe"));
}

#[tokio::test]
async fn rejects_an_invalid_chain_without_saving() {
    let (base, c) = serve(|s| {
        s.add_feed(&feed("news")).unwrap();
    })
    .await;

    let res = c
        .post(format!("{base}/ui/feeds/news/processors"))
        .form(&[("chain", "telepathy")])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);

    let body = c
        .get(format!("{base}/ui/feeds/news"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!body.contains("telepathy"), "壊れた設定は保存されない");
}

#[tokio::test]
async fn fetch_now_makes_the_feed_due() {
    let (base, c) = serve(|s| {
        let id = s.add_feed(&feed("news")).unwrap();
        s.mark_success(id, None, None, 9_999_999_999).unwrap();
    })
    .await;

    let res = c
        .post(format!("{base}/ui/feeds/news/fetch"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 303);
}

#[tokio::test]
async fn unknown_feed_pages_are_404() {
    let (base, c) = serve(|_| {}).await;
    assert_eq!(
        c.get(format!("{base}/ui/feeds/missing"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
}
