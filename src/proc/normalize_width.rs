//! 全角の英数字と記号を半角に直す。
//!
//! 「岐阜のケーキ店３人死亡火災」「レベル５」のように、報道系のフィードは
//! 数字を全角で書くことが多い。検索や読みやすさのために半角へ揃える。
//!
//! 変換するのは U+FF01〜U+FF5E (全角 ASCII) だけ。カギ括弧 (「」『』)、句読点
//! (、。)、なかてん (・)、波ダッシュ (〜) はいずれもこの範囲の外にあるため、
//! 何もしなくてもそのまま残る。範囲内にある全角チルダ (～ U+FF5E) だけは
//! 日本語で区間を表す記号として使われるので明示的に除く。

use crate::model::Feed;
use crate::proc::{Documents, Processor, ProcessorError, Target};

/// 全角 ASCII の開始位置と、半角との差。
const FULL_WIDTH_START: char = '\u{FF01}';
const FULL_WIDTH_END: char = '\u{FF5E}';
const OFFSET: u32 = 0xFEE0;
/// 全角チルダ。範囲内だが日本語の記号として使われるので変換しない
const FULL_WIDTH_TILDE: char = '\u{FF5E}';

#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct NormalizeWidth {
    pub target: Target,
}

impl Processor for NormalizeWidth {
    fn name(&self) -> &'static str {
        "normalize_width"
    }

    fn params(&self) -> String {
        serde_json::to_string(self).expect("normalize_width のパラメータを直列化できない")
    }

    fn apply(&self, mut feed: Feed, _docs: &Documents) -> Result<Feed, ProcessorError> {
        for item in &mut feed.items {
            if self.target.includes_title() {
                item.title = item.title.as_deref().map(to_half_width);
            }
            if self.target.includes_description() {
                item.description = item.description.as_deref().map(to_half_width);
            }
        }
        Ok(feed)
    }
}

fn to_half_width(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            FULL_WIDTH_TILDE => c,
            FULL_WIDTH_START..=FULL_WIDTH_END => char::from_u32(c as u32 - OFFSET).unwrap_or(c),
            _ => c,
        })
        .collect()
}
