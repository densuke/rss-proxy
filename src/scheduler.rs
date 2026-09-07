//! 巡回スケジューラ。
//!
//! フィードごとにタスクを常駐させず、1 本の tick ループが
//! 次回取得時刻を過ぎたフィードをまとめて処理する。

use std::time::Duration;

use chrono::Utc;
use reqwest::Client;

use crate::fetch::{self, Fetched};
use crate::store::{Feed, Store};
use crate::{parse, proc, render};

const TICK: Duration = Duration::from_secs(60);
const MAX_BACKOFF_SECS: i64 = 6 * 3600;

#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    #[error(transparent)]
    Fetch(#[from] fetch::FetchError),
    #[error(transparent)]
    Parse(#[from] parse::ParseError),
    #[error(transparent)]
    Processor(#[from] proc::ProcessorError),
    #[error(transparent)]
    Store(#[from] rusqlite::Error),
}

/// 連続失敗時の待ち時間。指数的に伸ばし、6 時間で頭打ちにする。
pub fn backoff_secs(fail_count: i64, interval_secs: i64) -> i64 {
    let shift = fail_count.clamp(0, 32) as u32;
    interval_secs
        .saturating_mul(1i64.checked_shl(shift).unwrap_or(i64::MAX))
        .min(MAX_BACKOFF_SECS)
        .max(interval_secs.min(MAX_BACKOFF_SECS))
}

/// 1 フィードを取得して処理し、結果を保存する。
pub async fn refresh(store: &Store, client: &Client, feed: &Feed) -> Result<(), RefreshError> {
    match run(store, client, feed).await {
        Ok(()) => Ok(()),
        Err(e) => {
            // 失敗しても直前の出力は残す。次回取得だけ後ろへずらす
            let next = Utc::now().timestamp() + backoff_secs(feed.fail_count, feed.interval_secs);
            store.mark_failure(feed.id, &e.to_string(), next)?;
            Err(e)
        }
    }
}

async fn run(store: &Store, client: &Client, feed: &Feed) -> Result<(), RefreshError> {
    let fetched = fetch::fetch(
        client,
        &feed.url,
        feed.etag.as_deref(),
        feed.last_modified.as_deref(),
    )
    .await?;

    let next = Utc::now().timestamp() + feed.interval_secs;

    let Fetched::Body {
        bytes,
        etag,
        last_modified,
    } = fetched
    else {
        // 304。解析も Processor 適用も行わず、次回取得時刻だけ更新する
        store.mark_success(
            feed.id,
            feed.etag.as_deref(),
            feed.last_modified.as_deref(),
            next,
        )?;
        return Ok(());
    };

    let mut parsed = parse::parse(&bytes)?;
    // 上流のタイトルは記録として残しつつ、表示名があれば配信する title を差し替える。
    // 検索フィードの title は検索クエリそのままで、購読すると読みづらいため
    let title = parsed.title.clone();
    if let Some(label) = &feed.label {
        parsed.title = label.clone();
    }

    // グローバル連鎖が先、フィード固有が後。
    // 全体に効かせたい整形 (全角の正規化など) を先に済ませてから、
    // フィード固有の判定がその結果に対して働くようにする
    let specs = store
        .global_processors()?
        .into_iter()
        .chain(store.processors(feed.id)?)
        .collect::<Vec<_>>();
    let chain = specs
        .iter()
        .map(|(kind, params)| proc::build(kind, params))
        .collect::<Result<Vec<_>, _>>()?;

    let processed = chain.iter().try_fold(parsed, |feed, p| p.apply(feed))?;

    store.set_output(feed.id, &render::to_rss2(&processed))?;
    if !title.is_empty() {
        store.set_title(feed.id, &title)?;
    }
    store.mark_success(feed.id, etag.as_deref(), last_modified.as_deref(), next)?;
    Ok(())
}

/// 60 秒ごとに巡回対象を拾って処理し続ける。
/// Store は Sync ではないため、このタスクが所有する接続を使う。
pub async fn run_loop(store: Store, client: Client) {
    loop {
        if let Err(e) = tick(&store, &client).await {
            eprintln!("scheduler: {e}");
        }
        tokio::time::sleep(TICK).await;
    }
}

async fn tick(store: &Store, client: &Client) -> Result<(), rusqlite::Error> {
    // ponytail: 逐次処理。Store が持つ SQLite 接続は Sync ではないため共有した並行実行ができず、
    // 巡回間隔に対してフィード数が十分少ないうちは逐次で足りる。
    // 必要になったら接続プール + JoinSet に置き換える。
    for feed in store.due_feeds(Utc::now().timestamp())? {
        if let Err(e) = refresh(store, client, &feed).await {
            eprintln!("refresh {}: {e}", feed.slug);
        }
    }
    Ok(())
}
