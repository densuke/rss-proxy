//! 記事が有料かどうかの判定。
//!
//! 判定の目印は媒体ごとに違う。共通規格の schema.org `isAccessibleForFree` を
//! 出す媒体もあれば、独自の値しか持たない媒体もある。ここにルールを 1 か所に
//! まとめ、媒体が増えたら足していく。
//!
//! 単純な文字列一致は使えない。記事ページには関連記事の一覧が載るため、
//! 「有料」を示す語や class 名は無料記事のページにも現れる。判定に使うのは
//! その記事自身を指す構造化データだけにする。

/// 記事の閲覧可否。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Paid,
    Free,
    /// 目印がなく判断できない。無料と決めつけない
    Unknown,
}

impl Access {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Paid => "paid",
            Self::Free => "free",
            Self::Unknown => "unknown",
        }
    }

    /// キャッシュに保存した文字列から戻す。未知の値は Unknown 扱い。
    pub fn parse(s: &str) -> Self {
        match s {
            "paid" => Self::Paid,
            "free" => Self::Free,
            _ => Self::Unknown,
        }
    }

    /// Processor が使う形。判定できなかったものは None。
    pub fn as_flag(self) -> Option<bool> {
        match self {
            Self::Paid => Some(true),
            Self::Free => Some(false),
            Self::Unknown => None,
        }
    }
}

pub struct Rule {
    /// この登録ドメインとその下位ドメインに適用する
    pub host: &'static str,
    detect: fn(&str) -> Access,
}

/// 読売新聞。schema.org の isAccessibleForFree を出す。
/// 一覧ページの鍵アイコンと一致することを実測で確認した。
fn yomiuri(page: &str) -> Access {
    match find_json_bool(page, "isAccessibleForFree") {
        Some(true) => Access::Free,
        Some(false) => Access::Paid,
        None => Access::Unknown,
    }
}

/// 日本経済新聞。paywallProps の isLockedArticle が本文の切り詰めを示す。
/// isPaidUserOnlyArticle は「完全会員限定」だけを指し、従量型では false になる。
fn nikkei(page: &str) -> Access {
    match find_json_bool(page, "isLockedArticle") {
        Some(true) => Access::Paid,
        Some(false) => Access::Free,
        None => Access::Unknown,
    }
}

const RULES: &[Rule] = &[
    Rule {
        host: "yomiuri.co.jp",
        detect: yomiuri,
    },
    Rule {
        host: "nikkei.com",
        detect: nikkei,
    },
];

/// この URL を判定できるルール。無ければ取りに行かない。
pub fn rule_for(url: &str) -> Option<&'static Rule> {
    let host = host_of(url)?;
    RULES
        .iter()
        .find(|r| host == r.host || host.ends_with(&format!(".{}", r.host)))
}

pub fn detect(rule: &Rule, page: &str) -> Access {
    (rule.detect)(page)
}

fn host_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// JSON の `"key": true` / `"key":"false"` を拾う。真偽値でも文字列でも受ける。
fn find_json_bool(page: &str, key: &str) -> Option<bool> {
    let needle = format!("\"{key}\"");
    let at = page.find(&needle)? + needle.len();
    let rest = page[at..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start().trim_start_matches('"');
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}
