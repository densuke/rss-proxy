//! Processor は純粋なまま保ちたいが、外部の文書を必要とするものがある。
//! 「何が必要か」を宣言させ、取得は呼び出し側が行い、結果を渡す形にする。

use rss_proxy::model::{Feed, Item};
use rss_proxy::proc::{Documents, build};

fn feed(links: &[&str]) -> Feed {
    Feed {
        title: "t".into(),
        link: None,
        description: None,
        updated: None,
        items: links
            .iter()
            .map(|l| Item {
                id: None,
                title: Some("t".into()),
                link: Some((*l).to_string()),
                description: None,
                published: None,
                authors: vec![],
                categories: vec![],
                paywalled: None,
            })
            .collect(),
    }
}

#[test]
fn most_processors_need_nothing() {
    let empty = feed(&["https://example.com/a"]);
    for kind in ["dedupe", "exclude", "max_age", "normalize_width", "paywall"] {
        let p = build(kind, "").unwrap();
        assert!(p.wants(&empty).is_empty(), "{kind} が取得を要求している");
    }
}

#[test]
fn a_processor_can_be_applied_without_any_documents() {
    let p = build("dedupe", "").unwrap();
    let out = p
        .apply(feed(&["https://a", "https://a"]), &Documents::empty())
        .unwrap();
    assert_eq!(out.items.len(), 1);
}

#[test]
fn documents_are_looked_up_by_url() {
    let mut docs = Documents::empty();
    docs.insert("https://example.com/a".into(), "本文A".into());

    assert_eq!(docs.get("https://example.com/a"), Some("本文A"));
    assert_eq!(docs.get("https://example.com/b"), None);
}
