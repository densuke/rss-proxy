//! 管理画面の認証。
//!
//! 配信パス (`/feeds/*`) は URL を知っていれば誰でも読める。一方、管理画面は
//! フィードの登録と Processor の設定を書き換えられるため、保護が要る。
//!
//! 方式は HTTP Basic 認証。ブラウザ標準の仕組みだけで完結し、セッションの保存も
//! Cookie の属性も CSRF トークンも要らない。パスワードは argon2 のハッシュで保持する。

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use subtle::ConstantTimeEq;

pub const USER_ENV: &str = "RSS_PROXY_ADMIN_USER";
pub const HASH_ENV: &str = "RSS_PROXY_ADMIN_PASSWORD_HASH";

/// 管理画面の認証情報。両方揃っているときだけ構成される。
#[derive(Clone)]
pub struct Admin {
    user: String,
    password_hash: String,
}

impl Admin {
    pub fn new(user: Option<String>, password_hash: Option<String>) -> Option<Self> {
        let user = user?.trim().to_string();
        let password_hash = password_hash?.trim().to_string();
        (!user.is_empty() && !password_hash.is_empty()).then_some(Self {
            user,
            password_hash,
        })
    }

    /// 環境変数から読む。片方しか設定されていない場合は未設定として扱う。
    pub fn from_env() -> Option<Self> {
        Self::new(std::env::var(USER_ENV).ok(), std::env::var(HASH_ENV).ok())
    }

    pub fn authenticates(&self, user: &str, password: &str) -> bool {
        // ユーザー名は秘密ではないが、比較時間から推測されないようにしておく
        let user_ok: bool = user.as_bytes().ct_eq(self.user.as_bytes()).into();
        // ユーザー名が違っても検証は行う。応答時間で存在を判別されないため
        let password_ok = verify_password(&self.password_hash, password);
        user_ok && password_ok
    }
}

/// パスワードを argon2 でハッシュ化する。
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    // ソルトは argon2 が安全な乱数から生成する
    Ok(Argon2::default()
        .hash_password(password.as_bytes())?
        .to_string())
}

/// 保存されたハッシュと突き合わせる。ハッシュが壊れている場合は常に失敗させる。
///
/// ponytail: 検証は 1 リクエストごとに走り、argon2 の設計上 100ms 前後かかる。
/// 管理画面のアクセス頻度では問題にならず、総当たりへの抑止にもなる。
/// 気になるならセッション方式へ切り替える。
pub fn verify_password(hash: &str, password: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}
