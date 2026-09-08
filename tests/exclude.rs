use rss_proxy::model::{Feed, Item};
use rss_proxy::proc::Processor;
use rss_proxy::proc::Target;
use rss_proxy::proc::exclude::Exclude;

fn item(title: &str, description: &str) -> Item {
    Item {
        id: None,
        title: Some(title.into()),
        link: None,
        description: Some(description.into()),
        published: None,
        authors: vec![],
        categories: vec![],
        paywalled: None,
    }
}

fn feed(items: Vec<Item>) -> Feed {
    Feed {
        title: "t".into(),
        link: None,
        description: None,
        updated: None,
        items,
    }
}

fn titles(feed: &Feed) -> Vec<String> {
    feed.items
        .iter()
        .map(|i| i.title.clone().unwrap_or_default())
        .collect()
}

#[test]
fn removes_items_whose_title_contains_any_keyword() {
    let out = Exclude {
        words: vec!["スポーツ".into(), "競馬".into()],
        target: Target::Title,
    }
    .apply(feed(vec![
        item("プロ野球のスポーツ面", ""),
        item("政治の話題", ""),
        item("競馬の結果", ""),
        item("経済ニュース", ""),
    ]))
    .unwrap();

    assert_eq!(titles(&out), vec!["政治の話題", "経済ニュース"]);
}

#[test]
fn matching_is_case_insensitive_for_ascii() {
    let out = Exclude {
        words: vec!["abema".into()],
        target: Target::Title,
    }
    .apply(feed(vec![item("ABEMA で配信", ""), item("残す記事", "")]))
    .unwrap();

    assert_eq!(titles(&out), vec!["残す記事"]);
}

#[test]
fn description_is_only_checked_when_asked() {
    let items = vec![item("普通の見出し", "本文にスポーツと書いてある")];

    let kept = Exclude {
        words: vec!["スポーツ".into()],
        target: Target::Title,
    }
    .apply(feed(items.clone()))
    .unwrap();
    assert_eq!(kept.items.len(), 1, "既定では title だけを見る");

    let removed = Exclude {
        words: vec!["スポーツ".into()],
        target: Target::Both,
    }
    .apply(feed(items))
    .unwrap();
    assert!(removed.items.is_empty());
}

#[test]
fn html_in_the_description_does_not_hide_a_keyword() {
    let out = Exclude {
        words: vec!["スポーツ".into()],
        target: Target::Both,
    }
    .apply(feed(vec![item("見出し", "<b>スポ</b>ーツ")]))
    .unwrap();

    assert!(out.items.is_empty(), "タグをまたいだ語も検出する");
}

#[test]
fn no_keywords_removes_nothing() {
    let out = Exclude {
        words: vec![],
        target: Target::Both,
    }
    .apply(feed(vec![item("何でも", "何でも")]))
    .unwrap();
    assert_eq!(out.items.len(), 1);
}

#[test]
fn items_without_the_target_field_are_kept() {
    let mut no_title = item("x", "y");
    no_title.title = None;

    let out = Exclude {
        words: vec!["x".into()],
        target: Target::Title,
    }
    .apply(feed(vec![no_title]))
    .unwrap();
    assert_eq!(out.items.len(), 1);
}
