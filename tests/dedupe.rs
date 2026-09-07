use rss_proxy::model::{Feed, Item};
use rss_proxy::proc::Processor;
use rss_proxy::proc::dedupe::{Dedupe, Key};

fn item(id: Option<&str>, title: Option<&str>, link: Option<&str>) -> Item {
    Item {
        id: id.map(str::to_string),
        title: title.map(str::to_string),
        link: link.map(str::to_string),
        description: None,
        published: None,
        authors: vec![],
        categories: vec![],
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

fn titles(feed: &Feed) -> Vec<&str> {
    feed.items
        .iter()
        .map(|i| i.title.as_deref().unwrap_or(""))
        .collect()
}

#[test]
fn drops_later_items_with_the_same_link() {
    let out = Dedupe { key: Key::Link }
        .apply(feed(vec![
            item(None, Some("1件目"), Some("https://a")),
            item(None, Some("2件目"), Some("https://b")),
            item(None, Some("1件目の再掲"), Some("https://a")),
        ]))
        .unwrap();

    assert_eq!(titles(&out), vec!["1件目", "2件目"]);
}

#[test]
fn dedupes_by_guid() {
    let out = Dedupe { key: Key::Guid }
        .apply(feed(vec![
            item(Some("g1"), Some("1件目"), Some("https://a")),
            item(Some("g1"), Some("別 URL の同一 guid"), Some("https://b")),
            item(Some("g2"), Some("2件目"), Some("https://c")),
        ]))
        .unwrap();

    assert_eq!(titles(&out), vec!["1件目", "2件目"]);
}

#[test]
fn dedupes_by_normalized_title() {
    let out = Dedupe {
        key: Key::NormalizedTitle,
    }
    .apply(feed(vec![
        item(None, Some("同じ 見出し"), Some("https://a")),
        item(None, Some("　同じ　見出し　"), Some("https://b")),
        item(None, Some("違う見出し"), Some("https://c")),
    ]))
    .unwrap();

    assert_eq!(titles(&out), vec!["同じ 見出し", "違う見出し"]);
}

#[test]
fn items_without_the_key_are_all_kept() {
    let out = Dedupe { key: Key::Link }
        .apply(feed(vec![
            item(None, Some("1件目"), None),
            item(None, Some("2件目"), None),
        ]))
        .unwrap();

    assert_eq!(titles(&out), vec!["1件目", "2件目"]);
}

#[test]
fn preserves_order_and_handles_empty_feed() {
    let out = Dedupe { key: Key::Link }
        .apply(feed(vec![
            item(None, Some("c"), Some("https://c")),
            item(None, Some("a"), Some("https://a")),
            item(None, Some("b"), Some("https://b")),
        ]))
        .unwrap();
    assert_eq!(titles(&out), vec!["c", "a", "b"]);

    assert!(
        Dedupe { key: Key::Link }
            .apply(feed(vec![]))
            .unwrap()
            .items
            .is_empty()
    );
}
