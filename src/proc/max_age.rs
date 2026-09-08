//! 古い item を落とす。
//!
//! 上流が過去分をいつまでも載せ続けるフィードで、リーダーの一覧が古い記事で
//! 埋まるのを防ぐ。

use chrono::{Duration, Utc};

use crate::model::Feed;
use crate::proc::{Documents, Processor, ProcessorError};

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MaxAge {
    /// この時間より前の item を落とす
    pub hours: i64,
}

impl Default for MaxAge {
    fn default() -> Self {
        Self { hours: 24 }
    }
}

impl Processor for MaxAge {
    fn name(&self) -> &'static str {
        "max_age"
    }

    fn params(&self) -> String {
        serde_json::to_string(self).expect("max_age のパラメータを直列化できない")
    }

    fn apply(&self, mut feed: Feed, _docs: &Documents) -> Result<Feed, ProcessorError> {
        let limit = Utc::now() - Duration::hours(self.hours.max(0));

        // 日時を持たない item は古いかどうか判断できないので残す。
        // 未来の日時 (上流のタイムゾーン誤りなど) も古くはないので残る
        feed.items
            .retain(|item| item.published.is_none_or(|published| published >= limit));
        Ok(feed)
    }
}
