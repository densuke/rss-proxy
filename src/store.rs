//! SQLite への永続化。CLI とサーバーの両方がこのモジュール経由で読み書きする。

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

/// 登録時に必要な項目。
pub struct NewFeed {
    /// 配信 URL に使う識別子。None なら乱数から作る
    pub slug: Option<String>,
    /// 表示名。None なら上流フィードのタイトルを使う
    pub label: Option<String>,
    pub url: String,
    pub interval_secs: i64,
}

#[derive(Debug, Clone)]
pub struct Feed {
    pub id: i64,
    /// 配信 URL とコマンドラインで使う識別子
    pub slug: String,
    /// 利用者が付けた表示名。未設定なら上流の title を使う
    pub label: Option<String>,
    pub url: String,
    pub title: Option<String>,
    pub interval_secs: i64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub next_fetch_at: i64,
    pub last_success_at: Option<i64>,
    pub last_error: Option<String>,
    pub fail_count: i64,
    /// 一度でも取得・処理に成功して配信できる状態か
    pub has_output: bool,
}

/// Processor 連鎖の 1 要素。(kind, params の JSON)
pub type ProcessorSpec = (String, String);

/// 初期スキーマ。以降の変更は MIGRATIONS で積み上げる。
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS feeds (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    url             TEXT NOT NULL,
    title           TEXT,
    interval_secs   INTEGER NOT NULL DEFAULT 900,
    etag            TEXT,
    last_modified   TEXT,
    next_fetch_at   INTEGER NOT NULL DEFAULT 0,
    last_success_at INTEGER,
    last_error      TEXT,
    fail_count      INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS processors (
    id       INTEGER PRIMARY KEY,
    feed_id  INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    kind     TEXT NOT NULL,
    params   TEXT NOT NULL,
    UNIQUE(feed_id, position)
);

-- 全フィードに適用する Processor。フィード固有の連鎖より先に走る
CREATE TABLE IF NOT EXISTS global_processors (
    id       INTEGER PRIMARY KEY,
    position INTEGER NOT NULL UNIQUE,
    kind     TEXT NOT NULL,
    params   TEXT NOT NULL
);

-- Processor が必要とする外部文書。URL が発表ごとに変わるものは取り直さない
CREATE TABLE IF NOT EXISTS documents (
    url        TEXT PRIMARY KEY,
    body       TEXT NOT NULL,
    fetched_at INTEGER NOT NULL
);

-- 有料記事の判定結果。1 記事につき 1 回だけ取りに行くため
CREATE TABLE IF NOT EXISTS paywall_cache (
    url        TEXT PRIMARY KEY,
    access     TEXT NOT NULL,   -- paid / free / unknown
    checked_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS outputs (
    feed_id      INTEGER PRIMARY KEY REFERENCES feeds(id) ON DELETE CASCADE,
    xml          TEXT NOT NULL,
    generated_at INTEGER NOT NULL
);
"#;

const FEED_COLUMNS: &str = "id, slug, label, url, title, interval_secs, etag, \
     last_modified, next_fetch_at, last_success_at, last_error, fail_count, \
     EXISTS(SELECT 1 FROM outputs o WHERE o.feed_id = feeds.id)";

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// スキーマを最新まで進める。`user_version` で適用済みの段数を持つ。
    pub fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(SCHEMA)?;

        let applied: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if applied < 1 {
            self.split_name_into_slug_and_label()?;
            self.conn.execute_batch("PRAGMA user_version = 1")?;
        }
        if applied < 2 {
            self.drop_unused_enabled_columns()?;
            self.conn.execute_batch("PRAGMA user_version = 2")?;
        }
        if applied < 3 {
            self.seed_global_processors()?;
            self.conn.execute_batch("PRAGMA user_version = 3")?;
        }
        Ok(())
    }

    /// name が URL とパスの両方を兼ねていた形からの移行。
    ///
    /// URL に使えていた名前はそのまま識別子にする。購読中の URL を壊さないため。
    /// 空白や日本語を含む名前は識別子をランダムに振り直し、元の名前は表示名に移す。
    fn split_name_into_slug_and_label(&self) -> Result<()> {
        let has_name = self
            .conn
            .prepare("SELECT 1 FROM pragma_table_info('feeds') WHERE name = 'name'")?
            .exists([])?;
        if !has_name {
            return Ok(());
        }

        self.conn.execute_batch(
            "ALTER TABLE feeds RENAME COLUMN name TO slug;
             ALTER TABLE feeds ADD COLUMN label TEXT;",
        )?;

        let rows: Vec<(i64, String)> = self
            .conn
            .prepare("SELECT id, slug FROM feeds")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>>>()?;

        for (id, slug) in rows {
            if crate::slug::is_valid(&slug) {
                continue;
            }
            self.conn.execute(
                "UPDATE feeds SET slug = ?2, label = ?3 WHERE id = ?1",
                params![id, crate::slug::generate(), slug],
            )?;
        }
        Ok(())
    }

    /// 登録し、実際に使われた slug を返す。
    pub fn add_feed(&self, feed: &NewFeed) -> Result<String> {
        let slug = feed.slug.clone().unwrap_or_else(crate::slug::generate);
        self.conn.execute(
            "INSERT INTO feeds (slug, label, url, interval_secs, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                slug,
                feed.label,
                feed.url,
                feed.interval_secs,
                Utc::now().timestamp()
            ],
        )?;
        Ok(slug)
    }

    pub fn feed_by_slug(&self, slug: &str) -> Result<Option<Feed>> {
        self.conn
            .query_row(
                &format!("SELECT {FEED_COLUMNS} FROM feeds WHERE slug = ?1"),
                [slug],
                row_to_feed,
            )
            .optional()
    }

    pub fn list_feeds(&self) -> Result<Vec<Feed>> {
        self.query_feeds(&format!("SELECT {FEED_COLUMNS} FROM feeds ORDER BY id"), [])
    }

    /// 巡回対象。有効かつ次回取得時刻を過ぎたもの。
    pub fn due_feeds(&self, now: i64) -> Result<Vec<Feed>> {
        self.query_feeds(
            &format!("SELECT {FEED_COLUMNS} FROM feeds WHERE next_fetch_at <= ?1 ORDER BY id"),
            [now],
        )
    }

    fn query_feeds<P: rusqlite::Params>(&self, sql: &str, p: P) -> Result<Vec<Feed>> {
        self.conn
            .prepare(sql)?
            .query_map(p, row_to_feed)?
            .collect::<Result<Vec<_>>>()
    }

    pub fn remove_feed(&self, slug: &str) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM feeds WHERE slug = ?1", [slug])?
            > 0)
    }

    /// 配信 URL と表示名を変更する。slug を変えると購読中の URL が変わる。
    pub fn rename(&self, id: i64, slug: &str, label: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE feeds SET slug = ?2, label = ?3 WHERE id = ?1",
            params![id, slug, label],
        )?;
        Ok(())
    }

    pub fn set_interval(&self, id: i64, interval_secs: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE feeds SET interval_secs = ?2 WHERE id = ?1",
            params![id, interval_secs],
        )?;
        Ok(())
    }

    pub fn set_enabled(&self, id: i64, enabled: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE feeds SET enabled = ?2 WHERE id = ?1",
            params![id, enabled],
        )?;
        Ok(())
    }

    pub fn set_title(&self, id: i64, title: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE feeds SET title = ?2 WHERE id = ?1",
            params![id, title],
        )?;
        Ok(())
    }

    /// グローバル連鎖の初期値。どのフィードでも効く整形と、広告記事の除去。
    ///
    /// 正規化を先に置く。［PR］ が [PR] になってから除外判定にかかるため。
    /// 不要なら画面か CLI から空にできる。
    fn seed_global_processors(&self) -> Result<()> {
        // 既存フィードは検証子を持っている。そのままだと 304 で処理がスキップされ、
        // 入れたばかりの連鎖が反映されない
        self.conn.execute_batch(
            "UPDATE feeds SET etag = NULL, last_modified = NULL, next_fetch_at = 0",
        )?;

        self.set_global_processors(&[
            // title だけを直すと、title と description を突き合わせる処理
            // (google_news_cluster など) が一致しなくなる。両方まとめて直す
            (
                "normalize_width".into(),
                r#"{"target":"both"}"#.into(),
            ),
            (
                "exclude".into(),
                // 広告であることが明示された表記だけを対象にする。
                // 「広告」単体は広告業界のニュースまで落とすので入れない
                r#"{"words":["【PR】","[PR]","PR:","【広告】","[広告]","<PR>","(PR)"],"target":"title"}"#
                    .into(),
            ),
        ])
    }

    /// 取得済みの外部文書。
    pub fn document_cached(&self, url: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT body FROM documents WHERE url = ?1", [url], |r| {
                r.get(0)
            })
            .optional()
    }

    pub fn remember_document(&self, url: &str, body: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO documents (url, body, fetched_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(url) DO UPDATE SET body = ?2, fetched_at = ?3",
            params![url, body, Utc::now().timestamp()],
        )?;
        Ok(())
    }

    pub fn prune_documents(&self, before: i64) -> Result<usize> {
        self.conn
            .execute("DELETE FROM documents WHERE fetched_at < ?1", [before])
    }

    /// 判定済みなら返す。無ければ取りに行く必要がある。
    pub fn paywall_cached(&self, url: &str) -> Result<Option<crate::paywall::Access>> {
        let access: Option<String> = self
            .conn
            .query_row(
                "SELECT access FROM paywall_cache WHERE url = ?1",
                [url],
                |r| r.get(0),
            )
            .optional()?;
        Ok(access.as_deref().map(crate::paywall::Access::parse))
    }

    pub fn remember_paywall(&self, url: &str, access: crate::paywall::Access) -> Result<()> {
        self.conn.execute(
            "INSERT INTO paywall_cache (url, access, checked_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(url) DO UPDATE SET access = ?2, checked_at = ?3",
            params![url, access.as_str(), Utc::now().timestamp()],
        )?;
        Ok(())
    }

    /// 配信から消えた記事の判定結果は使われない。古いものを捨てる。
    pub fn prune_paywall_cache(&self, before: i64) -> Result<usize> {
        self.conn
            .execute("DELETE FROM paywall_cache WHERE checked_at < ?1", [before])
    }

    /// 全フィードに適用する連鎖。フィード固有の連鎖より先に走る。
    pub fn global_processors(&self) -> Result<Vec<ProcessorSpec>> {
        self.conn
            .prepare("SELECT kind, params FROM global_processors ORDER BY position")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect()
    }

    pub fn set_global_processors(&self, chain: &[ProcessorSpec]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM global_processors", [])?;
        for (position, (kind, params)) in chain.iter().enumerate() {
            tx.execute(
                "INSERT INTO global_processors (position, kind, params) VALUES (?1, ?2, ?3)",
                params![position as i64, kind, params],
            )?;
        }
        tx.commit()
    }

    /// 有効/無効の切り替えは CLI にも画面にも導線がなく、false になる経路がなかった。
    /// 使う場面が出たら列ごと足し直す。
    fn drop_unused_enabled_columns(&self) -> Result<()> {
        for table in ["feeds", "processors"] {
            let exists = self
                .conn
                .prepare(&format!(
                    "SELECT 1 FROM pragma_table_info('{table}') WHERE name = 'enabled'"
                ))?
                .exists([])?;
            if exists {
                self.conn
                    .execute_batch(&format!("ALTER TABLE {table} DROP COLUMN enabled"))?;
            }
        }
        Ok(())
    }

    /// 条件付き GET の検証子を捨てる。
    ///
    /// 304 が返ると解析も Processor 適用も行われないため、設定を変えても
    /// 上流が変わるまで配信内容が古いままになる。設定変更時と即時取得時に
    /// これを呼び、次の取得で必ず作り直させる。
    pub fn clear_validators(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE feeds SET etag = NULL, last_modified = NULL WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    /// 次回取得時刻を直接指定する。「今すぐ取得」で 0 にして tick に拾わせる。
    pub fn set_next_fetch_at(&self, id: i64, at: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE feeds SET next_fetch_at = ?2 WHERE id = ?1",
            params![id, at],
        )?;
        Ok(())
    }

    pub fn mark_success(
        &self,
        id: i64,
        etag: Option<&str>,
        last_modified: Option<&str>,
        next_fetch_at: i64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE feeds SET etag = ?2, last_modified = ?3, next_fetch_at = ?4, \
             last_success_at = ?5, last_error = NULL, fail_count = 0 WHERE id = ?1",
            params![
                id,
                etag,
                last_modified,
                next_fetch_at,
                Utc::now().timestamp()
            ],
        )?;
        Ok(())
    }

    pub fn mark_failure(&self, id: i64, error: &str, next_fetch_at: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE feeds SET last_error = ?2, next_fetch_at = ?3, fail_count = fail_count + 1 \
             WHERE id = ?1",
            params![id, error, next_fetch_at],
        )?;
        Ok(())
    }

    pub fn processors(&self, feed_id: i64) -> Result<Vec<ProcessorSpec>> {
        self.conn
            .prepare("SELECT kind, params FROM processors WHERE feed_id = ?1 ORDER BY position")?
            .query_map([feed_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect()
    }

    /// 連鎖を丸ごと置き換える。順序は引数の並び。
    pub fn set_processors(&self, feed_id: i64, chain: &[ProcessorSpec]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM processors WHERE feed_id = ?1", [feed_id])?;
        for (position, (kind, params)) in chain.iter().enumerate() {
            tx.execute(
                "INSERT INTO processors (feed_id, position, kind, params) VALUES (?1, ?2, ?3, ?4)",
                params![feed_id, position as i64, kind, params],
            )?;
        }
        tx.commit()
    }

    pub fn set_output(&self, feed_id: i64, xml: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO outputs (feed_id, xml, generated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(feed_id) DO UPDATE SET xml = ?2, generated_at = ?3",
            params![feed_id, xml, Utc::now().timestamp()],
        )?;
        Ok(())
    }

    pub fn output(&self, slug: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT o.xml FROM outputs o JOIN feeds f ON f.id = o.feed_id WHERE f.slug = ?1",
                [slug],
                |r| r.get(0),
            )
            .optional()
    }
}

fn row_to_feed(row: &rusqlite::Row) -> Result<Feed> {
    Ok(Feed {
        id: row.get(0)?,
        slug: row.get(1)?,
        label: row.get(2)?,
        url: row.get(3)?,
        title: row.get(4)?,
        interval_secs: row.get(5)?,
        etag: row.get(6)?,
        last_modified: row.get(7)?,
        next_fetch_at: row.get(8)?,
        last_success_at: row.get(9)?,
        last_error: row.get(10)?,
        fail_count: row.get(11)?,
        has_output: row.get(12)?,
    })
}
