//! 配信 URL とコマンドラインで使う識別子。
//!
//! 表示名とは分けている。表示名は日本語や空白を含んでよいが、URL に載せる識別子は
//! そのままでは扱えないため。
//!
//! 既定はランダム。URL を知っている人だけが読める状態になり、推測での発見を防げる。
//! ただし URL は RSS リーダーの同期先やプロキシのログにも残るので、これは軽い目隠しで
//! あって認証ではない。秘匿が必要なら配信側にも認証を足すことになる。

use base64::Engine;

const MIN: usize = 3;
const MAX: usize = 64;

/// 128 ビットの乱数から作る。base64url なので 22 文字。
pub fn generate() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("乱数を取得できません");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// URL エンコードなしでパスに載せられる形式か。
pub fn is_valid(slug: &str) -> bool {
    (MIN..=MAX).contains(&slug.len())
        && slug
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
