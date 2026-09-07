//! item 間の重複除去。先に出現したものを残す。

use std::collections::HashSet;

use crate::model::{Feed, Item};
use crate::proc::{Processor, ProcessorError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Key {
    Guid,
    #[default]
    Link,
    NormalizedTitle,
}

pub struct Dedupe {
    pub key: Key,
}

impl Processor for Dedupe {
    fn name(&self) -> &'static str {
        "dedupe"
    }

    fn apply(&self, mut feed: Feed) -> Result<Feed, ProcessorError> {
        let mut seen = HashSet::new();
        // キーを持たない item は判定できないので、そのまま残す
        feed.items
            .retain(|item| self.key_of(item).is_none_or(|k| seen.insert(k)));
        Ok(feed)
    }
}

impl Dedupe {
    fn key_of(&self, item: &Item) -> Option<String> {
        match self.key {
            Key::Guid => item.id.clone(),
            Key::Link => item.link.clone(),
            Key::NormalizedTitle => item
                .title
                .as_deref()
                .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|t| !t.is_empty()),
        }
    }
}
