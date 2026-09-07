use chrono::FixedOffset;
use rss_proxy::web::ui::{format_time, offset_label};

fn jst() -> FixedOffset {
    FixedOffset::east_opt(9 * 3600).unwrap()
}

#[test]
fn renders_unix_time_in_the_given_offset() {
    // 2026-09-07T00:00:00Z = 2026-09-07 09:00 JST
    assert_eq!(format_time(1_788_739_200, jst()), "2026-09-07 09:00");

    let utc = FixedOffset::east_opt(0).unwrap();
    assert_eq!(format_time(1_788_739_200, utc), "2026-09-07 00:00");
}

/// オフセットは値ごとではなく見出しに 1 度だけ出す。
#[test]
fn the_offset_is_shown_as_a_label() {
    assert_eq!(offset_label(jst()), "+09:00");
    assert_eq!(offset_label(FixedOffset::east_opt(0).unwrap()), "+00:00");
    assert_eq!(
        offset_label(FixedOffset::west_opt(5 * 3600).unwrap()),
        "-05:00"
    );
}

#[test]
fn handles_dates_before_the_epoch() {
    assert!(format_time(-1, jst()).starts_with("1970-01-01"));
}
