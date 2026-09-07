//! 管理画面へのアクセス制御。
//!
//! 認証情報が設定されていれば HTTP Basic 認証を要求する。設定されていない場合は
//! ループバックからのアクセスだけを通す。設定漏れがそのまま全公開になるのを防ぐため。

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine;

use crate::auth::Admin;

pub async fn require_admin(
    State(admin): State<Option<Admin>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    if let Err(response) = check_origin(&request) {
        return response;
    }

    let Some(admin) = admin else {
        // 未設定。ループバックからのみ許可する
        let local = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip().is_loopback())
            // 接続元が分からない場合は拒否する側に倒す
            .unwrap_or(false);
        return if local {
            next.run(request).await
        } else {
            (
                StatusCode::FORBIDDEN,
                "管理画面は認証が未設定のため、ローカルからのみ利用できます。\
                 RSS_PROXY_ADMIN_USER と RSS_PROXY_ADMIN_PASSWORD_HASH を設定してください。",
            )
                .into_response()
        };
    };

    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_basic);

    match presented {
        Some((user, password)) if admin.authenticates(&user, &password) => next.run(request).await,
        _ => unauthorized(),
    }
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(
            header::WWW_AUTHENTICATE,
            r#"Basic realm="rss-proxy", charset="UTF-8""#,
        )],
    )
        .into_response()
}

/// `Basic <base64(user:password)>` を分解する。
fn parse_basic(header: &str) -> Option<(String, String)> {
    let encoded = header.strip_prefix("Basic ")?.trim();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (user, password) = decoded.split_once(':')?;
    Some((user.to_string(), password.to_string()))
}

/// 状態を変える要求が別オリジンから送られていないか確かめる。
///
/// Basic 認証の資格情報はブラウザが自動で付与するため、外部サイトに置かれた
/// フォームからの送信も認証済みとして届いてしまう (CSRF)。ブラウザは別オリジンへの
/// POST に必ず `Origin` を付けるので、それが自分自身でなければ拒否する。
///
/// `Origin` も `Referer` もない要求は通す。ブラウザ以外からの操作 (curl など) で
/// あり、資格情報が自動付与される経路ではないため。
fn check_origin(request: &axum::extract::Request) -> Result<(), Response> {
    if request.method() == Method::GET || request.method() == Method::HEAD {
        return Ok(());
    }

    let header_value = |name: header::HeaderName| {
        request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let Some(source) = header_value(header::ORIGIN).or_else(|| header_value(header::REFERER))
    else {
        return Ok(());
    };
    let Some(host) = header_value(header::HOST) else {
        return Err((StatusCode::FORBIDDEN, "Host ヘッダがありません").into_response());
    };

    // scheme を除いた authority で比べる。前段が TLS を終端すると scheme は一致しない
    let authority = source
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(&source);
    let authority = authority.split('/').next().unwrap_or(authority);

    if authority == host {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            "別オリジンからの操作は受け付けません",
        )
            .into_response())
    }
}
