pub mod serve;
pub mod ui;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::routing::{get, post};

use crate::store::Store;

// ponytail: 1 接続を Mutex で共有。rusqlite の Connection は Sync ではないため。
// 配信は 1 行読むだけで競合しない。必要になったら接続プールに置き換える。
pub type SharedStore = Arc<Mutex<Store>>;

pub fn app(store: Store) -> Router {
    Router::new()
        .route("/feeds/{name}", get(serve::feed))
        .route("/", get(ui::index))
        .route("/ui/feeds", post(ui::add))
        .route("/ui/feeds/{name}", get(ui::show))
        .route("/ui/feeds/{name}/delete", post(ui::delete))
        .route("/ui/feeds/{name}/processors", post(ui::set_chain))
        .route("/ui/feeds/{name}/fetch", post(ui::fetch_now))
        // 稼働中のバージョンを機械的に取得できるようにする。更新の有無の確認に使う
        .route(
            "/healthz",
            get(|| async {
                axum::Json(serde_json::json!({
                    "status": "ok",
                    "version": env!("CARGO_PKG_VERSION"),
                }))
            }),
        )
        .with_state(Arc::new(Mutex::new(store)))
}
