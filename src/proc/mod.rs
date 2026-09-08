pub mod dedupe;
pub mod exclude;
pub mod google_news;
pub mod jma;
pub mod max_age;
pub mod normalize_width;
pub mod paywall;

use crate::model::Feed;
use crate::proc::dedupe::Dedupe;
use crate::proc::exclude::Exclude;
use crate::proc::google_news::GoogleNewsCluster;
use crate::proc::jma::JmaWarning;
use crate::proc::max_age::MaxAge;
use crate::proc::normalize_width::NormalizeWidth;
use crate::proc::paywall::Paywall;

#[derive(Debug, thiserror::Error)]
pub enum ProcessorError {
    #[error("unknown processor: {0}")]
    UnknownKind(String),
    #[error("invalid parameters for {kind}: {detail}")]
    Params { kind: String, detail: String },
}

/// 取得済みの外部文書。URL で引く。
///
/// 一部の Processor は外部の文書を必要とする (気象庁の XML など)。
/// それでも Processor 自体は純粋なままにしたいので、「何が必要か」を
/// [`Processor::wants`] で宣言させ、取得は呼び出し側が行い、結果をここに入れて渡す。
#[derive(Debug, Default)]
pub struct Documents(std::collections::HashMap<String, String>);

impl Documents {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, url: String, body: String) {
        self.0.insert(url, body);
    }

    pub fn get(&self, url: &str) -> Option<&str> {
        self.0.get(url).map(String::as_str)
    }
}

/// フィードを変換する処理。副作用を持たない純粋な変換として実装する。
pub trait Processor: Send + Sync {
    fn name(&self) -> &'static str;
    fn apply(&self, feed: Feed, docs: &Documents) -> Result<Feed, ProcessorError>;

    /// 処理に必要な外部文書の URL。既定は空。
    /// 取得は呼び出し側が行い、結果を `apply` の `docs` に入れて渡す。
    fn wants(&self, _feed: &Feed) -> Vec<String> {
        Vec::new()
    }

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

const PAYWALL_PARAMS: &[ParamInfo] = &[
    ParamInfo {
        name: "action",
        description: "有料と判定した item をどうするか",
        default: "mark",
        values: &["mark", "exclude"],
    },
    ParamInfo {
        name: "unknown",
        description: "判定できなかった item の扱い。目印を出さない媒体があるため、無料とは決めつけない",
        default: "keep",
        values: &["keep", "exclude"],
    },
    ParamInfo {
        name: "prefix",
        description: "action が mark のときに title の先頭へ付ける文字列",
        default: "[有料] ",
        values: &[],
    },
];

const JMA_PARAMS: &[ParamInfo] = &[
    ParamInfo {
        name: "areas",
        description: "対象の市区町村名。前方一致するので「神戸市」で 9 区すべてを拾う。「神戸市北区」と書けばその区だけ",
        default: "[]",
        values: &[],
    },
    ParamInfo {
        name: "kinds",
        description: "残す種別。部分一致。空なら全部。「警報」は「特別警報」も拾う",
        default: "[]",
        values: &[],
    },
    ParamInfo {
        name: "new_prefix",
        description: "今回新しく発表された種別の前に付ける。空にすれば印を付けない",
        default: "【新】",
        values: &[],
    },
    ParamInfo {
        name: "new_suffix",
        description: "同じく後ろに付ける。Slack の太字なら前後とも * にする",
        default: "",
        values: &[],
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
        kind: "jma_warning",
        summary: "気象庁専用。防災情報 XML から、指定した市区町村の警報・注意報を取り出して本文にする。今回新しく発表されたものには印を付ける。該当のない発表は落とす",
        params: JMA_PARAMS,
    },
    ProcessorInfo {
        kind: "paywall",
        summary: "有料記事に印を付ける、または取り除く。判定できる媒体は読売新聞と日本経済新聞",
        params: PAYWALL_PARAMS,
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
        "jma_warning" => Ok(Box::new(
            serde_json::from_str::<JmaWarning>(json).map_err(parse)?,
        )),
        "paywall" => Ok(Box::new(
            serde_json::from_str::<Paywall>(json).map_err(parse)?,
        )),
        other => Err(ProcessorError::UnknownKind(other.to_string())),
    }
}
