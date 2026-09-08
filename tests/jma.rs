use rss_proxy::proc::{Documents, Processor, build};
use rss_proxy::{model::Feed, parse};

const FEED: &str = include_str!("fixtures/jma/feed.xml");
const HYOGO: &str = include_str!("fixtures/jma/hyogo_vpww53.xml");
const HYOGO_URL: &str =
    "https://www.data.jma.go.jp/developer/xml/data/20260908031713_0_VPWW53_280000.xml";

fn feed() -> Feed {
    parse::parse(FEED.as_bytes()).unwrap()
}

fn proc(params: &str) -> Box<dyn Processor> {
    build("jma_warning", params).unwrap()
}

#[test]
fn only_the_comprehensive_document_of_the_matching_prefecture_is_requested() {
    let wants = proc(r#"{"areas":["神戸市"]}"#).wants(&feed());

    // 兵庫県 (280000) の VPWW53 だけ。VPWW54 は旧形式の重複、他県は対象外
    assert!(!wants.is_empty());
    for u in &wants {
        assert!(
            u.contains("_VPWW53_280000.xml"),
            "余計な文書を要求している: {u}"
        );
    }
    // 同じ予報区でも複数の発表があるので、最新のものだけを取る
    assert_eq!(wants.len(), 1, "{wants:?}");
    assert_eq!(wants[0], HYOGO_URL);
}

#[test]
fn multiple_municipalities_across_prefectures_are_resolved() {
    let wants = proc(r#"{"areas":["神戸市","佐倉市","名古屋市"]}"#).wants(&feed());
    let codes: Vec<&str> = ["280000", "120000", "230000"].into();

    assert_eq!(wants.len(), 3, "県ごとに 1 件ずつ: {wants:?}");
    for code in codes {
        assert!(
            wants
                .iter()
                .any(|u| u.contains(&format!("_VPWW53_{code}.xml"))),
            "{code} の文書が要求されていない"
        );
    }
}

#[test]
fn an_unknown_municipality_requests_nothing() {
    assert!(proc(r#"{"areas":["架空市"]}"#).wants(&feed()).is_empty());
    assert!(proc(r#"{"areas":[]}"#).wants(&feed()).is_empty());
}

#[test]
fn the_warnings_of_the_target_municipality_become_the_description() {
    let mut docs = Documents::empty();
    docs.insert(HYOGO_URL.into(), HYOGO.into());

    let out = proc(r#"{"areas":["神戸市"]}"#)
        .apply(feed(), &docs)
        .unwrap();

    assert_eq!(out.items.len(), 1, "該当する 1 件だけが残る");
    let item = &out.items[0];
    let body = item.description.as_deref().unwrap();
    assert!(
        body.contains("神戸市東灘区: 大雨注意報, 雷注意報"),
        "{body}"
    );
    assert!(body.contains("神戸市西区"), "9 区すべて出る");
    assert!(!body.contains("姫路市"), "対象外の市町村が混ざっている");
    assert!(item.title.as_deref().unwrap().contains("兵庫県"));
}

#[test]
fn a_sub_area_can_be_named_directly() {
    let mut docs = Documents::empty();
    docs.insert(HYOGO_URL.into(), HYOGO.into());

    let out = proc(r#"{"areas":["神戸市北区"]}"#)
        .apply(feed(), &docs)
        .unwrap();
    let body = out.items[0].description.as_deref().unwrap();
    assert!(body.contains("神戸市北区"));
    assert!(!body.contains("神戸市西区"), "前方一致が広がりすぎている");
}

#[test]
fn kinds_can_be_narrowed() {
    let mut docs = Documents::empty();
    docs.insert(HYOGO_URL.into(), HYOGO.into());

    let out = proc(r#"{"areas":["神戸市"],"kinds":["大雨"]}"#)
        .apply(feed(), &docs)
        .unwrap();
    let body = out.items[0].description.as_deref().unwrap();
    assert!(body.contains("大雨注意報"));
    assert!(!body.contains("雷注意報"), "絞り込みが効いていない");

    // 神戸市に警報は出ていないので、該当なしとして item ごと消える
    let out = proc(r#"{"areas":["神戸市"],"kinds":["警報"]}"#)
        .apply(feed(), &docs)
        .unwrap();
    assert!(out.items.is_empty(), "該当なしの item が残っている");
}

#[test]
fn items_without_their_document_are_dropped() {
    // まだ取得できていない文書の item は、次の巡回まで出さない
    let out = proc(r#"{"areas":["神戸市"]}"#)
        .apply(feed(), &Documents::empty())
        .unwrap();
    assert!(out.items.is_empty());
}
