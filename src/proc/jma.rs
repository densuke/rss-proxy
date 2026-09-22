//! 気象庁の防災情報 XML から、指定した市区町村の警報・注意報を取り出す。
//!
//! 気象庁のフィード (`extra.xml`) の entry は、それ自体には市区町村の情報を持たない。
//! リンク先の XML を取得して初めて分かる。そこで [`Processor::wants`] で必要な
//! XML を宣言し、取得結果を受け取って description に展開する。
//!
//! 1. **URL に府県予報区コードが入っている** (`..._VPWW53_280000.xml` の 280000)。
//!    市区町村名から必要なコードを引き、一致する entry だけを取りに行く。ここは通信しない
//! 2. **配信するのは予報区ごとに最新の 1 件だけ**。古い発表は上書きされている。
//!    過去の発表も取得するが、発令時刻を辿るためだけに使う
//!
//! 前回からの差分は自分で保存しない。XML の `Kind` が `Status` を持っており、
//! 今回新しく出たもの (`発表`) と継続中のもの (`継続`) を気象庁が区別している。
//!
//! 継続中の種別の発令時刻は XML に無い。フィードに残っている過去の発表を新しい順に
//! 辿り、`継続` でなくなった発表の時刻を発令時刻とする。辿れるのはフィードに
//! 残っている範囲だけなので、長期フィード (`extra_l.xml`) のほうが遡れる。
//!
//! 文書型は `VPWW53` (気象特別警報・警報・注意報) のみを使う。`VPWW54` は旧形式の
//! 重複で、`VPWW55`/`56`/`58`/`59` は同じ事象をレベル表記で分割したもの。
//! いずれも追加の情報を持たない。

use std::collections::BTreeMap;
use std::sync::LazyLock;

use chrono::{DateTime, FixedOffset};

use crate::model::{Feed, Item};
use crate::proc::{Documents, Processor, ProcessorError};

