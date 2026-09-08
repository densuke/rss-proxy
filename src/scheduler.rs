//! 巡回スケジューラ。
//!
//! フィードごとにタスクを常駐させず、1 本の tick ループが
//! 次回取得時刻を過ぎたフィードをまとめて処理する。

use std::time::Duration;

use chrono::Utc;
use reqwest::Client;

use crate::fetch::{self, Fetched};
use crate::model::Feed;
use crate::paywall::{self, Access};
use crate::proc::Documents;
use crate::store::{Feed as StoredFeed, Store};
use crate::{parse, proc, render};

const TICK: Duration = Duration::from_secs(60);
const MAX_BACKOFF_SECS: i64 = 6 * 3600;
/// 1 回の巡回で有料判定のために取りに行く記事数の上限。
/// 新着が大量にあっても、媒体のサイトを叩き続けないようにする。
const MAX_PAYWALL_LOOKUPS: usize = 20;
/// 有料判定の保持期間。配信から消えた記事の結果は使われない。
const PAYWALL_CACHE_DAYS: i64 = 30;
/// 1 回の巡回で取りに行く外部文書の数の上限。
const MAX_DOCUMENT_FETCHES: usize = 10;
/// 外部文書の保持期間。
const DOCUMENT_CACHE_DAYS: i64 = 7;

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
pub async fn refresh(
    store: &Store,
    client: &Client,
    feed: &StoredFeed,
) -> Result<(), RefreshError> {
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

async fn run(store: &Store, client: &Client, feed: &StoredFeed) -> Result<(), RefreshError> {
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

    let parsed = parse::parse(&bytes)?;
    let mut parsed = classify_items(store, client, parsed, MAX_PAYWALL_LOOKUPS).await;
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

    // Processor が必要とする外部文書をここで取りに行く。
    // Processor 自体は純粋なまま保ち、取得は呼び出し側の責務にする
    let docs = gather_documents(store, client, &chain, &parsed).await;
    let processed = chain
        .iter()
        .try_fold(parsed, |feed, p| p.apply(feed, &docs))?;

    store.set_output(feed.id, &render::to_rss2(&processed))?;
    if !title.is_empty() {
        store.set_title(feed.id, &title)?;
    }
    store.mark_success(feed.id, etag.as_deref(), last_modified.as_deref(), next)?;
    Ok(())
}

/// Processor が要求した外部文書を取得する。
///
/// URL 単位でキャッシュする。気象庁の XML のように URL が発表ごとに変わるものは
/// 一度取れば取り直す必要がない。1 回の巡回で取りに行く数には上限を置く。
async fn gather_documents(
    store: &Store,
    client: &Client,
    chain: &[Box<dyn crate::proc::Processor>],
    feed: &Feed,
) -> Documents {
    let mut docs = Documents::empty();
    let mut wanted: Vec<String> = chain.iter().flat_map(|p| p.wants(feed)).collect();
    wanted.sort();
    wanted.dedup();

    let mut fetches = 0;
    for url in wanted {
        match store.document_cached(&url) {
            Ok(Some(body)) => {
                docs.insert(url, body);
                continue;
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("document cache {url}: {e}");
                continue;
            }
        }
        if fetches >= MAX_DOCUMENT_FETCHES {
            // 残りは次の巡回で取る
            break;
        }
        fetches += 1;
        match fetch::fetch(client, &url, None, None).await {
            Ok(Fetched::Body { bytes, .. }) => {
                let body = String::from_utf8_lossy(&bytes).into_owned();
                if let Err(e) = store.remember_document(&url, &body) {
                    eprintln!("document cache {url}: {e}");
                }
                docs.insert(url, body);
            }
            Ok(Fetched::NotModified) => {}
            Err(e) => eprintln!("document {url}: {e}"),
        }
    }
    if fetches > 0 {
        let before = Utc::now().timestamp() - DOCUMENT_CACHE_DAYS * 24 * 3600;
        let _ = store.prune_documents(before);
    }
    docs
}

/// 各 item が有料記事かどうかを判定して埋める。
///
/// 判定は記事ページを 1 件ずつ取りに行くので、次の順で費用を抑える。
///
/// 1. ルールのある媒体の item だけを対象にする (それ以外は 0 リクエスト)
/// 2. 判定済みならキャッシュを使う
/// 3. 1 回の巡回で取りに行く数に上限を置く
///
/// 判定に失敗しても巡回自体は続ける。有料かどうかは配信の可否ではない。
pub async fn classify_items(
    store: &Store,
    client: &Client,
    mut feed: Feed,
    max_lookups: usize,
) -> Feed {
    let mut lookups = 0;
    for item in &mut feed.items {
        let Some(link) = item.link.clone() else {
            continue;
        };
        let Some(rule) = paywall::rule_for(&link) else {
            continue;
        };

        let access = match store.paywall_cached(&link) {
            Ok(Some(cached)) => cached,
            Ok(None) if lookups < max_lookups => {
                lookups += 1;
                let access = look_up(client, rule, &link).await;
                if let Err(e) = store.remember_paywall(&link, access) {
                    eprintln!("paywall cache {link}: {e}");
                }
                access
            }
            // 上限に達した。次の巡回で判定する
            Ok(None) => continue,
            Err(e) => {
                eprintln!("paywall cache {link}: {e}");
                continue;
            }
        };
        item.paywalled = access.as_flag();
    }
    lookups.gt(&0).then(|| {
        let before = Utc::now().timestamp() - PAYWALL_CACHE_DAYS * 24 * 3600;
        store.prune_paywall_cache(before).ok()
    });
    feed
}

async fn look_up(client: &Client, rule: &paywall::Rule, url: &str) -> Access {
    match fetch::fetch(client, url, None, None).await {
        Ok(Fetched::Body { bytes, .. }) => paywall::detect(rule, &String::from_utf8_lossy(&bytes)),
        Ok(Fetched::NotModified) => Access::Unknown,
        Err(e) => {
            eprintln!("paywall {url}: {e}");
            Access::Unknown
        }
    }
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
