//! フィード本文の HTML を扱う小さなヘルパー。
//!
//! description は HTML を含み、そこには実体参照も混ざる。タグを落として
//! 比較したり抜粋を作ったりする場面が複数あるため、ここに集約する。

use std::sync::LazyLock;

use regex::Regex;

static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// タグと実体参照を落とし、空白を畳んだプレーンテキストにする。
pub fn to_plain_text(html: &str) -> String {
    let text = TAG.replace_all(html, " ");
    let text = text
        .replace("&nbsp;", " ")
        .replace(['\u{a0}', '\u{3000}'], " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        // &amp; は最後に置く。先に戻すと "&amp;lt;" が "<" になってしまう
        .replace("&amp;", "&");
    SPACES.replace_all(&text, " ").trim().to_string()
}
