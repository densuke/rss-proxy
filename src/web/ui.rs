//! 管理画面。フォーム POST だけで完結させ、JavaScript もテンプレートエンジンも使わない。
//!
//! 上流フィード由来の文字列 (title / last_error) がこの画面に載るため、
//! 出力時に必ず HTML エスケープする。

use axum::extract::{Form, Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use chrono::{DateTime, FixedOffset, Local, Offset};
use serde::Deserialize;

use crate::html::to_plain_text;
use crate::proc;
use crate::store::{Feed, NewFeed, ProcessorSpec, Store};
use crate::web::SharedStore;

/// UNIX 時刻を指定オフセットで表示する。
///
/// 上流フィードの時刻表記は GMT / Z / +0900 と揃っておらず、内部では UTC に
/// 正規化している。画面には運用者のローカル時刻で出す。
/// どのオフセットで表示しているかは [`offset_label`] を見出しに添えて示す。
pub fn format_time(unix: i64, offset: FixedOffset) -> String {
    DateTime::from_timestamp(unix, 0)
        .map(|t| {
            t.with_timezone(&offset)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "-".into())
}

/// 見出しに添えるオフセット表記。値ごとに繰り返さず、ここで 1 度だけ示す。
pub fn offset_label(offset: FixedOffset) -> String {
    offset.to_string()
}

// ponytail: 現在時刻のオフセットを全行に使う。夏時間のある地域では、
// 切り替えを挟んだ過去の時刻がずれる。表示するのは直近の取得時刻だけなので実害はない。
fn local_offset() -> FixedOffset {
    Local::now().offset().fix()
}

/// 信頼できない文字列を HTML に埋め込む前に必ず通す。
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

const STYLE: &str = "<style>
body{font-family:system-ui,sans-serif;margin:2rem auto;max-width:60rem;line-height:1.6}
table{border-collapse:collapse;width:100%}th,td{border:1px solid #ccc;padding:.4rem;text-align:left}
textarea{width:100%;font-family:ui-monospace,monospace}
.err{color:#b00}form{display:inline}
ol{padding-left:1.5rem}li{margin:.4rem 0}small{color:#666}
</style>";

fn page(title: &str, body: &str) -> Html<String> {
    Html(format!(
        "<!doctype html><html lang=\"ja\"><head><meta charset=\"utf-8\">\
         <title>{}</title>{STYLE}</head><body>{body}</body></html>",
        escape(title)
    ))
}

fn with<T>(store: &SharedStore, f: impl FnOnce(&Store) -> T) -> T {
    f(&store.lock().expect("store lock"))
}

fn server_error(e: impl std::fmt::Display) -> Response {
    eprintln!("ui: {e}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

pub async fn index(State(store): State<SharedStore>) -> Response {
    let feeds = match with(&store, |s| s.list_feeds()) {
        Ok(feeds) => feeds,
        Err(e) => return server_error(e),
    };

    let offset = local_offset();
    let rows = feeds
        .iter()
        .map(|f| {
            format!(
                "<tr><td><a href=\"/ui/feeds/{name}\">{name}</a></td><td>{title}</td>\
                 <td>{interval}s</td><td>{last}</td><td>{state}</td><td>{delivery}</td></tr>",
                name = escape(&f.name),
                title = escape(f.title.as_deref().unwrap_or("-")),
                interval = f.interval_secs,
                last = last_fetch(f, offset),
                state = status(f),
                delivery = delivery(f),
            )
        })
        .collect::<String>();

    page(
        "rss-proxy",
        &format!(
            "<h1>フィード</h1>\
             <table><tr><th>名前</th><th>タイトル</th><th>間隔</th>\
             <th>最終取得 ({offset})</th><th>状態</th><th>配信</th></tr>\
             {rows}</table>\
             <h2>登録</h2>\
             <form method=\"post\" action=\"/ui/feeds\">\
             <p>名前 <input name=\"name\" required pattern=\"[A-Za-z0-9_-]+\"></p>\
             <p>URL <input name=\"url\" type=\"url\" size=\"60\" required></p>\
             <p>間隔(秒) <input name=\"interval\" type=\"number\" value=\"900\" min=\"60\"></p>\
             <p><button>追加</button></p></form>",
            offset = offset_label(offset),
        ),
    )
    .into_response()
}

fn last_fetch(f: &Feed, offset: FixedOffset) -> String {
    f.last_success_at
        .map(|t| format_time(t, offset))
        .unwrap_or_else(|| "-".into())
}

/// 一度も取得できていないフィードの配信 URL は 404 になる。リンクにしない。
fn delivery(f: &Feed) -> String {
    if f.has_output {
        format!(
            "<a href=\"/feeds/{name}\">/feeds/{name}</a>",
            name = escape(&f.name)
        )
    } else {
        "-".into()
    }
}

fn status(f: &Feed) -> String {
    match (&f.last_error, f.last_success_at) {
        (Some(e), _) => format!(
            "<span class=\"err\">失敗{}回: {}</span>",
            f.fail_count,
            escape(e)
        ),
        (None, Some(_)) => "正常".into(),
        (None, None) => "未取得".into(),
    }
}

pub async fn show(State(store): State<SharedStore>, Path(name): Path<String>) -> Response {
    let found = with(&store, |s| {
        let Some(feed) = s.feed_by_name(&name)? else {
            return Ok(None);
        };
        let chain = s.processors(feed.id)?;
        let output = s.output(&name)?;
        Ok::<_, rusqlite::Error>(Some((feed, chain, output)))
    });

    let (feed, chain, output) = match found {
        Ok(Some(v)) => v,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return server_error(e),
    };

    let text = chain
        .iter()
        .map(|(kind, params)| format!("{kind} {params}\n"))
        .collect::<String>();

    page(
        &feed.name,
        &format!(
            "<p><a href=\"/\">← 一覧</a></p><h1>{name}</h1>\
             <p>{url}</p><p>最終取得 ({offset}): {last} / 状態: {state}</p>\
             <h2>Processor 連鎖</h2>\
             <p>1 行に 1 つ、「種別 パラメータ(JSON)」の形式で書く。並び順が適用順。<br>\
             利用可能: {kinds}</p>\
             <form method=\"post\" action=\"/ui/feeds/{name}/processors\">\
             <textarea name=\"chain\" rows=\"6\">{text}</textarea>\
             <p><button>保存</button></p></form>\
             <h2>配信中の内容</h2>{items}\
             <h2>操作</h2>\
             <form method=\"post\" action=\"/ui/feeds/{name}/fetch\"><button>今すぐ取得</button></form>\
             <form method=\"post\" action=\"/ui/feeds/{name}/delete\"><button>削除</button></form>",
            name = escape(&feed.name),
            url = escape(&feed.url),
            offset = offset_label(local_offset()),
            last = last_fetch(&feed, local_offset()),
            state = status(&feed),
            kinds = crate::cli::KINDS.join(", "),
            text = escape(&text),
            items = served_items(output.as_deref()),
        ),
    )
    .into_response()
}

/// 配信中の XML を解析し、書かれている順に item を並べる。
/// Processor を付け替えた結果、実際に何が配信されるのかを確認するための表示。
fn served_items(xml: Option<&str>) -> String {
    let Some(xml) = xml else {
        return "<p>まだ取得されていません。「今すぐ取得」を実行してください。</p>".into();
    };
    let feed = match crate::parse::parse(xml.as_bytes()) {
        Ok(feed) => feed,
        Err(e) => {
            return format!(
                "<p class=\"err\">配信内容を解析できません: {}</p>",
                escape(&e.to_string())
            );
        }
    };

    let rows = feed
        .items
        .iter()
        .map(|item| {
            let title = escape(item.title.as_deref().unwrap_or("(タイトルなし)"));
            let title = match &item.link {
                Some(link) => format!("<a href=\"{}\">{title}</a>", escape(link)),
                None => title,
            };
            let excerpt = item
                .description
                .as_deref()
                .map(excerpt)
                .filter(|e| !e.is_empty())
                .map(|e| format!("<br><small>{}</small>", escape(&e)))
                .unwrap_or_default();
            format!("<li>{title}{excerpt}</li>")
        })
        .collect::<String>();

    format!("<p>{} 件</p><ol>{rows}</ol>", feed.items.len())
}

/// HTML を落とし、先頭だけを取り出す。
fn excerpt(html: &str) -> String {
    let text = to_plain_text(html);
    match text.char_indices().nth(120) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

#[derive(Deserialize)]
pub struct AddForm {
    name: String,
    url: String,
    interval: i64,
}

pub async fn add(State(store): State<SharedStore>, Form(form): Form<AddForm>) -> Response {
    let added = with(&store, |s| {
        s.add_feed(&NewFeed {
            name: form.name,
            url: form.url,
            interval_secs: form.interval,
        })
    });
    match added {
        Ok(_) => Redirect::to("/").into_response(),
        // 名前の重複など。入力の問題として扱う
        Err(e) => (StatusCode::BAD_REQUEST, format!("登録できません: {e}")).into_response(),
    }
}

pub async fn delete(State(store): State<SharedStore>, Path(name): Path<String>) -> Response {
    match with(&store, |s| s.remove_feed(&name)) {
        Ok(true) => Redirect::to("/").into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => server_error(e),
    }
}

#[derive(Deserialize)]
pub struct ChainForm {
    chain: String,
}

pub async fn set_chain(
    State(store): State<SharedStore>,
    Path(name): Path<String>,
    Form(form): Form<ChainForm>,
) -> Response {
    let chain = match parse_chain(&form.chain) {
        Ok(chain) => chain,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };

    let saved = with(&store, |s| {
        let Some(feed) = s.feed_by_name(&name)? else {
            return Ok(false);
        };
        s.set_processors(feed.id, &chain)?;
        Ok::<_, rusqlite::Error>(true)
    });
    match saved {
        Ok(true) => Redirect::to(&format!("/ui/feeds/{name}")).into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => server_error(e),
    }
}

/// 「種別 パラメータ」形式の各行を Processor 連鎖にする。
/// 保存前にすべて組み立てて検証し、壊れた設定を DB に残さない。
fn parse_chain(text: &str) -> Result<Vec<ProcessorSpec>, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (kind, params) = line.split_once(char::is_whitespace).unwrap_or((line, "{}"));
            let params = params.trim();
            let params = if params.is_empty() { "{}" } else { params };
            proc::build(kind, params).map_err(|e| e.to_string())?;
            Ok((kind.to_string(), params.to_string()))
        })
        .collect()
}

/// 次回取得時刻を過去にして、次の tick で拾わせる。
pub async fn fetch_now(State(store): State<SharedStore>, Path(name): Path<String>) -> Response {
    let marked = with(&store, |s| {
        let Some(feed) = s.feed_by_name(&name)? else {
            return Ok(false);
        };
        s.set_next_fetch_at(feed.id, 0)?;
        Ok::<_, rusqlite::Error>(true)
    });
    match marked {
        Ok(true) => Redirect::to(&format!("/ui/feeds/{name}")).into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => server_error(e),
    }
}
