//! 公開アドレスの判定。
//!
//! 標準ライブラリの `is_global` は不安定なので、非公開の範囲を表で持つ。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// IPv4 の非公開範囲 (ネットワークアドレス, プレフィックス長)。
const V4_DENIED: &[([u8; 4], u32)] = &[
    ([0, 0, 0, 0], 8),
    ([10, 0, 0, 0], 8),
    ([100, 64, 0, 0], 10),
    ([127, 0, 0, 0], 8),
    ([169, 254, 0, 0], 16),
    ([172, 16, 0, 0], 12),
    ([192, 0, 0, 0], 24),
    ([192, 0, 2, 0], 24),
    ([192, 168, 0, 0], 16),
    ([198, 18, 0, 0], 15),
    ([198, 51, 100, 0], 24),
    ([203, 0, 113, 0], 24),
    ([224, 0, 0, 0], 4),
    ([240, 0, 0, 0], 4),
];

/// IPv6 の非公開範囲。IPv4 射影と 6to4 は埋め込まれた IPv4 で判定するので、ここには含めない。
const V6_DENIED: &[([u16; 8], u32)] = &[
    ([0, 0, 0, 0, 0, 0, 0, 0], 96),           // ::/96 (:: と ::1 を含む)
    ([0x64, 0xff9b, 0, 0, 0, 0, 0, 0], 96),   // NAT64
    ([0x64, 0xff9b, 1, 0, 0, 0, 0, 0], 48),   // NAT64 (ローカル用)
    ([0, 0, 0, 0, 0xffff, 0, 0, 0], 96),      // SIIT
    ([0x2001, 0, 0, 0, 0, 0, 0, 0], 23),      // Teredo、ベンチマーク、ORCHID を含む
    ([0x2001, 0x0db8, 0, 0, 0, 0, 0, 0], 32), // ドキュメント用
    ([0x3fff, 0, 0, 0, 0, 0, 0, 0], 20),      // ドキュメント用 (RFC 9637)
    ([0x100, 0, 0, 0, 0, 0, 0, 0], 64),       // 破棄用
    ([0xfc00, 0, 0, 0, 0, 0, 0, 0], 7),       // ユニークローカル
    ([0xfe80, 0, 0, 0, 0, 0, 0, 0], 10),      // リンクローカル
    ([0xfec0, 0, 0, 0, 0, 0, 0, 0], 10),      // サイトローカル (廃止済み)
    ([0xff00, 0, 0, 0, 0, 0, 0, 0], 8),       // マルチキャスト
];

/// 公開アドレスなら true。
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let addr = u128::from(u32::from(ip));
    !V4_DENIED
        .iter()
        .any(|&(net, prefix)| in_net(addr, u128::from(u32::from(Ipv4Addr::from(net))), prefix, 32))
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let seg = ip.segments();
    // IPv4 射影 (::ffff:a.b.c.d) は埋め込まれた IPv4 で判定する
    if seg[..5] == [0; 5] && seg[5] == 0xffff {
        return is_public_v4(embedded_v4(seg[6], seg[7]));
    }
    // 6to4 (2002:aabb:ccdd::/48) は埋め込まれた IPv4 で判定する
    if seg[0] == 0x2002 {
        return is_public_v4(embedded_v4(seg[1], seg[2]));
    }
    let addr = u128::from(ip);
    !V6_DENIED
        .iter()
        .any(|&(net, prefix)| in_net(addr, u128::from(Ipv6Addr::from(net)), prefix, 128))
}

fn embedded_v4(high: u16, low: u16) -> Ipv4Addr {
    let [a, b] = high.to_be_bytes();
    let [c, d] = low.to_be_bytes();
    Ipv4Addr::new(a, b, c, d)
}

/// `addr` が `net/prefix` に含まれるか。`width` はアドレスのビット幅。
fn in_net(addr: u128, net: u128, prefix: u32, width: u32) -> bool {
    let shift = width - prefix;
    addr >> shift == net >> shift
}
