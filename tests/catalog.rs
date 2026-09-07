use rss_proxy::proc::{build, catalog};

#[test]
fn every_kind_has_help_text() {
    let kinds: Vec<&str> = catalog().iter().map(|p| p.kind).collect();
    assert!(kinds.contains(&"google_news_cluster"));
    assert!(kinds.contains(&"dedupe"));

    for info in catalog() {
        assert!(!info.summary.is_empty(), "{} に説明がない", info.kind);
        // 登録されている種別はすべて既定パラメータで組み立てられる
        assert!(
            build(info.kind, "").is_ok(),
            "{} を組み立てられない",
            info.kind
        );
    }
}

#[test]
fn documented_parameters_describe_their_default() {
    let dedupe = catalog().iter().find(|p| p.kind == "dedupe").unwrap();
    let key = dedupe.params.iter().find(|p| p.name == "key").unwrap();

    assert!(!key.description.is_empty());
    assert_eq!(key.default, "link");
    assert!(key.values.contains(&"guid"));
    assert!(key.values.contains(&"normalized_title"));
}

#[test]
fn a_processor_without_parameters_says_so() {
    let cluster = catalog()
        .iter()
        .find(|p| p.kind == "google_news_cluster")
        .unwrap();
    assert!(cluster.params.is_empty());
}

#[test]
fn built_processors_report_their_effective_parameters() {
    // 省略されたパラメータは既定値で埋めて返す
    assert_eq!(build("dedupe", "").unwrap().params(), r#"{"key":"link"}"#);
    assert_eq!(build("dedupe", "{}").unwrap().params(), r#"{"key":"link"}"#);
    assert_eq!(
        build("dedupe", r#"{"key":"guid"}"#).unwrap().params(),
        r#"{"key":"guid"}"#
    );
    assert_eq!(build("google_news_cluster", "").unwrap().params(), "{}");
}
