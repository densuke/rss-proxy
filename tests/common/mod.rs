//! テスト用の上流フィードサーバー。
//! 条件付き GET に応答し、リクエスト回数を数える。

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;

pub const ETAG: &str = "W/\"fixture-1\"";

#[derive(Default)]
pub struct Upstream {
    pub hits: AtomicUsize,
    pub body: std::sync::Mutex<String>,
    pub fail: std::sync::atomic::AtomicBool,
}

impl Upstream {
    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
    pub fn set_body(&self, body: &str) {
        *self.body.lock().unwrap() = body.to_string();
    }
    pub fn set_failing(&self, failing: bool) {
        self.fail.store(failing, Ordering::SeqCst);
    }
}

/// テストサーバーを起動し、(URL, 状態) を返す。
pub async fn serve(body: &str) -> (String, Arc<Upstream>) {
    let state = Arc::new(Upstream::default());
    state.set_body(body);

    let app = axum::Router::new()
        .route("/feed.xml", axum::routing::get(handler))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    (format!("http://{addr}/feed.xml"), state)
}

async fn handler(State(state): State<Arc<Upstream>>, headers: HeaderMap) -> impl IntoResponse {
    state.hits.fetch_add(1, Ordering::SeqCst);

    if state.fail.load(Ordering::SeqCst) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            HeaderMap::new(),
            String::new(),
        );
    }
    if headers.get("if-none-match").map(|v| v == ETAG) == Some(true) {
        return (StatusCode::NOT_MODIFIED, HeaderMap::new(), String::new());
    }

    let mut out = HeaderMap::new();
    out.insert("etag", ETAG.parse().unwrap());
    out.insert("content-type", "application/rss+xml".parse().unwrap());
    (StatusCode::OK, out, state.body.lock().unwrap().clone())
}
