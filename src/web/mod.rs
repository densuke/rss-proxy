pub mod guard;
pub mod serve;
pub mod ui;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::routing::{get, post};

use crate::auth::Admin;
use crate::store::Store;

// ponytail: 1 接続を Mutex で共有。rusqlite の Connection は Sync ではないため。
// 配信は 1 行読むだけで競合しない。必要になったら接続プールに置き換える。
pub type SharedStore = Arc<Mutex<Store>>;

/// 認証なし。手元での試用とテスト用。
pub fn app(store: Store) -> Router {
    app_with_auth(store, None)
}

/// 配信は誰でも読めるままにし、管理画面だけを保護する。
///
/// `admin` が `None` のときは認証を要求しない代わりに、管理画面へのアクセスを
/// ループバックからに限る。設定漏れがそのまま全公開になる事故を防ぐ。
pub fn app_with_auth(store: Store, admin: Option<Admin>) -> Router {
    let state = Arc::new(Mutex::new(store));

    let admin_routes = Router::new()
        .route("/", get(ui::index))
        .route("/ui/feeds", post(ui::add))
        .route("/ui/feeds/{name}", get(ui::show))
        .route("/ui/feeds/{name}/delete", post(ui::delete))
        .route("/ui/feeds/{name}/rename", post(ui::rename))
        .route("/ui/feeds/{name}/processors", post(ui::set_chain))
        .route("/ui/feeds/{name}/fetch", post(ui::fetch_now))
        .layer(axum::middleware::from_fn_with_state(
            admin,
            guard::require_admin,
        ));

    Router::new()
        // 配信は URL を知っていれば読める。ここに認証をかけると RSS リーダーが読めなくなる
        .route("/feeds/{name}", get(serve::feed))
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
        .merge(admin_routes)
        .with_state(state)
}
