//! 配信 URL の一覧を OPML 2.0 で書き出す。
//!
//! 識別子は省略すると乱数から作られるため、RSS リーダーに登録するには
//! 管理画面から 1 つずつ写すことになる。OPML はほとんどのリーダーが
//! 取り込みに対応しているので、1 ファイルで一括登録できるようにする。

use crate::store::Feed;

/// `base_url` は配信の起点 (`https://rss.example.com` など)。末尾のスラッシュは足しても足さなくてもよい。
pub fn render(feeds: &[Feed], base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');

    // 別の環境で組み直しても同じ並びになるように。config export と揃える
    let mut feeds: Vec<&Feed> = feeds.iter().collect();
    feeds.sort_by(|a, b| a.slug.cmp(&b.slug));

    let outlines: String = feeds
        .iter()
        .map(|feed| {
            let name = escape(display_name(feed));
            // 起点も外部から来る (CLI の引数、HTTP の Host ヘッダ)。
            // 組み立ててから 1 回だけ通す。先に通すと二重にエスケープされる
            let url = escape(&format!("{base}/feeds/{slug}", slug = feed.slug));
            format!(
                "    <outline type=\"rss\" text=\"{name}\" title=\"{name}\" \
                 xmlUrl=\"{url}\"/>\n"
            )
        })
        .collect();

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <opml version=\"2.0\">\n\
         \x20 <head>\n\
         \x20   <title>rss-proxy</title>\n\
         \x20 </head>\n\
         \x20 <body>\n\
         {outlines}\
         \x20 </body>\n\
         </opml>\n"
    )
}

/// リーダーの一覧に出る名前。空欄だと見分けられないので識別子で代用する。
fn display_name(feed: &Feed) -> &str {
    feed.label
        .as_deref()
        .or(feed.title.as_deref())
        .unwrap_or(&feed.slug)
}

/// 属性値に入れる前に必ず通す。表示名も上流のタイトルも外部の入力で、
/// 引用符を含んでいれば属性を閉じて OPML を壊せる。
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
