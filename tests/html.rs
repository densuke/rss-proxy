use rss_proxy::html::to_plain_text;

#[test]
fn strips_tags() {
    assert_eq!(to_plain_text("<p>本文の<b>要約</b></p>"), "本文の 要約");
}

#[test]
fn decodes_entities_that_appear_in_real_feeds() {
    assert_eq!(to_plain_text("A&nbsp;&nbsp;B"), "A B");
    assert_eq!(to_plain_text("&lt;script&gt;"), "<script>");
    assert_eq!(to_plain_text("a &amp; b"), "a & b");
    assert_eq!(to_plain_text("&quot;q&quot; &#39;s&#39;"), "\"q\" 's'");
}

#[test]
fn collapses_whitespace_including_full_width_and_nbsp() {
    assert_eq!(to_plain_text("  a \u{a0} b\u{3000}c\n\nd  "), "a b c d");
}

#[test]
fn handles_empty_input() {
    assert_eq!(to_plain_text(""), "");
    assert_eq!(to_plain_text("<br>"), "");
}
