pub mod serve;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::routing::get;

use crate::store::Store;

// ponytail: 1 接続を Mutex で共有。rusqlite の Connection は Sync ではないため。
// 配信は 1 行読むだけで競合しない。必要になったら接続プールに置き換える。
pub type SharedStore = Arc<Mutex<Store>>;

pub fn app(store: Store) -> Router {
    Router::new()
        .route("/feeds/{name}", get(serve::feed))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(Arc::new(Mutex::new(store)))
}
