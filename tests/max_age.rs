use chrono::{Duration, Utc};
use rss_proxy::model::{Feed, Item};
use rss_proxy::proc::Processor;
use rss_proxy::proc::max_age::MaxAge;

fn item(title: &str, hours_ago: Option<i64>) -> Item {
    Item {
        id: None,
        title: Some(title.into()),
        link: None,
        description: None,
        published: hours_ago.map(|h| Utc::now() - Duration::hours(h)),
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
fn drops_items_older_than_the_limit() {
    let out = MaxAge { hours: 24 }
        .apply(feed(vec![
            item("1時間前", Some(1)),
            item("23時間前", Some(23)),
            item("25時間前", Some(25)),
            item("1週間前", Some(24 * 7)),
        ]))
        .unwrap();

    assert_eq!(titles(&out), vec!["1時間前", "23時間前"]);
}

#[test]
fn items_without_a_date_are_kept() {
    // 日時が分からないものは古いかどうか判断できない。落とさない
    let out = MaxAge { hours: 1 }
        .apply(feed(vec![item("日時なし", None), item("古い", Some(100))]))
        .unwrap();

    assert_eq!(titles(&out), vec!["日時なし"]);
}

#[test]
fn future_dates_are_kept() {
    // 上流のタイムゾーン誤りなどで未来になることがある。古くはないので残す
    let out = MaxAge { hours: 24 }
        .apply(feed(vec![item("未来", Some(-5))]))
        .unwrap();
    assert_eq!(out.items.len(), 1);
}

#[test]
fn zero_hours_removes_everything_dated() {
    let out = MaxAge { hours: 0 }
        .apply(feed(vec![item("1時間前", Some(1)), item("日時なし", None)]))
        .unwrap();
    assert_eq!(titles(&out), vec!["日時なし"]);
}
