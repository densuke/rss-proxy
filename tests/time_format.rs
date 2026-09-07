use chrono::FixedOffset;
use rss_proxy::web::ui::format_time;

fn jst() -> FixedOffset {
    FixedOffset::east_opt(9 * 3600).unwrap()
}

#[test]
fn renders_unix_time_in_the_given_offset() {
    // 2026-09-07T00:00:00Z = 2026-09-07 09:00 JST
    assert_eq!(format_time(1_788_739_200, jst()), "2026-09-07 09:00 +09:00");
}

#[test]
fn the_offset_is_shown_so_the_zone_is_never_ambiguous() {
    let utc = FixedOffset::east_opt(0).unwrap();
    assert_eq!(format_time(1_788_739_200, utc), "2026-09-07 00:00 +00:00");
}

#[test]
fn handles_dates_before_the_epoch() {
    assert!(format_time(-1, jst()).starts_with("1970-01-01"));
}
