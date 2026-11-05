//! 上流フィードの HTTP 取得。条件付き GET に対応する。

mod address;

use std::net::{IpAddr, SocketAddr};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
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

/// 解決結果に内部アドレスが 1 つでも混ざれば、名前解決を失敗させる。
///
/// 接続に使うアドレスそのものを検査するので、検査と接続の間に DNS の応答を
/// 変えられても (DNS rebinding) すり抜けない。IP アドレスで書かれた URL はここを通らない。
struct PublicOnly;

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let addrs: Vec<SocketAddr> =
                tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            if addrs.iter().any(|a| !address::is_public_ip(a.ip())) {
                return Err(format!("{} は内部アドレスに解決されます", name.as_str()).into());
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

pub fn client() -> Client {
    Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .dns_resolver(PublicOnly)
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

/// 外部に公開されていない宛先か。リダイレクトの追従先を字面で判定する。
///
/// ホスト名の解決結果は [`PublicOnly`] が接続時に検査する。
pub fn is_internal_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return true;
    }
    host.parse::<IpAddr>()
        .is_ok_and(|ip| !address::is_public_ip(ip))
}

/// 上流が書いた URL (item の link など) の宛先が内部か。接続側と同じ解釈で判定する。
/// IP アドレスは名前解決の検査を通らないので、取得の前にここで弾く。解釈できない URL も内部扱い。
pub fn is_internal_url(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(is_internal_host))
        .unwrap_or(true)
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
