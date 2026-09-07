use rss_proxy::{parse, render};

const FIXTURE: &str = include_str!("fixtures/google_news_headline.xml");

#[test]
fn parses_fixture_into_feed() {
    let feed = parse::parse(FIXTURE.as_bytes()).expect("parse should succeed");

    assert_eq!(feed.title, "ヘッドライン - 最新 - Google ニュース");
    assert_eq!(feed.items.len(), 70);
}

#[test]
fn parses_item_fields() {
    let feed = parse::parse(FIXTURE.as_bytes()).unwrap();
    let item = &feed.items[0];

    assert!(
        item.title
            .as_deref()
            .unwrap()
            .starts_with("岐阜のケーキ店３人死亡火災")
    );
    assert!(item.title.as_deref().unwrap().ends_with(" - 読売新聞"));
    assert!(
        item.link
            .as_deref()
            .unwrap()
            .starts_with("https://news.google.com/rss/articles/")
    );
    assert!(item.id.is_some());
    assert!(item.published.is_some());
    // description は HTML がアンエスケープされた状態で入る
    assert!(item.description.as_deref().unwrap().starts_with("<ol>"));
}

#[test]
fn every_item_has_title_link_and_description() {
    let feed = parse::parse(FIXTURE.as_bytes()).unwrap();
    for (i, item) in feed.items.iter().enumerate() {
        assert!(item.title.is_some(), "item {i} has no title");
        assert!(item.link.is_some(), "item {i} has no link");
        assert!(item.description.is_some(), "item {i} has no description");
    }
}

#[test]
fn renders_back_to_parseable_rss() {
    let feed = parse::parse(FIXTURE.as_bytes()).unwrap();
    let xml = render::to_rss2(&feed);

    let reparsed = parse::parse(xml.as_bytes()).expect("rendered output should re-parse");
    assert_eq!(reparsed.title, feed.title);
    assert_eq!(reparsed.items.len(), feed.items.len());
}

#[test]
fn render_preserves_item_content() {
    let feed = parse::parse(FIXTURE.as_bytes()).unwrap();
    let reparsed = parse::parse(render::to_rss2(&feed).as_bytes()).unwrap();

    for (a, b) in feed.items.iter().zip(reparsed.items.iter()) {
        assert_eq!(a.title, b.title);
        assert_eq!(a.link, b.link);
        assert_eq!(a.description, b.description);
        assert_eq!(a.published, b.published);
    }
}

#[test]
fn rejects_garbage_input() {
    assert!(parse::parse(b"not a feed at all").is_err());
}

/// 上流の時刻表記は GMT / Z / +0900 と揃っていない。
/// 内部では UTC に正規化し、出力も UTC で書き出す。瞬間は保たれる。
#[test]
fn timezone_offsets_are_normalised_to_utc_without_shifting_the_instant() {
    let src = r#"<?xml version="1.0"?><rss version="2.0"><channel>
      <title>t</title><link>https://example.com</link><description>d</description>
      <item><title>JST</title><link>https://example.com/1</link>
        <pubDate>Sat, 08 Aug 2026 21:54:30 +0900</pubDate></item>
      <item><title>GMT</title><link>https://example.com/2</link>
        <pubDate>Sat, 08 Aug 2026 12:54:30 GMT</pubDate></item>
    </channel></rss>"#;

    let feed = parse::parse(src.as_bytes()).unwrap();
    // +0900 の 21:54:30 と GMT の 12:54:30 は同じ瞬間
    assert_eq!(feed.items[0].published, feed.items[1].published);
    assert_eq!(
        feed.items[0].published.unwrap().to_rfc3339(),
        "2026-08-08T12:54:30+00:00"
    );

    // 出力を読み直しても瞬間が変わらない
    let reparsed = parse::parse(render::to_rss2(&feed).as_bytes()).unwrap();
    assert_eq!(reparsed.items[0].published, feed.items[0].published);
    assert!(render::to_rss2(&feed).contains("+0000"));
}
