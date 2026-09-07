use rss_proxy::proc::Processor;
use rss_proxy::proc::vendor::google_news::GoogleNewsCluster;
use rss_proxy::{model::Item, parse};

const FIXTURE: &str = include_str!("fixtures/google_news_headline.xml");

fn item(title: &str, description: Option<&str>) -> Item {
    Item {
        id: None,
        title: Some(title.to_string()),
        link: None,
        description: description.map(str::to_string),
        published: None,
        authors: vec![],
        categories: vec![],
    }
}

fn apply_to_one(it: Item) -> Item {
    let mut feed = parse::parse(FIXTURE.as_bytes()).unwrap();
    feed.items = vec![it];
    GoogleNewsCluster.apply(feed).unwrap().items.pop().unwrap()
}

#[test]
fn single_form_description_becomes_empty() {
    let out = apply_to_one(item(
        "見出しテキスト - 読売新聞",
        Some(
            r##"<a href="https://example.com/a" target="_blank">見出しテキスト</a>&nbsp;&nbsp;<font color="#6f6f6f">読売新聞</font>"##,
        ),
    ));
    assert_eq!(out.description.as_deref(), Some(""));
    assert_eq!(out.title.as_deref(), Some("見出しテキスト - 読売新聞"));
}

#[test]
fn cluster_form_drops_only_the_duplicate_head() {
    let out = apply_to_one(item(
        "一件目 - 読売新聞",
        Some(concat!(
            "<ol>",
            r##"<li><a href="https://example.com/1">一件目</a>&nbsp;&nbsp;<font color="#6f6f6f">読売新聞</font></li>"##,
            r##"<li><a href="https://example.com/2">二件目</a>&nbsp;&nbsp;<font color="#6f6f6f">NHK</font></li>"##,
            r##"<li><a href="https://example.com/3">三件目</a>&nbsp;&nbsp;<font color="#6f6f6f">時事</font></li>"##,
            "</ol>",
        )),
    ));
    let d = out.description.unwrap();
    assert!(!d.contains("一件目"), "重複した先頭要素が残っている: {d}");
    assert!(d.contains("二件目") && d.contains("三件目"));
    // 順序が保たれること
    assert!(d.find("二件目").unwrap() < d.find("三件目").unwrap());
    // リンクが保たれること
    assert!(d.contains("https://example.com/2") && d.contains("https://example.com/3"));
    assert!(d.starts_with("<ol>") && d.ends_with("</ol>"));
}

#[test]
fn cluster_of_one_becomes_empty() {
    let out = apply_to_one(item(
        "唯一 - 読売新聞",
        Some(
            r##"<ol><li><a href="https://example.com/1">唯一</a>&nbsp;&nbsp;<font color="#6f6f6f">読売新聞</font></li></ol>"##,
        ),
    ));
    assert_eq!(out.description.as_deref(), Some(""));
}

#[test]
fn unrelated_description_is_untouched() {
    let original = "<p>ふつうの本文の要約です。</p>";
    let out = apply_to_one(item("まったく別の見出し", Some(original)));
    assert_eq!(out.description.as_deref(), Some(original));
}

#[test]
fn missing_fields_do_not_panic() {
    assert_eq!(apply_to_one(item("見出し", None)).description, None);
    assert_eq!(
        apply_to_one(item("見出し", Some("")))
            .description
            .as_deref(),
        Some("")
    );

    let mut no_title = item("x", Some("<ol><li><a href=\"u\">y</a></li></ol>"));
    no_title.title = None;
    assert!(apply_to_one(no_title).description.is_some());
}

#[test]
fn every_fixture_item_loses_its_duplicate_head() {
    let feed = parse::parse(FIXTURE.as_bytes()).unwrap();
    let before = feed.items.clone();
    let after = GoogleNewsCluster.apply(feed).unwrap();

    assert_eq!(after.items.len(), before.len());

    let mut cleared = 0;
    let mut trimmed = 0;
    for (b, a) in before.iter().zip(after.items.iter()) {
        assert_eq!(b.title, a.title, "title は変更されない");
        let bd = b.description.as_deref().unwrap();
        let ad = a.description.as_deref().unwrap();
        if bd.starts_with("<ol>") {
            trimmed += 1;
            // 先頭の重複要素が 1 つだけ消え、残りは維持される
            assert_eq!(
                ad.matches("<li>").count(),
                bd.matches("<li>").count() - 1,
                "li が 1 件だけ減っていない"
            );
        } else {
            cleared += 1;
            assert_eq!(ad, "", "単一形式は空になる");
        }
    }
    assert_eq!(cleared, 7, "単一形式は 7 件");
    assert_eq!(trimmed, 63, "クラスタ形式は 63 件");
}
