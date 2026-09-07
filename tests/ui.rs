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

#[tokio::test]
async fn delivery_link_appears_only_after_a_successful_fetch() {
    let (base, c) = serve(|s| {
        let fetched = s.add_feed(&feed("fetched")).unwrap();
        s.set_output(fetched, "<rss/>").unwrap();
        s.add_feed(&feed("never")).unwrap();
    })
    .await;

    let body = c.get(&base).send().await.unwrap().text().await.unwrap();
    assert!(body.contains(r#"href="/feeds/fetched""#));
    // 一度も取得できていないフィードは 404 になるのでリンクにしない
    assert!(!body.contains(r#"href="/feeds/never""#));
}

#[tokio::test]
async fn index_shows_the_last_fetch_time() {
    let (base, c) = serve(|s| {
        let id = s.add_feed(&feed("news")).unwrap();
        s.mark_success(id, None, None, 0).unwrap();
    })
    .await;

    let body = c.get(&base).send().await.unwrap().text().await.unwrap();
    // オフセットは見出しに 1 度だけ出し、各行の値には付けない
    let offset = chrono::Local::now().format("%:z").to_string();
    assert!(
        body.contains(&format!("最終取得 ({offset})")),
        "見出しにオフセットがある"
    );

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    assert!(body.contains(&now), "{now} が出力に含まれていない");
    assert!(
        !body.contains(&format!("{now} {offset}")),
        "値にオフセットが重複して付いている"
    );
}

#[tokio::test]
async fn edit_page_lists_the_items_being_served() {
    let xml = r#"<?xml version="1.0"?><rss version="2.0"><channel>
      <title>t</title><link>https://example.com</link><description>d</description>
      <item><title>1 件目の見出し</title><link>https://example.com/1</link>
        <description>&lt;p&gt;本文の要約&lt;/p&gt;</description></item>
      <item><title>2 件目の見出し</title><link>https://example.com/2</link></item>
    </channel></rss>"#;

    let (base, c) = serve(|s| {
        let id = s.add_feed(&feed("news")).unwrap();
        s.set_output(id, xml).unwrap();
    })
    .await;

    let body = c
        .get(format!("{base}/ui/feeds/news"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert!(body.contains("配信中の内容"));
    assert!(body.contains("2 件"), "件数が出る");
    // フィードに書かれている順で並ぶ
    assert!(body.find("1 件目の見出し").unwrap() < body.find("2 件目の見出し").unwrap());
    // description は HTML を落として抜粋する
    assert!(body.contains("本文の要約"));
    assert!(!body.contains("<p>本文の要約</p>"));
}

#[tokio::test]
async fn edit_page_of_a_never_fetched_feed_says_so() {
    let (base, c) = serve(|s| {
        s.add_feed(&feed("news")).unwrap();
    })
    .await;

    let body = c
        .get(format!("{base}/ui/feeds/news"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("まだ取得されていません"));
}

#[tokio::test]
async fn saving_a_chain_fills_in_default_parameters() {
    let (base, c) = serve(|s| {
        s.add_feed(&feed("news")).unwrap();
    })
    .await;

    // 種別だけ書いて保存すると、既定値が埋まった形で残る
    let res = c
        .post(format!("{base}/ui/feeds/news/processors"))
        .form(&[("chain", "dedupe\ngoogle_news_cluster")])
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
    assert!(
        body.contains(r#"dedupe {&quot;key&quot;:&quot;link&quot;}"#),
        "既定値が書き戻されていない"
    );
}

#[tokio::test]
async fn edit_page_documents_each_processor() {
    let (base, c) = serve(|s| {
        s.add_feed(&feed("news")).unwrap();
    })
    .await;

    let body = c
        .get(format!("{base}/ui/feeds/news"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    for info in rss_proxy::proc::catalog() {
        assert!(body.contains(info.kind), "{} が載っていない", info.kind);
        assert!(
            body.contains(&html_escape(info.summary)),
            "{} の説明が載っていない",
            info.kind
        );
        for p in info.params {
            assert!(body.contains(p.name));
            assert!(body.contains(p.default));
        }
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[tokio::test]
async fn edit_page_shows_the_raw_fields_of_served_items() {
    let xml = r#"<?xml version="1.0"?><rss version="2.0"><channel>
      <title>t</title><link>https://example.com</link><description>d</description>
      <item><title>見出し</title><link>https://example.com/1</link>
        <guid isPermaLink="false">GUID-1</guid>
        <pubDate>Sat, 08 Aug 2026 21:54:30 +0900</pubDate>
        <category>ニュース</category>
        <description>本文</description></item>
    </channel></rss>"#;

    let (base, c) = serve(|s| {
        let id = s.add_feed(&feed("news")).unwrap();
        s.set_output(id, xml).unwrap();
    })
    .await;

    let body = c
        .get(format!("{base}/ui/feeds/news"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    // dedupe のキーを選ぶ材料として、各フィールドに何が入っているかを見せる
    assert!(body.contains("フィールド"));
    for field in ["guid", "link", "title", "published", "categories"] {
        assert!(body.contains(field), "{field} が載っていない");
    }
    assert!(body.contains("GUID-1"));
    assert!(body.contains("2026-08-08"), "時刻が表示される");
}
