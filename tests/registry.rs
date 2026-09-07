use rss_proxy::proc::build;

#[test]
fn builds_known_processors_from_stored_params() {
    assert_eq!(
        build("google_news_cluster", "{}").unwrap().name(),
        "google_news_cluster"
    );
    assert_eq!(
        build("dedupe", r#"{"key":"guid"}"#).unwrap().name(),
        "dedupe"
    );
}

#[test]
fn empty_params_fall_back_to_defaults() {
    assert!(build("dedupe", "").is_ok());
    assert!(build("dedupe", "{}").is_ok());
    assert!(build("google_news_cluster", "").is_ok());
}

#[test]
fn rejects_unknown_kind_and_bad_params() {
    assert!(build("nope", "{}").is_err());
    assert!(build("dedupe", r#"{"key":"telepathy"}"#).is_err());
    assert!(build("dedupe", "{not json").is_err());
}
