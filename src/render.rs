use crate::model::Feed;

/// 内部モデルを RSS 2.0 の XML 文字列へ変換する。
pub fn to_rss2(feed: &Feed) -> String {
    let items = feed
        .items
        .iter()
        .map(|item| {
            rss::ItemBuilder::default()
                .title(item.title.clone())
                .link(item.link.clone())
                .description(item.description.clone())
                .guid(item.id.clone().map(|value| {
                    rss::GuidBuilder::default()
                        .value(value)
                        .permalink(false)
                        .build()
                }))
                .pub_date(item.published.map(|d| d.to_rfc2822()))
                .categories(
                    item.categories
                        .iter()
                        .map(|c| rss::CategoryBuilder::default().name(c.clone()).build())
                        .collect::<Vec<_>>(),
                )
                .build()
        })
        .collect::<Vec<_>>();

    rss::ChannelBuilder::default()
        // どのバージョンが処理した出力かを配信物自体に残す
        .generator(Some(
            concat!("rss-proxy ", env!("CARGO_PKG_VERSION")).to_string(),
        ))
        .title(feed.title.clone())
        .link(feed.link.clone().unwrap_or_default())
        .description(feed.description.clone().unwrap_or_default())
        .last_build_date(feed.updated.map(|d| d.to_rfc2822()))
        .items(items)
        .build()
        .to_string()
}
