//! 気象庁の防災情報 XML から、指定した市区町村の警報・注意報を取り出す。
//!
//! 気象庁のフィード (`extra.xml`) の entry は、それ自体には市区町村の情報を持たない。
//! リンク先の XML を取得して初めて分かる。そこで [`Processor::wants`] で必要な
//! XML を宣言し、取得結果を受け取って description に展開する。
//!
//! 取りに行く数を抑えるための絞り込みが 2 段ある。
//!
//! 1. **URL に府県予報区コードが入っている** (`..._VPWW53_280000.xml` の 280000)。
//!    市区町村名から必要なコードを引き、一致する entry だけを対象にする。ここは通信しない
//! 2. **予報区ごとに最新の 1 件だけ**を取る。古い発表は上書きされている
//!
//! 前回からの差分は自分で保存しない。XML の `Kind` が `Status` を持っており、
//! 今回新しく出たもの (`発表`) と継続中のもの (`継続`) を気象庁が区別している。
//!
//! 文書型は `VPWW53` (気象特別警報・警報・注意報) のみを使う。`VPWW54` は旧形式の
//! 重複で、`VPWW55`/`56`/`58`/`59` は同じ事象をレベル表記で分割したもの。
//! いずれも追加の情報を持たない。

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::model::{Feed, Item};
use crate::proc::{Documents, Processor, ProcessorError};

/// 総合版の情報種別コード。
const DOCUMENT_TYPE: &str = "VPWW53";
/// 市区町村単位の警報・注意報が入るブロック。
const MUNICIPALITY_BLOCK: &str = "気象警報・注意報（市町村等）";
/// 今回新しく発表された種別の Status。継続中のものは「継続」になる。
const STATUS_NEW: &str = "発表";
/// 人が読める警報ページ。フィードの link は XML を指しており、リーダーから
/// 開いても読めないため差し替える。
const WARNING_PAGE: &str = "https://www.jma.go.jp/bosai/warning/#area_type=offices&area_code=";
/// 地域の区切り。Slack の /feed は改行を潰すため、1 行になっても切れ目が分かるようにする。
const AREA_SEPARATOR: &str = "／";

fn default_new_prefix() -> String {
    "【新】".into()
}

/// 市区町村名 → 府県予報区コード。気象庁の area.json から生成した表。
static AREAS: LazyLock<Vec<(&'static str, &'static str)>> = LazyLock::new(|| {
    include_str!("jma_areas.csv")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| l.split_once(','))
        .collect()
});

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct JmaWarning {
    /// 対象の市区町村名。前方一致するので「神戸市」で 9 区すべてを拾える
    pub areas: Vec<String>,
    /// 残す種別。部分一致。空なら全部。「警報」は「特別警報」も拾う
    pub kinds: Vec<String>,
    /// 今回新しく発表された種別の前に付ける。空にすれば印を付けない
    pub new_prefix: String,
    /// 同じく後ろに付ける。読み手に合わせて変える (Slack の太字なら両方 `*`)
    pub new_suffix: String,
}

impl Default for JmaWarning {
    fn default() -> Self {
        Self {
            areas: Vec::new(),
            kinds: Vec::new(),
            new_prefix: default_new_prefix(),
            new_suffix: String::new(),
        }
    }
}

impl Processor for JmaWarning {
    fn name(&self) -> &'static str {
        "jma_warning"
    }

    fn params(&self) -> String {
        serde_json::to_string(self).expect("jma_warning のパラメータを直列化できない")
    }

    fn wants(&self, feed: &Feed) -> Vec<String> {
        let codes = self.office_codes();
        if codes.is_empty() {
            return Vec::new();
        }
        // 予報区ごとに最新の 1 件だけ。古い発表は上書きされている
        let mut latest: HashMap<&str, (&str, i64)> = HashMap::new();
        for item in &feed.items {
            let Some(link) = item.link.as_deref() else {
                continue;
            };
            let Some(code) = office_code_of(link) else {
                continue;
            };
            if !codes.contains(&code) {
                continue;
            }
            let at = item.published.map(|t| t.timestamp()).unwrap_or(0);
            let entry = latest.entry(code).or_insert((link, at));
            if at > entry.1 {
                *entry = (link, at);
            }
        }
        let mut urls: Vec<String> = latest.values().map(|(u, _)| (*u).to_string()).collect();
        urls.sort();
        urls
    }

    fn apply(&self, mut feed: Feed, docs: &Documents) -> Result<Feed, ProcessorError> {
        let wanted = self.wants(&feed);
        feed.items.retain_mut(|item| {
            let Some(link) = item.link.clone() else {
                return false;
            };
            // wants で選ばれなかった entry (旧形式・他県・古い発表) は落とす
            if !wanted.contains(&link) {
                return false;
            }
            // まだ取得できていない文書は、次の巡回まで出さない
            let Some(xml) = docs.get(&link) else {
                return false;
            };
            self.rewrite(item, xml)
        });
        Ok(feed)
    }
}

