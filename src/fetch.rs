//! 上流フィードの HTTP 取得。条件付き GET に対応する。

use std::net::IpAddr;

use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, USER_AGENT};
use reqwest::redirect::Policy;
use reqwest::{Client, StatusCode};

/// 受け取る本文の上限。上流は信用できないので、いくらでも読むことはしない。
/// 実測では Google ニュースのフィードが 200KB 程度。10MB あれば通常の用途には十分。
pub const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

/// 追従するリダイレクトの上限。
const MAX_REDIRECTS: usize = 5;

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("unexpected status: {0}")]
    Status(StatusCode),
    #[error("本文が大きすぎます ({0} バイトを超えました)")]
    TooLarge(usize),
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
        // 上流はリダイレクト先を自由に指定できる。内部アドレスへ誘導されると、
        // 公開していないサービスの内容を取りに行ってしまう
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.stop();
            }
            match attempt.url().host_str() {
                Some(host) if is_internal_host(host) => attempt.stop(),
                _ => attempt.follow(),
            }
        }))
        .build()
        .expect("HTTP クライアントの構築に失敗")
}

/// 外部に公開されていない宛先か。リダイレクトの追従先を判定する。
///
/// 名前解決の結果までは見ていないので、DNS を内部アドレスに向ける手口は防げない。
/// ホスト名で明示的に内部を指す場合を弾くための、最初の一段。
pub fn is_internal_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()   // 169.254.0.0/16 (クラウドのメタデータ)
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_documentation()
        }
        Ok(IpAddr::V6(ip)) => {
            ip.is_loopback()
                || ip.is_unspecified()
                // ユニークローカル (fc00::/7) とリンクローカル (fe80::/10)
                || matches!(ip.segments()[0] & 0xfe00, 0xfc00)
                || matches!(ip.segments()[0] & 0xffc0, 0xfe80)
        }
        // ホスト名は解決してみないと分からない
        Err(_) => false,
    }
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

    // Content-Length があれば読む前に弾く
    if let Some(len) = res.content_length()
        && len > MAX_BODY_BYTES as u64
    {
        return Err(FetchError::TooLarge(MAX_BODY_BYTES));
    }

    Ok(Fetched::Body {
        bytes: read_capped(res).await?,
        etag,
        last_modified,
    })
}

/// 上限まで読み、超えたら打ち切る。Content-Length を偽る上流にも耐える。
async fn read_capped(mut res: reqwest::Response) -> Result<Vec<u8>, FetchError> {
    let mut body = Vec::new();
    while let Some(chunk) = res.chunk().await? {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(FetchError::TooLarge(MAX_BODY_BYTES));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
