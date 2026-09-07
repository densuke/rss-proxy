pub mod dedupe;
pub mod exclude;
pub mod vendor;

use crate::model::Feed;
use crate::proc::dedupe::Dedupe;
use crate::proc::exclude::Exclude;
use crate::proc::vendor::google_news::GoogleNewsCluster;

#[derive(Debug, thiserror::Error)]
pub enum ProcessorError {
    #[error("unknown processor: {0}")]
    UnknownKind(String),
    #[error("invalid parameters for {kind}: {detail}")]
    Params { kind: String, detail: String },
}

/// フィードを変換する処理。副作用を持たない純粋な変換として実装する。
pub trait Processor: Send + Sync {
    fn name(&self) -> &'static str;
    fn apply(&self, feed: Feed) -> Result<Feed, ProcessorError>;

    /// 実際に使われるパラメータを JSON で返す。
    /// 省略された項目は既定値で埋まるため、保存時にこれを書き戻すことで
    /// 「何を設定したのか」が後から画面上で分かる。
    fn params(&self) -> String {
        "{}".into()
    }
}

/// Processor の種別ごとの説明。CLI と Web UI の両方から参照する。
pub struct ProcessorInfo {
    pub kind: &'static str,
    pub summary: &'static str,
    pub params: &'static [ParamInfo],
}

pub struct ParamInfo {
    pub name: &'static str,
    pub description: &'static str,
    pub default: &'static str,
    /// 取りうる値。自由入力なら空
    pub values: &'static [&'static str],
}

const DEDUPE_PARAMS: &[ParamInfo] = &[ParamInfo {
    name: "key",
    description: "同一と判定する基準。値が空の item は判定できないためそのまま残す",
    default: "link",
    values: &["guid", "link", "normalized_title"],
}];

const EXCLUDE_PARAMS: &[ParamInfo] = &[
    ParamInfo {
        name: "words",
        description: "この語を含む item を落とす。配列で複数指定でき、1 つでも一致すれば対象。大文字小文字は区別しない",
        default: "[]",
        values: &[],
    },
    ParamInfo {
        name: "target",
        description: "どこを見るか。description は HTML を落としてから探す",
        default: "title",
        values: &["title", "description", "both"],
    },
];

const CATALOG: &[ProcessorInfo] = &[
    ProcessorInfo {
        kind: "google_news_cluster",
        summary: "Google ニュース専用。description の関連記事リストから、title と重複する先頭要素を取り除く。関連記事は残す",
        params: &[],
    },
    ProcessorInfo {
        kind: "exclude",
        summary: "指定した語を含む item を取り除く。上流の検索条件で書ききれなかった語を落とす",
        params: EXCLUDE_PARAMS,
    },
    ProcessorInfo {
        kind: "dedupe",
        summary: "同じ item が複数あるとき、先に出現したものだけを残す",
        params: DEDUPE_PARAMS,
    },
];

pub fn catalog() -> &'static [ProcessorInfo] {
    CATALOG
}

/// DB に保存された (kind, params) から Processor を組み立てる。
/// params は JSON。空文字列は既定値とみなす。
pub fn build(kind: &str, params: &str) -> Result<Box<dyn Processor>, ProcessorError> {
    let json = if params.trim().is_empty() {
        "{}"
    } else {
        params
    };
    let parse = |detail: serde_json::Error| ProcessorError::Params {
        kind: kind.to_string(),
        detail: detail.to_string(),
    };

    match kind {
        "google_news_cluster" => Ok(Box::new(GoogleNewsCluster)),
        "dedupe" => Ok(Box::new(
            serde_json::from_str::<Dedupe>(json).map_err(parse)?,
        )),
        "exclude" => Ok(Box::new(
            serde_json::from_str::<Exclude>(json).map_err(parse)?,
        )),
        other => Err(ProcessorError::UnknownKind(other.to_string())),
    }
}

/// 連鎖を順に適用する。
pub fn apply_chain(chain: &[Box<dyn Processor>], feed: Feed) -> Result<Feed, ProcessorError> {
    chain.iter().try_fold(feed, |f, p| p.apply(f))
}
