pub mod dedupe;
pub mod vendor;

use crate::model::Feed;

#[derive(Debug, thiserror::Error)]
pub enum ProcessorError {
    #[error("invalid parameters for {kind}: {detail}")]
    Params { kind: &'static str, detail: String },
}

/// フィードを変換する処理。副作用を持たない純粋な変換として実装する。
pub trait Processor: Send + Sync {
    fn name(&self) -> &'static str;
    fn apply(&self, feed: Feed) -> Result<Feed, ProcessorError>;
}
