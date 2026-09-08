use crate::model::{Feed, Item};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("feed parse failed: {0}")]
    Feed(#[from] feed_rs::parser::ParseFeedError),
}

/// RSS/Atom のバイト列を内部モデルへ変換する。
pub fn parse(bytes: &[u8]) -> Result<Feed, ParseError> {
    let parsed = feed_rs::parser::parse(bytes)?;

    Ok(Feed {
        title: parsed.title.map(|t| t.content).unwrap_or_default(),
        link: first_link(&parsed.links),
        description: parsed.description.map(|t| t.content),
        updated: parsed.updated,
        items: parsed.entries.into_iter().map(to_item).collect(),
    })
}

fn to_item(entry: feed_rs::model::Entry) -> Item {
    Item {
        id: (!entry.id.is_empty()).then_some(entry.id),
        title: entry.title.map(|t| t.content),
        link: first_link(&entry.links),
        // RSS の description は summary に入る。Atom は content を持つことがある
        description: entry
            .summary
            .map(|t| t.content)
            .or_else(|| entry.content.and_then(|c| c.body)),
        published: entry.published.or(entry.updated),
        authors: entry.authors.into_iter().map(|p| p.name).collect(),
        categories: entry.categories.into_iter().map(|c| c.term).collect(),
        // 判定は取得段階で行う
        paywalled: None,
    }
}

fn first_link(links: &[feed_rs::model::Link]) -> Option<String> {
    links.first().map(|l| l.href.clone())
}
