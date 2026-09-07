//! 処理済みフィードの配信。保存済みの出力を返すだけで、上流には触らない。

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::web::SharedStore;

pub async fn feed(State(store): State<SharedStore>, Path(name): Path<String>) -> Response {
    let output = store.lock().expect("store lock").output(&name);

    match output {
        Ok(Some(xml)) => (
            [(header::CONTENT_TYPE, "application/rss+xml; charset=utf-8")],
            xml,
        )
            .into_response(),
        // 未登録のフィードも、まだ一度も取得できていないフィードも 404
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            eprintln!("serve {name}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
