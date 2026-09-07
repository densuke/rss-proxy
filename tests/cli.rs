use rss_proxy::cli::{Command, FeedCmd, ProcCmd, run};
use rss_proxy::store::Store;

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

fn add(s: &Store, name: &str) {
    run(
        s,
        Command::Feed(FeedCmd::Add {
            name: name.into(),
            url: format!("https://example.com/{name}.xml"),
            interval: 900,
        }),
    )
    .unwrap();
}

#[test]
fn adds_lists_and_removes_feeds() {
    let s = store();
    add(&s, "news");

    let listed = run(&s, Command::Feed(FeedCmd::List)).unwrap();
    assert!(listed.contains("news"));
    assert!(listed.contains("https://example.com/news.xml"));

    assert!(
        run(
            &s,
            Command::Feed(FeedCmd::Rm {
                name: "news".into()
            })
        )
        .is_ok()
    );
    assert!(
        !run(&s, Command::Feed(FeedCmd::List))
            .unwrap()
            .contains("news")
    );
}

#[test]
fn rejects_removing_a_missing_feed() {
    let s = store();
    assert!(
        run(
            &s,
            Command::Feed(FeedCmd::Rm {
                name: "nope".into()
            })
        )
        .is_err()
    );
}

#[test]
fn shows_a_feed_with_its_processor_chain() {
    let s = store();
    add(&s, "news");
    run(
        &s,
        Command::Proc(ProcCmd::Attach {
            feed: "news".into(),
            kind: "google_news_cluster".into(),
            params: None,
            at: None,
        }),
    )
    .unwrap();

    let shown = run(
        &s,
        Command::Feed(FeedCmd::Show {
            name: "news".into(),
        }),
    )
    .unwrap();
    assert!(shown.contains("news"));
    assert!(shown.contains("google_news_cluster"));
}

#[test]
fn attaches_detaches_and_reorders_processors() {
    let s = store();
    add(&s, "news");
    let attach = |kind: &str, params: Option<&str>| {
        run(
            &s,
            Command::Proc(ProcCmd::Attach {
                feed: "news".into(),
                kind: kind.into(),
                params: params.map(str::to_string),
                at: None,
            }),
        )
    };
    attach("google_news_cluster", None).unwrap();
    attach("dedupe", Some(r#"{"key":"guid"}"#)).unwrap();

    let shown = run(
        &s,
        Command::Feed(FeedCmd::Show {
            name: "news".into(),
        }),
    )
    .unwrap();
    assert!(shown.find("google_news_cluster").unwrap() < shown.find("dedupe").unwrap());

    run(
        &s,
        Command::Proc(ProcCmd::Move {
            feed: "news".into(),
            from: 1,
            to: 0,
        }),
    )
    .unwrap();
    let shown = run(
        &s,
        Command::Feed(FeedCmd::Show {
            name: "news".into(),
        }),
    )
    .unwrap();
    assert!(shown.find("dedupe").unwrap() < shown.find("google_news_cluster").unwrap());

    run(
        &s,
        Command::Proc(ProcCmd::Detach {
            feed: "news".into(),
            position: 0,
        }),
    )
    .unwrap();
    let shown = run(
        &s,
        Command::Feed(FeedCmd::Show {
            name: "news".into(),
        }),
    )
    .unwrap();
    assert!(!shown.contains("dedupe"));
}

#[test]
fn rejects_unknown_processor_kinds_and_bad_params() {
    let s = store();
    add(&s, "news");
    let attach = |kind: &str, params: Option<&str>| {
        run(
            &s,
            Command::Proc(ProcCmd::Attach {
                feed: "news".into(),
                kind: kind.into(),
                params: params.map(str::to_string),
                at: None,
            }),
        )
    };
    assert!(
        attach("telepathy", None).is_err(),
        "未知の種別は登録できない"
    );
    assert!(
        attach("dedupe", Some(r#"{"key":"nope"}"#)).is_err(),
        "不正なパラメータは登録できない"
    );
}

#[test]
fn lists_available_processor_kinds() {
    let listed = run(&store(), Command::Proc(ProcCmd::List)).unwrap();
    assert!(listed.contains("google_news_cluster"));
    assert!(listed.contains("dedupe"));
}
