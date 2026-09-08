use rss_proxy::paywall::{Access, detect, rule_for};

const YOMIURI_PAID: &str = include_str!("fixtures/paywall/yomiuri_paid.html");
const YOMIURI_FREE: &str = include_str!("fixtures/paywall/yomiuri_free.html");
const NIKKEI_PAID: &str = include_str!("fixtures/paywall/nikkei_paid.html");
const NIKKEI_FREE: &str = include_str!("fixtures/paywall/nikkei_free.html");

#[test]
fn a_rule_is_chosen_by_the_host_of_the_link() {
    assert!(rule_for("https://www.yomiuri.co.jp/national/20260908-XYZ/").is_some());
    assert!(rule_for("https://www.nikkei.com/article/DGXZQ.../").is_some());
    // ルールのない媒体は判定しない。取りに行かせないため
    assert!(rule_for("https://www.publickey1.jp/blog/26/x.html").is_none());
    assert!(rule_for("https://news.web.nhk/newsweb/").is_none());
    assert!(rule_for("not a url").is_none());
}

#[test]
fn subdomains_of_a_known_publisher_are_covered() {
    assert!(rule_for("https://yomiuri.co.jp/x/").is_some());
    assert!(rule_for("https://www.yomiuri.co.jp/x/").is_some());
    // 別ドメインを巻き込まない
    assert!(rule_for("https://notyomiuri.co.jp/x/").is_none());
    assert!(rule_for("https://yomiuri.co.jp.evil.example/x/").is_none());
}

#[test]
fn yomiuri_is_classified_by_its_structured_data() {
    let rule = rule_for("https://www.yomiuri.co.jp/national/20260908-XYZ/").unwrap();
    assert_eq!(detect(rule, YOMIURI_PAID), Access::Paid);
    assert_eq!(detect(rule, YOMIURI_FREE), Access::Free);
}

#[test]
fn nikkei_is_classified_by_its_lock_flag() {
    let rule = rule_for("https://www.nikkei.com/article/DGXZQ/").unwrap();
    assert_eq!(detect(rule, NIKKEI_PAID), Access::Paid);
    assert_eq!(detect(rule, NIKKEI_FREE), Access::Free);
}

#[test]
fn a_page_without_the_marker_is_unknown() {
    // 目印が無い記事は「無料」と決めつけない。取りこぼすため
    let rule = rule_for("https://www.yomiuri.co.jp/x/").unwrap();
    assert_eq!(
        detect(rule, "<html><body>本文だけ</body></html>"),
        Access::Unknown
    );
    assert_eq!(detect(rule, ""), Access::Unknown);

    let rule = rule_for("https://www.nikkei.com/x/").unwrap();
    assert_eq!(detect(rule, "<html></html>"), Access::Unknown);
}

/// 記事ページには関連記事の一覧が載る。そこに有料記事が並んでいても、
/// その記事自身の判定に影響してはいけない。
#[test]
fn markers_from_related_articles_do_not_leak() {
    let rule = rule_for("https://www.yomiuri.co.jp/x/").unwrap();
    let page = format!(
        r#"<div data-icon-type="key-locked">会員限定</div>{YOMIURI_FREE}<div data-icon-type="key-locked">会員限定</div>"#
    );
    assert_eq!(
        detect(rule, &page),
        Access::Free,
        "サイドバーに引きずられている"
    );
}
