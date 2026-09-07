use chrono::{DateTime, Utc};

/// Processor の入出力となる正規化済みフィード。
#[derive(Debug, Clone, PartialEq)]
pub struct Feed {
    pub title: String,
    pub link: Option<String>,
    pub description: Option<String>,
    pub updated: Option<DateTime<Utc>>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// RSS の guid / Atom の id
    pub id: Option<String>,
    pub title: Option<String>,
    pub link: Option<String>,
    /// HTML を含みうる。エスケープを解いた状態で保持する
    pub description: Option<String>,
    pub published: Option<DateTime<Utc>>,
    pub authors: Vec<String>,
    pub categories: Vec<String>,
}
