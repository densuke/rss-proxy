use rss_proxy::model::{Feed, Item};
use rss_proxy::proc::paywall::{Action, Paywall, Unknown};
use rss_proxy::proc::{Documents, Processor};

fn item(title: &str, paywalled: Option<bool>) -> Item {
    Item {
        id: None,
        title: Some(title.into()),
        link: None,
        description: None,
        published: None,
        authors: vec![],
        categories: vec![],
        paywalled,
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

fn titles(f: &Feed) -> Vec<String> {
    f.items
        .iter()
        .map(|i| i.title.clone().unwrap_or_default())
        .collect()
}

#[test]
fn excluding_drops_only_the_paid_ones() {
    let out = Paywall {
        action: Action::Exclude,
        unknown: Unknown::Keep,
        prefix: "[有料] ".into(),
    }
    .apply(
        feed(vec![
            item("有料", Some(true)),
            item("無料", Some(false)),
            item("不明", None),
        ]),
        &Documents::empty(),
    )
    .unwrap();

    assert_eq!(titles(&out), vec!["無料", "不明"]);
}

#[test]
fn unknown_can_be_dropped_too() {
    let out = Paywall {
        action: Action::Exclude,
        unknown: Unknown::Exclude,
        prefix: "[有料] ".into(),
    }
    .apply(
        feed(vec![
            item("有料", Some(true)),
            item("無料", Some(false)),
            item("不明", None),
        ]),
        &Documents::empty(),
    )
    .unwrap();

    assert_eq!(titles(&out), vec!["無料"]);
}

#[test]
fn marking_keeps_everything_and_labels_the_paid_ones() {
    let out = Paywall {
        action: Action::Mark,
        unknown: Unknown::Keep,
        prefix: "[有料] ".into(),
    }
    .apply(
        feed(vec![
            item("有料", Some(true)),
            item("無料", Some(false)),
            item("不明", None),
        ]),
        &Documents::empty(),
    )
    .unwrap();

    assert_eq!(titles(&out), vec!["[有料] 有料", "無料", "不明"]);
}

#[test]
fn marking_twice_does_not_stack_the_label() {
    let p = Paywall {
        action: Action::Mark,
        unknown: Unknown::Keep,
        prefix: "[有料] ".into(),
    };
    let once = p
        .apply(feed(vec![item("記事", Some(true))]), &Documents::empty())
        .unwrap();
    let twice = p.apply(once, &Documents::empty()).unwrap();
    assert_eq!(titles(&twice), vec!["[有料] 記事"]);
}

#[test]
fn items_without_a_title_do_not_panic() {
    let mut bare = item("x", Some(true));
    bare.title = None;
    let out = Paywall {
        action: Action::Mark,
        unknown: Unknown::Keep,
        prefix: "[有料] ".into(),
    }
    .apply(feed(vec![bare]), &Documents::empty())
    .unwrap();
    assert_eq!(out.items.len(), 1);
}
