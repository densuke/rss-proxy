mod common;

use rss_proxy::fetch::{Fetched, client, fetch};

const FIXTURE: &str = include_str!("fixtures/google_news_headline.xml");

#[tokio::test]
async fn fetches_body_and_validators() {
    let (url, upstream) = common::serve(FIXTURE).await;

    let got = fetch(&client(), &url, None, None).await.unwrap();
    let Fetched::Body { bytes, etag, .. } = got else {
        panic!("本文が返るはず");
    };
    assert_eq!(bytes.len(), FIXTURE.len());
    assert_eq!(etag.as_deref(), Some(common::ETAG));
    assert_eq!(upstream.hits(), 1);
}

#[tokio::test]
async fn sends_etag_and_handles_304() {
    let (url, upstream) = common::serve(FIXTURE).await;

    let got = fetch(&client(), &url, Some(common::ETAG), None)
        .await
        .unwrap();
    assert!(matches!(got, Fetched::NotModified));
    assert_eq!(upstream.hits(), 1);
}

#[tokio::test]
async fn server_errors_become_errors() {
    let (url, upstream) = common::serve(FIXTURE).await;
    upstream.set_failing(true);

    assert!(fetch(&client(), &url, None, None).await.is_err());
}

#[tokio::test]
async fn unreachable_host_is_an_error() {
    assert!(
        fetch(&client(), "http://127.0.0.1:1/feed.xml", None, None)
            .await
            .is_err()
    );
}

/// 上流は信用できない。巨大な本文を送りつけられてもメモリを食い潰さない。
#[tokio::test]
async fn oversized_responses_are_rejected() {
    let huge = "x".repeat(rss_proxy::fetch::MAX_BODY_BYTES + 1);
    let (url, _up) = common::serve(&huge).await;

    let err = match fetch(&client(), &url, None, None).await {
        Err(e) => e,
        Ok(_) => panic!("サイズ超過が素通りした"),
    };
    assert!(
        err.to_string().contains("大きすぎ"),
        "サイズ超過として扱われていない: {err}"
    );
}

#[tokio::test]
async fn a_body_within_the_limit_is_accepted() {
    let (url, _up) = common::serve(FIXTURE).await;
    assert!(fetch(&client(), &url, None, None).await.is_ok());
}
