//! 有料記事の扱い。判定そのものは取得段階で済んでいる (`Item::paywalled`)。
//! ここは判定結果をどう扱うかだけを決める。

use crate::model::Feed;
use crate::proc::{Documents, Processor, ProcessorError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// title に印を付けて残す
    #[default]
    Mark,
    /// 取り除く
    Exclude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unknown {
    /// 判定できなかったものは残す
    #[default]
    Keep,
    Exclude,
}

fn default_prefix() -> String {
    "[有料] ".into()
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Paywall {
    pub action: Action,
    pub unknown: Unknown,
    /// `action` が `mark` のときに title の先頭へ付ける文字列
    pub prefix: String,
}

impl Default for Paywall {
    fn default() -> Self {
        Self {
            action: Action::default(),
            unknown: Unknown::default(),
            prefix: default_prefix(),
        }
    }
}

impl Processor for Paywall {
    fn name(&self) -> &'static str {
        "paywall"
    }

    fn params(&self) -> String {
        serde_json::to_string(self).expect("paywall のパラメータを直列化できない")
    }

    fn apply(&self, mut feed: Feed, _docs: &Documents) -> Result<Feed, ProcessorError> {
        if self.unknown == Unknown::Exclude {
            feed.items.retain(|i| i.paywalled.is_some());
        }
        match self.action {
            Action::Exclude => feed.items.retain(|i| i.paywalled != Some(true)),
            Action::Mark => {
                for item in &mut feed.items {
                    if item.paywalled != Some(true) {
                        continue;
                    }
                    // 二重に付かないようにする。巡回のたびに適用されるため
                    item.title = item.title.take().map(|t| {
                        if t.starts_with(&self.prefix) {
                            t
                        } else {
                            format!("{}{t}", self.prefix)
                        }
                    });
                }
            }
        }
        Ok(feed)
    }
}
