use rss_proxy::fetch::is_internal_host;

/// 上流フィードはリダイレクト先を自由に指定できる。
/// 内部アドレスへ誘導されると、公開されていないサービスの内容を取りに行ってしまう。
#[test]
fn internal_destinations_are_recognised() {
    for host in [
        "localhost",
        "127.0.0.1",
        "127.1.2.3",
        "::1",
        "0.0.0.0",
        "169.254.169.254", // クラウドのメタデータ
        "10.0.0.1",
        "192.168.1.1",
        "172.16.0.1",
        "172.31.255.255",
        "[::1]",
        "fd00::1",
    ] {
        assert!(is_internal_host(host), "{host} を内部と判定していない");
    }
}

#[test]
fn public_destinations_are_allowed() {
    for host in [
        "news.google.com",
        "www.publickey1.jp",
        "8.8.8.8",
        "172.32.0.1", // 172.16/12 の外
        "11.0.0.1",
        "192.169.0.1", // 192.168/16 の外
    ] {
        assert!(!is_internal_host(host), "{host} を誤って内部と判定している");
    }
}
