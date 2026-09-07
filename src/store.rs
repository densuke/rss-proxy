//! SQLite への永続化。CLI とサーバーの両方がこのモジュール経由で読み書きする。

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

/// 登録時に必要な項目。
pub struct NewFeed {
    pub name: String,
    pub url: String,
    pub interval_secs: i64,
}

#[derive(Debug, Clone)]
pub struct Feed {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub title: Option<String>,
    pub interval_secs: i64,
    pub enabled: bool,
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

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS feeds (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    url             TEXT NOT NULL,
    title           TEXT,
    interval_secs   INTEGER NOT NULL DEFAULT 900,
    enabled         INTEGER NOT NULL DEFAULT 1,
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
    enabled  INTEGER NOT NULL DEFAULT 1,
    UNIQUE(feed_id, position)
);

CREATE TABLE IF NOT EXISTS outputs (
    feed_id      INTEGER PRIMARY KEY REFERENCES feeds(id) ON DELETE CASCADE,
    xml          TEXT NOT NULL,
    generated_at INTEGER NOT NULL
);
"#;

const FEED_COLUMNS: &str = "id, name, url, title, interval_secs, enabled, etag, last_modified, \
     next_fetch_at, last_success_at, last_error, fail_count, \
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

    pub fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(SCHEMA)
    }

    pub fn add_feed(&self, feed: &NewFeed) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO feeds (name, url, interval_secs, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                feed.name,
                feed.url,
                feed.interval_secs,
                Utc::now().timestamp()
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn feed_by_name(&self, name: &str) -> Result<Option<Feed>> {
        self.conn
            .query_row(
                &format!("SELECT {FEED_COLUMNS} FROM feeds WHERE name = ?1"),
                [name],
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
            &format!(
                "SELECT {FEED_COLUMNS} FROM feeds \
                 WHERE enabled = 1 AND next_fetch_at <= ?1 ORDER BY id"
            ),
            [now],
        )
    }

    fn query_feeds<P: rusqlite::Params>(&self, sql: &str, p: P) -> Result<Vec<Feed>> {
        self.conn
            .prepare(sql)?
            .query_map(p, row_to_feed)?
            .collect::<Result<Vec<_>>>()
    }

    pub fn remove_feed(&self, name: &str) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM feeds WHERE name = ?1", [name])?
            > 0)
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
            .prepare(
                "SELECT kind, params FROM processors \
                 WHERE feed_id = ?1 AND enabled = 1 ORDER BY position",
            )?
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

    pub fn output(&self, name: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT o.xml FROM outputs o JOIN feeds f ON f.id = o.feed_id WHERE f.name = ?1",
                [name],
                |r| r.get(0),
            )
            .optional()
    }
}

fn row_to_feed(row: &rusqlite::Row) -> Result<Feed> {
    Ok(Feed {
        id: row.get(0)?,
        name: row.get(1)?,
        url: row.get(2)?,
        title: row.get(3)?,
        interval_secs: row.get(4)?,
        enabled: row.get(5)?,
        etag: row.get(6)?,
        last_modified: row.get(7)?,
        next_fetch_at: row.get(8)?,
        last_success_at: row.get(9)?,
        last_error: row.get(10)?,
        fail_count: row.get(11)?,
        has_output: row.get(12)?,
    })
}