/// 総合版の情報種別コード。
const DOCUMENT_TYPE: &str = "VPWW53";
/// 市区町村単位の警報・注意報が入るブロック。
const MUNICIPALITY_BLOCK: &str = "気象警報・注意報（市町村等）";
/// 今回新しく発表された種別の Status。継続中のものは「継続」になる。
const STATUS_NEW: &str = "発表";
/// 前回の発表から続いている種別の Status。
const STATUS_CONTINUING: &str = "継続";
/// 人が読める警報ページ。フィードの link は XML を指しており、リーダーから
/// 開いても読めないため差し替える。
const WARNING_PAGE: &str = "https://www.jma.go.jp/bosai/warning/#area_type=offices&area_code=";
/// 地域の行頭に付ける。全行に付けることで行頭が揃い、改行が潰れる読み手でも
/// 切れ目が分かる。
const AREA_BULLET: &str = "・";

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

    /// 各予報区の最新を先に、過去の発表をその後に並べる。取得数の上限で
    /// 打ち切られても、配信に要る最新が先に揃う。
    fn wants(&self, feed: &Feed) -> Vec<String> {
        let history = self.history(feed);
        let latest = history.iter().filter_map(|h| h.first());
        let past = history.iter().flat_map(|h| h.iter().skip(1));
        latest.chain(past).cloned().collect()
    }

    fn apply(&self, mut feed: Feed, docs: &Documents) -> Result<Feed, ProcessorError> {
        let history = self.history(&feed);
        feed.items.retain_mut(|item| {
            let Some(link) = item.link.clone() else {
                return false;
            };
            // 予報区ごとの最新だけを残す。旧形式・他県・古い発表は落とす
            let Some(chain) = history.iter().find(|h| h.first() == Some(&link)) else {
                return false;
            };
            // まだ取得できていない文書は、次の巡回まで出さない
            let Some(xml) = docs.get(&link) else {
                return false;
            };
            // 間に未取得の文書があれば、そこで遡るのをやめる
            let past: Vec<&str> = chain[1..].iter().map_while(|u| docs.get(u)).collect();
            self.rewrite(item, xml, &past)
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

    /// 対象の予報区ごとの総合版の URL。それぞれ新しい順。
    fn history(&self, feed: &Feed) -> Vec<Vec<String>> {
        let codes = self.office_codes();
        let mut by_office: BTreeMap<&str, Vec<(i64, &str)>> = BTreeMap::new();
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
            by_office.entry(code).or_default().push((at, link));
        }
        by_office
            .into_values()
            .map(|mut v| {
                v.sort_unstable_by(|a, b| b.cmp(a));
                v.into_iter().map(|(_, u)| u.to_string()).collect()
            })
            .collect()
    }

    /// 該当があれば item を書き換えて `true`。無ければ `false` で item ごと落とす。
    /// `past` は同じ予報区の過去の発表で、新しい順。
    fn rewrite(&self, item: &mut Item, xml: &str, past: &[&str]) -> bool {
        let report = report_at(xml);
        // ponytail: 辿る前に全件を解析している。巡回が遅くなったら必要な分だけ解析する
        let past: Vec<Snapshot> = past
            .iter()
            .map(|x| (report_at(x), parse_municipality_warnings(x)))
            .collect();
        let hits = self.warnings_in(xml, report, &past);
        if hits.is_empty() {
            return false;
        }

        let areas = hits
            .iter()
            .map(|(area, kinds)| format!("{AREA_BULLET}{area}: {}", kinds.join(", ")))
            .collect::<Vec<_>>()
            .join("\n");

        // いつ時点の情報かが分からないと、警戒すべきかを判断できない
        item.description = Some(match report {
            Some(at) => format!("{} 時点\n{areas}", at.format("%Y-%m-%d %H:%M")),
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

    fn warnings_in(
        &self,
        xml: &str,
        report: Option<DateTime<FixedOffset>>,
        past: &[Snapshot],
    ) -> Vec<(String, Vec<String>)> {
        parse_municipality_warnings(xml)
            .into_iter()
            .filter(|(area, _)| self.areas.iter().any(|a| area.starts_with(a.as_str())))
            .filter_map(|(area, kinds)| {
                let kinds: Vec<String> = kinds
                    .into_iter()
                    .filter(|k| {
                        self.kinds.is_empty() || self.kinds.iter().any(|want| k.name.contains(want))
                    })
                    .map(|k| {
                        let since = (k.status == STATUS_CONTINUING)
                            .then(|| issued_at(&area, &k.name, past))
                            .flatten()
                            .map(|at| since_text(at, report));
                        self.label(&k, since)
                    })
                    .collect();
                (!kinds.is_empty()).then_some((area, kinds))
            })
            .collect()
    }

    /// 新しく発表された種別には印を付ける。継続中のものは発令時刻が分かれば添える。
    fn label(&self, kind: &Kind, since: Option<String>) -> String {
        match since {
            _ if kind.status == STATUS_NEW => {
                format!("{}{}{}", self.new_prefix, kind.name, self.new_suffix)
            }
            Some(since) => format!("{}({since}〜)", kind.name),
            None => kind.name.clone(),
        }
    }
}

/// 過去の 1 回分の発表。発表時刻と、市町村等ブロックの中身。
type Snapshot = (Option<DateTime<FixedOffset>>, Vec<(String, Vec<Kind>)>);

/// 継続中の種別が発令された時刻。過去の発表を新しい順に辿り、`継続` でなくなった
/// 発表の時刻を返す。途中で種別が消えていたり、辿り尽くしたりしたら分からない。
fn issued_at(area: &str, kind: &str, past: &[Snapshot]) -> Option<DateTime<FixedOffset>> {
    for (at, warnings) in past {
        let (_, kinds) = warnings.iter().find(|(a, _)| a == area)?;
        let status = &kinds.iter().find(|k| k.name == kind)?.status;
        if status != STATUS_CONTINUING {
            return *at;
        }
    }
    None
}

/// 発令時刻の表示。発表と同じ日なら時刻だけにする。
fn since_text(at: DateTime<FixedOffset>, report: Option<DateTime<FixedOffset>>) -> String {
    let same_day = report.is_some_and(|r| r.date_naive() == at.date_naive());
    let format = if same_day { "%H:%M" } else { "%-m/%-d %H:%M" };
    at.format(format).to_string()
}

/// `..._VPWW53_280000.xml` から府県予報区コードを取り出す。
/// 総合版以外は対象にしない。
fn office_code_of(url: &str) -> Option<&str> {
    let name = url.rsplit('/').next()?;
    let rest = name.strip_suffix(".xml")?;
    let (head, code) = rest.rsplit_once('_')?;
    head.ends_with(DOCUMENT_TYPE).then_some(code)
}

/// 発表時刻。XML の Head/ReportDateTime を日本時間にする。
fn report_at(xml: &str) -> Option<DateTime<FixedOffset>> {
    let raw = first_text_of(xml, "ReportDateTime")?;
    let at = DateTime::parse_from_rfc3339(&raw).ok()?;
    Some(at.with_timezone(&FixedOffset::east_opt(9 * 3600)?))
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
