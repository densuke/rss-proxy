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
