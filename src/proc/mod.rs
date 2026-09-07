pub mod dedupe;
pub mod exclude;
pub mod google_news;
pub mod max_age;
pub mod normalize_width;

use crate::model::Feed;
use crate::proc::dedupe::Dedupe;
use crate::proc::exclude::Exclude;
use crate::proc::google_news::GoogleNewsCluster;
use crate::proc::max_age::MaxAge;
use crate::proc::normalize_width::NormalizeWidth;

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

/// item のどのフィールドを見るか。複数の Processor が同じ選択肢を持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    #[default]
    Title,
    Description,
    Both,
}

impl Target {
    pub fn includes_title(self) -> bool {
        matches!(self, Self::Title | Self::Both)
    }

    pub fn includes_description(self) -> bool {
        matches!(self, Self::Description | Self::Both)
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

const MAX_AGE_PARAMS: &[ParamInfo] = &[ParamInfo {
    name: "hours",
    description: "この時間より前の item を落とす。日時を持たない item は判断できないので残す",
    default: "24",
    values: &[],
}];

const NORMALIZE_WIDTH_PARAMS: &[ParamInfo] = &[ParamInfo {
    name: "target",
    description: "どこを直すか",
    default: "title",
    values: &["title", "description", "both"],
}];

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
        kind: "max_age",
        summary: "指定した時間より古い item を落とす",
        params: MAX_AGE_PARAMS,
    },
    ProcessorInfo {
        kind: "normalize_width",
        summary: "全角の英数字と記号を半角に直す。カギ括弧・句読点・なかてん・波ダッシュはそのまま",
        params: NORMALIZE_WIDTH_PARAMS,
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
        "max_age" => Ok(Box::new(
            serde_json::from_str::<MaxAge>(json).map_err(parse)?,
        )),
        "normalize_width" => Ok(Box::new(
            serde_json::from_str::<NormalizeWidth>(json).map_err(parse)?,
        )),
        other => Err(ProcessorError::UnknownKind(other.to_string())),
    }
}
