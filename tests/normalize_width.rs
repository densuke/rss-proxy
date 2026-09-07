use rss_proxy::model::{Feed, Item};
use rss_proxy::proc::Processor;
use rss_proxy::proc::normalize_width::{NormalizeWidth, Target};

fn item(title: &str, description: &str) -> Item {
    Item {
        id: None,
        title: Some(title.into()),
        link: None,
        description: Some(description.into()),
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

fn apply(target: Target, title: &str, description: &str) -> Item {
    NormalizeWidth { target }
        .apply(feed(vec![item(title, description)]))
        .unwrap()
        .items
        .pop()
        .unwrap()
}

#[test]
fn converts_full_width_digits_and_letters() {
    let out = apply(
        Target::Title,
        "岐阜のケーキ店３人死亡火災 レベル５ ＡＢＥＭＡ",
        "",
    );
    assert_eq!(
        out.title.as_deref(),
        Some("岐阜のケーキ店3人死亡火災 レベル5 ABEMA")
    );
}

#[test]
fn converts_full_width_symbols() {
    let out = apply(Target::Title, "（９月６日　１８時００分発表）！？＆＃", "");
    assert_eq!(out.title.as_deref(), Some("(9月6日　18時00分発表)!?&#"));
}

#[test]
fn leaves_japanese_punctuation_alone() {
    // カギ括弧・句読点・なかてん・波ダッシュは日本語の記号であって全角英数字ではない
    let source = "「助けてくれ」と叫び声。『引用』、中黒・波ダッシュ〜";
    let out = apply(Target::Title, source, "");
    assert_eq!(out.title.as_deref(), Some(source));
}

#[test]
fn leaves_the_full_width_tilde_alone() {
    // ～ (U+FF5E) は変換範囲に入るが、日本語では区間を表す記号として使われる
    let out = apply(Target::Title, "９時～１８時", "");
    assert_eq!(out.title.as_deref(), Some("9時～18時"));
}

#[test]
fn description_is_only_touched_when_asked() {
    let out = apply(Target::Title, "見出し", "本文の１２３");
    assert_eq!(out.description.as_deref(), Some("本文の１２３"));

    let out = apply(Target::Both, "見出し", "本文の１２３");
    assert_eq!(out.description.as_deref(), Some("本文の123"));
}

#[test]
fn missing_fields_do_not_panic() {
    let mut bare = item("x", "y");
    bare.title = None;
    bare.description = None;
    let out = NormalizeWidth {
        target: Target::Both,
    }
    .apply(feed(vec![bare]))
    .unwrap();
    assert_eq!(out.items.len(), 1);
}
