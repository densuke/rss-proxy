//! Google ニュースのフィード固有の処理。
//!
//! Google ニュース (generator: NFE/5.0) の item は description に
//! 「関連記事リスト」を持ち、その先頭要素が title と重複する。
//!
//! - 単一形式:   `<a>見出し</a>&nbsp;&nbsp;<font>媒体名</font>`
//! - クラスタ形式: `<ol><li>(単一形式と同じ構造)</li>...</ol>`
//!
//! いずれも先頭要素が title と同じ内容なので、それだけを取り除く。

use std::sync::LazyLock;

use regex::Regex;

use crate::html::to_plain_text;
use crate::model::Feed;
use crate::proc::{Processor, ProcessorError};

// ponytail: 正規表現による抽出。対象は NFE/5.0 が機械生成する定型 HTML に限られ、
// li の入れ子も属性中の '>' も現れない。崩れたら html5ever ベースに置き換える。
static LI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<li>(.*?)</li>").unwrap());
static ANCHOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<a[^>]*>(.*?)</a>").unwrap());
static FONT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<font[^>]*>(.*?)</font>").unwrap());

pub struct GoogleNewsCluster;

impl Processor for GoogleNewsCluster {
    fn name(&self) -> &'static str {
        "google_news_cluster"
    }

    fn apply(&self, mut feed: Feed) -> Result<Feed, ProcessorError> {
        for item in &mut feed.items {
            let (Some(title), Some(description)) = (&item.title, &item.description) else {
                continue;
            };
            if let Some(rewritten) = strip_duplicate_head(title, description) {
                item.description = Some(rewritten);
            }
        }
        Ok(feed)
    }
}

/// title と重複する先頭要素を取り除いた description を返す。
/// 対象の構造でない場合や取り除くものがない場合は `None`。
fn strip_duplicate_head(title: &str, description: &str) -> Option<String> {
    let entries: Vec<&str> = if description.contains("<li>") {
        LI.captures_iter(description)
            .map(|c| c.get(0).unwrap().as_str())
            .collect()
    } else {
        vec![description]
    };
    let head = entries.first()?;

    let head_text = to_plain_text(&extract(&ANCHOR, head)?);
    let publisher = extract(&FONT, head).map(|p| to_plain_text(&p));
    if head_text != normalize_title(title, publisher.as_deref()) {
        return None;
    }

    let rest = &entries[1..];
    if rest.is_empty() {
        return Some(String::new());
    }
    Some(format!("<ol>{}</ol>", rest.concat()))
}

fn extract(re: &Regex, html: &str) -> Option<String> {
    re.captures(html).map(|c| c[1].to_string())
}

/// title 末尾の " - 媒体名" を落としたうえで正規化する。
fn normalize_title(title: &str, publisher: Option<&str>) -> String {
    let normalized = to_plain_text(title);
    match publisher {
        Some(p) if !p.is_empty() => normalized
            .strip_suffix(&format!(" - {p}"))
            .unwrap_or(&normalized)
            .to_string(),
        _ => normalized,
    }
}
