use rss_proxy::cli::{Command, FeedCmd, ProcCmd, run};
use rss_proxy::store::Store;

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

fn add(s: &Store, slug: &str) {
    run(
        s,
        Command::Feed(FeedCmd::Add {
            url: format!("https://example.com/{slug}.xml"),
            slug: Some(slug.into()),
            label: None,
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
                slug: "news".into()
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
                slug: "nope".into()
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
            slug: "news".into(),
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
            slug: "news".into(),
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
            slug: "news".into(),
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
            slug: "news".into(),
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

#[test]
fn an_omitted_slug_becomes_a_random_one() {
    let s = store();
    let out = run(
        &s,
        Command::Feed(FeedCmd::Add {
            url: "https://example.com/x.xml".into(),
            slug: None,
            label: Some("ニュース".into()),
            interval: 900,
        }),
    )
    .unwrap();

    let slug = out.rsplit(' ').next().unwrap();
    assert!(rss_proxy::slug::is_valid(slug), "{slug}");
    assert_eq!(slug.len(), 22);

    let shown = run(&s, Command::Feed(FeedCmd::Show { slug: slug.into() })).unwrap();
    assert!(shown.contains("ニュース"), "表示名が保持される");
}

#[test]
fn rejects_a_slug_that_cannot_go_in_a_url() {
    let s = store();
    let err = run(
        &s,
        Command::Feed(FeedCmd::Add {
            url: "https://example.com/x.xml".into(),
            slug: Some("NHK 主要ニュース".into()),
            label: None,
            interval: 900,
        }),
    )
    .unwrap_err();
    assert!(err.to_string().contains("使えません"));
}

#[test]
fn renames_the_slug_and_label() {
    let s = store();
    add(&s, "old");

    run(
        &s,
        Command::Feed(FeedCmd::Set {
            slug: "old".into(),
            new_slug: Some("new".into()),
            label: Some("表示名".into()),
            interval: Some(300),
        }),
    )
    .unwrap();

    assert!(run(&s, Command::Feed(FeedCmd::Show { slug: "old".into() })).is_err());
    let shown = run(&s, Command::Feed(FeedCmd::Show { slug: "new".into() })).unwrap();
    assert!(shown.contains("表示名"));
    assert!(shown.contains("300s"));
}

#[test]
fn renaming_to_an_existing_slug_fails() {
    let s = store();
    add(&s, "one");
    add(&s, "two");

    assert!(
        run(
            &s,
            Command::Feed(FeedCmd::Set {
                slug: "one".into(),
                new_slug: Some("two".into()),
                label: None,
                interval: None,
            }),
        )
        .is_err()
    );
}

/// 設定を変えたのに次の巡回まで反映されないと「効いていない」ように見える。
/// 変更したら検証子を捨て、すぐ巡回対象にする。
#[test]
fn changing_the_configuration_schedules_an_immediate_refetch() {
    let s = store();
    add(&s, "news");
    let id = s.feed_by_slug("news").unwrap().unwrap().id;

    let park = |s: &Store| {
        s.mark_success(id, Some("W/\"1\""), Some("Mon"), 9_999_999_999)
            .unwrap()
    };
    let parked = |s: &Store| {
        let f = s.feed_by_slug("news").unwrap().unwrap();
        f.etag.is_some() || f.last_modified.is_some() || f.next_fetch_at != 0
    };

    // Processor の追加
    park(&s);
    run(
        &s,
        Command::Proc(ProcCmd::Attach {
            feed: "news".into(),
            kind: "dedupe".into(),
            params: None,
            at: None,
        }),
    )
    .unwrap();
    assert!(!parked(&s), "追加後は作り直しの対象になる");

    // Processor の削除
    park(&s);
    run(
        &s,
        Command::Proc(ProcCmd::Detach {
            feed: "news".into(),
            position: 0,
        }),
    )
    .unwrap();
    assert!(!parked(&s), "削除後は作り直しの対象になる");

    // 表示名の変更 (配信する title に使われる)
    park(&s);
    run(
        &s,
        Command::Feed(FeedCmd::Set {
            slug: "news".into(),
            new_slug: None,
            label: Some("表示名".into()),
            interval: None,
        }),
    )
    .unwrap();
    assert!(!parked(&s), "表示名の変更後は作り直しの対象になる");
}

#[test]
fn the_global_chain_has_its_own_subcommands() {
    use rss_proxy::cli::GlobalCmd;
    let s = store();

    let listed = run(&s, Command::Global(GlobalCmd::Show)).unwrap();
    assert!(listed.contains("normalize_width"), "既定の連鎖が出る");

    run(
        &s,
        Command::Global(GlobalCmd::Attach {
            kind: "dedupe".into(),
            params: None,
            at: Some(0),
        }),
    )
    .unwrap();
    assert!(
        run(&s, Command::Global(GlobalCmd::Show))
            .unwrap()
            .starts_with("0. dedupe"),
        "先頭に入る"
    );

    run(&s, Command::Global(GlobalCmd::Detach { position: 0 })).unwrap();
    assert!(
        !run(&s, Command::Global(GlobalCmd::Show))
            .unwrap()
            .contains("dedupe")
    );
}

/// グローバル連鎖を変えたら、全フィードを作り直しの対象にする。
#[test]
fn changing_the_global_chain_reschedules_every_feed() {
    use rss_proxy::cli::GlobalCmd;
    let s = store();
    add(&s, "one");
    add(&s, "two");
    for slug in ["one", "two"] {
        let id = s.feed_by_slug(slug).unwrap().unwrap().id;
        s.mark_success(id, Some("W/\"1\""), Some("Mon"), 9_999_999_999)
            .unwrap();
    }

    run(&s, Command::Global(GlobalCmd::Detach { position: 0 })).unwrap();

    for slug in ["one", "two"] {
        let f = s.feed_by_slug(slug).unwrap().unwrap();
        assert!(
            f.etag.is_none() && f.next_fetch_at == 0,
            "{slug} が作り直しの対象になっていない"
        );
    }
}
