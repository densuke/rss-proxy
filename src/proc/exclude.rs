//! 指定した語を含む item を取り除く。
//!
//! 上流の検索フィード側でも除外を書けるが、書ききれなかった語や、後から気づいた語を
//! こちら側で落とすために使う。

use crate::html::to_plain_text;
use crate::model::{Feed, Item};
use crate::proc::{Processor, ProcessorError, Target};

#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Exclude {
    /// この語を含む item を落とす。1 つでも一致すれば対象
    pub words: Vec<String>,
    pub target: Target,
}

impl Processor for Exclude {
    fn name(&self) -> &'static str {
        "exclude"
    }

    fn params(&self) -> String {
        serde_json::to_string(self).expect("exclude のパラメータを直列化できない")
    }

    fn apply(&self, mut feed: Feed) -> Result<Feed, ProcessorError> {
        if self.words.is_empty() {
            return Ok(feed);
        }
        // 大文字小文字は無視する。日本語には影響しないが ABEMA のような表記ゆれを拾える
        let words: Vec<String> = self.words.iter().map(|w| w.to_lowercase()).collect();

        feed.items.retain(|item| !self.matches(item, &words));
        Ok(feed)
    }
}

impl Exclude {
    fn matches(&self, item: &Item, words: &[String]) -> bool {
        let mut haystack = String::new();
        if self.target.includes_title() {
            haystack.push_str(item.title.as_deref().unwrap_or_default());
            haystack.push('\n');
        }
        if self.target.includes_description() {
            haystack.push_str(&to_plain_text(
                item.description.as_deref().unwrap_or_default(),
            ));
        }
        let haystack = haystack.to_lowercase();
        // タグで分断された語も拾えるよう、空白を除いた形でも探す
        let squeezed: String = haystack.chars().filter(|c| !c.is_whitespace()).collect();

        words
            .iter()
            .any(|word| haystack.contains(word) || squeezed.contains(word))
    }
}
