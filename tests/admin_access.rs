use std::net::SocketAddr;

use base64::Engine;
use rss_proxy::auth::{Admin, hash_password};
use rss_proxy::store::{NewFeed, Store};
use rss_proxy::web;

/// 認証設定つきでサーバーを起動する。
async fn serve(admin: Option<Admin>) -> (String, reqwest::Client) {
    let store = Store::open_in_memory().unwrap();
    let id = store
        .add_feed(&NewFeed {
            name: "news".into(),
            url: "https://example.com/f.xml".into(),
            interval_secs: 900,
        })
        .unwrap();
    store.set_output(id, "<rss/>").unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            web::app_with_auth(store, admin).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });

    (
        format!("http://{addr}"),
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
    )
}

fn basic(user: &str, password: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
    )
}

fn admin() -> Admin {
    Admin::new(Some("admin".into()), Some(hash_password("秘密").unwrap())).unwrap()
}

#[tokio::test]
async fn feeds_are_served_without_credentials() {
    let (base, c) = serve(Some(admin())).await;

    // 配信は URL を知っていれば読める。認証をかけると RSS リーダーが読めなくなる
    assert_eq!(
        c.get(format!("{base}/feeds/news"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        c.get(format!("{base}/healthz"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
}

#[tokio::test]
async fn admin_pages_require_credentials() {
    let (base, c) = serve(Some(admin())).await;

    let res = c.get(&base).send().await.unwrap();
    assert_eq!(res.status(), 401);
    assert!(
        res.headers()["www-authenticate"]
            .to_str()
            .unwrap()
            .starts_with("Basic"),
        "ブラウザに入力を促す"
    );

    // 書き込み側も守られている
    assert_eq!(
        c.post(format!("{base}/ui/feeds/news/delete"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
}

#[tokio::test]
async fn correct_credentials_are_accepted() {
    let (base, c) = serve(Some(admin())).await;

    let res = c
        .get(&base)
        .header("authorization", basic("admin", "秘密"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn wrong_credentials_are_rejected() {
    let (base, c) = serve(Some(admin())).await;

    for header in [
        basic("admin", "違う"),
        basic("別人", "秘密"),
        "Basic ????".into(),
    ] {
        let res = c
            .get(&base)
            .header("authorization", header)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 401);
    }
}

#[tokio::test]
async fn without_credentials_admin_is_reachable_from_localhost_only() {
    // 認証未設定でも手元での試用は今までどおり動く
    let (base, c) = serve(None).await;
    assert_eq!(c.get(&base).send().await.unwrap().status(), 200);
    assert_eq!(
        c.get(format!("{base}/feeds/news"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
}

/// Basic 認証はブラウザが自動で付与するため、外部サイトからのフォーム送信でも
/// 認証済みとして届く。状態を変える POST は同一オリジンからのみ受け付ける。
#[tokio::test]
async fn state_changing_posts_must_come_from_the_same_origin() {
    let (base, c) = serve(Some(admin())).await;
    let host = base.strip_prefix("http://").unwrap();
    let delete = format!("{base}/ui/feeds/news/delete");

    let res = c
        .post(&delete)
        .header("authorization", basic("admin", "秘密"))
        .header("origin", "https://evil.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403, "別オリジンからの POST は拒否する");

    let res = c
        .post(&delete)
        .header("authorization", basic("admin", "秘密"))
        .header("origin", format!("http://{host}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 303, "同一オリジンなら通る");
}

#[tokio::test]
async fn reads_are_not_affected_by_the_origin_check() {
    let (base, c) = serve(Some(admin())).await;

    let res = c
        .get(&base)
        .header("authorization", basic("admin", "秘密"))
        .header("origin", "https://evil.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "GET は状態を変えないので影響しない");
}

#[tokio::test]
async fn posts_without_an_origin_header_still_work_for_scripts() {
    // curl などブラウザ以外からの操作は Origin を送らない。
    // CSRF はブラウザが認証情報を自動付与することで成立するため、これは通してよい
    let (base, c) = serve(Some(admin())).await;

    let res = c
        .post(format!("{base}/ui/feeds/news/fetch"))
        .header("authorization", basic("admin", "秘密"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 303);
}
