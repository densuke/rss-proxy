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

/// 保存時に既定値を書き戻す機能の実体。壊れると DB に誤った設定が残る。
/// 種別が増えても自動で対象になるよう、カタログを総当たりする。
#[test]
fn every_kind_round_trips_its_parameters() {
    for info in catalog() {
        let built = build(info.kind, "").expect(info.kind);
        assert_eq!(built.name(), info.kind, "name がカタログと一致しない");

        let params = built.params();
        // 書き戻した内容がそのまま読み直せる
        let again = build(info.kind, &params).expect(info.kind);
        assert_eq!(again.params(), params, "{} の往復で値が変わる", info.kind);

        // 説明にある既定値が実際の既定値と一致する
        let parsed: serde_json::Value = serde_json::from_str(&params).unwrap();
        for p in info.params {
            let actual = parsed
                .get(p.name)
                .unwrap_or_else(|| panic!("{} に {} がない", info.kind, p.name));
            let actual = actual
                .as_str()
                .map(str::to_string)
                .unwrap_or(actual.to_string());
            assert_eq!(
                actual, p.default,
                "{} の {} の既定値が説明と違う",
                info.kind, p.name
            );
        }
    }
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