impl JmaWarning {
    /// 指定された市区町村を含む府県予報区コード。
    fn office_codes(&self) -> Vec<&'static str> {
        let mut codes: Vec<&'static str> = AREAS
            .iter()
            .filter(|(name, _)| self.areas.iter().any(|a| name.starts_with(a.as_str())))
            .map(|(_, code)| *code)
            .collect();
        codes.sort_unstable();
        codes.dedup();
        codes
    }

    /// 該当があれば item を書き換えて `true`。無ければ `false` で item ごと落とす。
    fn rewrite(&self, item: &mut Item, xml: &str) -> bool {
        let hits = self.warnings_in(xml);
        if hits.is_empty() {
            return false;
        }

        let areas = hits
            .iter()
            .map(|(area, kinds)| format!("{area}: {}", kinds.join(", ")))
            .collect::<Vec<_>>()
            .join(&format!("\n{AREA_SEPARATOR}"));

        // いつ時点の情報かが分からないと、警戒すべきかを判断できない
        item.description = Some(match report_time(xml) {
            Some(at) => format!("{at} 時点\n{areas}"),
            None => areas,
        });

        // フィードの link は XML を指している。人が読めるページに差し替える
        if let Some(code) = item.link.as_deref().and_then(office_code_of) {
            item.link = Some(format!("{WARNING_PAGE}{code}"));
        }
        if let Some(head) = headline_of(xml) {
            item.title = Some(head);
        }
        true
    }

    fn warnings_in(&self, xml: &str) -> Vec<(String, Vec<String>)> {
        parse_municipality_warnings(xml)
            .into_iter()
            .filter(|(area, _)| self.areas.iter().any(|a| area.starts_with(a.as_str())))
            .filter_map(|(area, kinds)| {
                let kinds: Vec<String> = kinds
                    .into_iter()
                    .filter(|k| {
                        self.kinds.is_empty() || self.kinds.iter().any(|want| k.name.contains(want))
                    })
                    .map(|k| self.label(&k))
                    .collect();
                (!kinds.is_empty()).then_some((area, kinds))
            })
            .collect()
    }

    /// 新しく発表された種別には印を付ける。継続中のものはそのまま。
    fn label(&self, kind: &Kind) -> String {
        if kind.status == STATUS_NEW {
            format!("{}{}{}", self.new_prefix, kind.name, self.new_suffix)
        } else {
            kind.name.clone()
        }
    }
}

/// `..._VPWW53_280000.xml` から府県予報区コードを取り出す。
/// 総合版以外は対象にしない。
fn office_code_of(url: &str) -> Option<&str> {
    let name = url.rsplit('/').next()?;
    let rest = name.strip_suffix(".xml")?;
    let (head, code) = rest.rsplit_once('_')?;
    head.ends_with(DOCUMENT_TYPE).then_some(code)
}

/// 発表時刻。XML の Head/ReportDateTime を日本時間で表示する。
fn report_time(xml: &str) -> Option<String> {
    let raw = first_text_of(xml, "ReportDateTime")?;
    let at = chrono::DateTime::parse_from_rfc3339(&raw).ok()?;
    let jst = chrono::FixedOffset::east_opt(9 * 3600)?;
    Some(at.with_timezone(&jst).format("%Y-%m-%d %H:%M").to_string())
}

