use rss_proxy::store::{NewFeed, Store};
use rss_proxy::web;

async fn serve_with(setup: impl FnOnce(&Store)) -> String {
    let store = Store::open_in_memory().unwrap();
    setup(&store);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            web::app(store).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn serves_the_stored_output() {
    let base = serve_with(|store| {
        store
            .add_feed(&NewFeed {
                slug: Some("news".into()),
                label: None,
                url: "https://example.com/f.xml".into(),
                interval_secs: 900,
            })
            .unwrap();
        let id = store.feed_by_slug("news").unwrap().unwrap().id;
        store.set_output(id, "<rss>済</rss>").unwrap();
    })
    .await;

    let res = reqwest::get(format!("{base}/feeds/news")).await.unwrap();
    assert_eq!(res.status(), 200);
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("application/rss+xml")
    );
    assert_eq!(res.text().await.unwrap(), "<rss>済</rss>");
}

#[tokio::test]
async fn unknown_feed_is_404() {
    let base = serve_with(|_| {}).await;
    let res = reqwest::get(format!("{base}/feeds/missing")).await.unwrap();
    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn registered_but_never_fetched_feed_is_404() {
    let base = serve_with(|store| {
        store
            .add_feed(&NewFeed {
                slug: Some("news".into()),
                label: None,
                url: "https://example.com/f.xml".into(),
                interval_secs: 900,
            })
            .unwrap();
    })
    .await;

    let res = reqwest::get(format!("{base}/feeds/news")).await.unwrap();
    assert_eq!(res.status(), 404, "まだ取得していないフィードは配信しない");
}

#[tokio::test]
async fn healthz_is_ok() {
    let base = serve_with(|_| {}).await;
    let res = reqwest::get(format!("{base}/healthz")).await.unwrap();
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn healthz_reports_the_version_for_update_checks() {
    let base = serve_with(|_| {}).await;
    let body = reqwest::get(format!("{base}/healthz"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    // 稼働中のバージョンを機械的に取得できるようにする
    let json: serde_json::Value = serde_json::from_str(&body).expect("JSON で返る");
    assert_eq!(json["status"], "ok");
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
}
