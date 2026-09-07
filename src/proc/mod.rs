pub mod dedupe;
pub mod vendor;

use crate::model::Feed;
use crate::proc::dedupe::Dedupe;
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
        other => Err(ProcessorError::UnknownKind(other.to_string())),
    }
}

/// 連鎖を順に適用する。
pub fn apply_chain(chain: &[Box<dyn Processor>], feed: Feed) -> Result<Feed, ProcessorError> {
    chain.iter().try_fold(feed, |f, p| p.apply(f))
}
