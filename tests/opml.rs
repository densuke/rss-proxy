//! 配信 URL の一覧を OPML で書き出す。識別子は乱数なので手で写すのが辛い。

use rss_proxy::opml;
use rss_proxy::store::{NewFeed, Store};

fn store_with(feeds: &[(&str, Option<&str>)]) -> Store {
    let store = Store::open_in_memory().unwrap();
    for (slug, label) in feeds {
        store
            .add_feed(&NewFeed {
                slug: Some((*slug).into()),
                label: label.map(Into::into),
                url: format!("https://example.com/{slug}.xml"),
                interval_secs: 900,
            })
            .unwrap();
    }
    store
}

/// XML として最後まで読み切れるか。
fn parses(xml: &str) -> bool {
    let mut reader = quick_xml::Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Eof) => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
}

#[test]
fn lists_every_feed_with_its_delivery_url() {
    let store = store_with(&[("gnews", Some("ニュース")), ("nhk", None)]);
    let xml = opml::render(&store.list_feeds().unwrap(), "https://rss.example.com");

    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    assert!(xml.contains(r#"<opml version="2.0">"#));
    assert!(
        xml.contains(r#"xmlUrl="https://rss.example.com/feeds/gnews""#),
        "配信 URL が出ていない: {xml}"
    );
    assert!(xml.contains(r#"xmlUrl="https://rss.example.com/feeds/nhk""#));

    // 表示名があればそれを使う
    assert!(xml.contains(r#"text="ニュース""#));
    // なければ識別子で代用する。空欄ではリーダー側で見分けられない
    assert!(xml.contains(r#"text="nhk""#));
}

/// 末尾のスラッシュを重ねない。利用者がどちらで書くかは決められない。
#[test]
fn accepts_a_base_url_with_a_trailing_slash() {
    let store = store_with(&[("gnews", None)]);
    let xml = opml::render(&store.list_feeds().unwrap(), "https://rss.example.com/");
    assert!(
        xml.contains(r#"xmlUrl="https://rss.example.com/feeds/gnews""#),
        "{xml}"
    );
}

/// 表示名も上流のタイトルも外部の入力。属性を閉じられると OPML が壊れる。
#[test]
fn escapes_values_that_go_into_attributes() {
    let store = store_with(&[("gnews", Some(r#"A & B <"quoted">"#))]);
    let xml = opml::render(&store.list_feeds().unwrap(), "https://rss.example.com");

    assert!(!xml.contains(r#"<"quoted">"#), "生の値が出ている: {xml}");
    assert!(xml.contains("&amp;") && xml.contains("&lt;") && xml.contains("&quot;"));
    assert!(parses(&xml), "XML として読めない: {xml}");
}

/// 1 件も登録がなくても壊れた XML にしない。
#[test]
fn renders_an_empty_list() {
    let xml = opml::render(&[], "https://rss.example.com");
    assert!(parses(&xml), "{xml}");
}

/// 差分を見るために出すので並びを決める。config export と揃える。
#[test]
fn lists_feeds_in_slug_order() {
    let store = store_with(&[("zzz", None), ("aaa", None)]);
    let xml = opml::render(&store.list_feeds().unwrap(), "https://rss.example.com");
    let first = xml.find("aaa").unwrap();
    let second = xml.find("zzz").unwrap();
    assert!(first < second, "識別子順になっていない: {xml}");
}

/// 起点も外部から来る。CLI の引数にも HTTP の Host ヘッダにも任意の文字列が入る。
#[test]
fn escapes_the_base_url_too() {
    let store = store_with(&[("gnews", None)]);
    let xml = opml::render(&store.list_feeds().unwrap(), r#"http://x" bad="1"#);

    assert!(!xml.contains(r#"" bad="#), "属性を閉じられている: {xml}");
    assert!(parses(&xml), "XML として読めない: {xml}");
}