/// 指定した要素の最初のテキスト。
fn first_text_of(xml: &str, tag: &str) -> Option<String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut inside = false;
    let mut text = String::new();
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) => {
                inside = e.local_name().as_ref() == tag;
                text.clear();
            }
            Ok(quick_xml::events::Event::Text(t)) if inside => text.push_str(t.as_ref()),
            Ok(quick_xml::events::Event::End(e)) => {
                if e.local_name().as_ref() == tag && !text.trim().is_empty() {
                    return Some(text.trim().to_string());
                }
                inside = false;
                text.clear();
            }
            Ok(quick_xml::events::Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
}

/// 見出しに使う Title を取り出す。
///
/// XML には Title が 2 つある。Control の Title は種別名 (「気象特別警報・警報・注意報」)、
/// Head の Title は府県名入り (「兵庫県気象警報・注意報」)。後者を使う。
fn headline_of(xml: &str) -> Option<String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut in_title = false;
    let mut titles: Vec<String> = Vec::new();
    let mut text = String::new();

    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) => {
                in_title = e.local_name().as_ref() == "Title";
                text.clear();
            }
            Ok(quick_xml::events::Event::Text(t)) if in_title => text.push_str(t.as_ref()),
            Ok(quick_xml::events::Event::End(e)) => {
                if e.local_name().as_ref() == "Title" {
                    let value = text.trim().to_string();
                    if !value.is_empty() {
                        titles.push(value);
                    }
                }
                in_title = false;
                text.clear();
            }
            Ok(quick_xml::events::Event::Eof) | Err(_) => break,
            _ => {}
        }
        if titles.len() >= 2 {
            break;
        }
    }
    titles.pop()
}

/// 種別と、その発表状況。
pub struct Kind {
    pub name: String,
    /// 「発表」なら今回新しく出たもの、「継続」なら前回から続いているもの
    pub status: String,
}

/// 市町村等ブロックから (地域名, 種別) を取り出す。
fn parse_municipality_warnings(xml: &str) -> Vec<(String, Vec<Kind>)> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut out: Vec<(String, Vec<Kind>)> = Vec::new();

    let mut in_block = false;
    let mut path: Vec<String> = Vec::new();
    let mut area: Option<String> = None;
    let mut kinds: Vec<Kind> = Vec::new();
    let mut kind_name: Option<String> = None;
    let mut kind_status = String::new();
    let mut text = String::new();

    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) => {
                let name = e.local_name().as_ref().to_string();
                if name == "Warning" {
                    in_block = e
                        .attributes()
                        .flatten()
                        .any(|a| a.value.as_ref() == MUNICIPALITY_BLOCK);
                }
                if in_block && name == "Item" {
                    area = None;
                    kinds.clear();
                }
                if in_block && name == "Kind" {
                    kind_name = None;
                    kind_status.clear();
                }
                path.push(name);
                text.clear();
            }
            Ok(quick_xml::events::Event::Text(t)) => text.push_str(t.as_ref()),
            Ok(quick_xml::events::Event::End(e)) => {
                let name = e.local_name().as_ref().to_string();
                let value = text.trim().to_string();
                let parent = path.get(path.len().wrapping_sub(2)).map(String::as_str);
                if in_block && !value.is_empty() {
                    match (name.as_str(), parent) {
                        ("Name", Some("Area")) if area.is_none() => area = Some(value),
                        ("Name", Some("Kind")) if value != "解除" => kind_name = Some(value),
                        ("Status", Some("Kind")) => kind_status = value,
                        _ => {}
                    }
                }
                if in_block
                    && name == "Kind"
                    && let Some(n) = kind_name.take()
                {
                    kinds.push(Kind {
                        name: n,
                        status: std::mem::take(&mut kind_status),
                    });
                }
                if in_block && name == "Item" {
                    if let (Some(a), false) = (area.take(), kinds.is_empty()) {
                        out.push((a, std::mem::take(&mut kinds)));
                    }
                    kinds.clear();
                }
                if name == "Warning" {
                    in_block = false;
                }
                path.pop();
                text.clear();
            }
            Ok(quick_xml::events::Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}
