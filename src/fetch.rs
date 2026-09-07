//! 上流フィードの HTTP 取得。条件付き GET に対応する。

use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, USER_AGENT};
use reqwest::{Client, StatusCode};

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("unexpected status: {0}")]
    Status(StatusCode),
}

pub enum Fetched {
    /// 304。前回から変化なし。
    NotModified,
    Body {
        bytes: Vec<u8>,
        etag: Option<String>,
        last_modified: Option<String>,
    },
}

pub fn client() -> Client {
    Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("HTTP クライアントの構築に失敗")
}

pub async fn fetch(
    client: &Client,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<Fetched, FetchError> {
    let mut req = client
        .get(url)
        .header(USER_AGENT, concat!("rss-proxy/", env!("CARGO_PKG_VERSION")));
    if let Some(etag) = etag {
        req = req.header(IF_NONE_MATCH, etag);
    }
    if let Some(lm) = last_modified {
        req = req.header(IF_MODIFIED_SINCE, lm);
    }

    let res = req.send().await?;
    if res.status() == StatusCode::NOT_MODIFIED {
        return Ok(Fetched::NotModified);
    }
    if !res.status().is_success() {
        return Err(FetchError::Status(res.status()));
    }

    let header = |name: reqwest::header::HeaderName| {
        res.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let (etag, last_modified) = (header(ETAG), header(LAST_MODIFIED));

    Ok(Fetched::Body {
        bytes: res.bytes().await?.to_vec(),
        etag,
        last_modified,
    })
}
