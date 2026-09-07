# rss-proxy 設計書

RSS/Atom フィードを取得し、登録された処理を適用したうえで再配信するプロキシ。

本書は v1 の設計を記述する。v1 では「1 フィードを整形して再配信する」という中核機能のみを実装し、それ以外は 14 章に仕様だけを残して見送る。

## 1. 背景と目的

配信元のフィードには、そのままでは読みづらい構造上の問題があることが多い。代表例が Google ニュースのフィードで、1 つの item の中で `<title>` と `<description>` が実質同じ内容を持つ。

多くの RSS リーダーは title と description を両方描画するため、同じ文章が 2 回表示される。これは item 間の重複ではなく item 内の重複であり、一般的な重複除去では解決しない。

### 1.1 実フィードの調査結果

Google ニュースのトピックフィード (`https://news.google.com/rss/topics/...`) を実際に取得して確認したところ、description には 2 つの形式が混在していた (70 item 中、単一形式 7 件 / クラスタ形式 63 件)。

**単一形式** — description が title の複製でしかない。

```xml
<item>
  <title>JavaScriptをC言語にコンパイルする「porffor」がアルファ版に到達 - Publickey</title>
  <description>&lt;a href="..."&gt;JavaScriptをC言語にコンパイルする「porffor」がアルファ版に到達&lt;/a&gt;&amp;nbsp;&amp;nbsp;&lt;font color="#6f6f6f"&gt;Publickey&lt;/font&gt;</description>
</item>
```

**クラスタ形式** — description が関連記事のリストになっており、その先頭要素だけが title と重複する。

```xml
<description>
  &lt;ol&gt;
    &lt;li&gt;&lt;a href="..."&gt;岐阜のケーキ店３人死亡火災、屋根から…&lt;/a&gt;&amp;nbsp;&amp;nbsp;&lt;font color="#6f6f6f"&gt;読売新聞&lt;/font&gt;&lt;/li&gt;   <!-- title と同一 -->
    &lt;li&gt;&lt;a href="..."&gt;ケーキ店の火事 死亡した3人のうち…&lt;/a&gt;&amp;nbsp;&amp;nbsp;&lt;font color="#6f6f6f"&gt;Yahoo!ニュース&lt;/font&gt;&lt;/li&gt;  <!-- 別記事 -->
    ...
  &lt;/ol&gt;
</description>
```

クラスタ形式で description を丸ごと削除すると、2 件目以降の関連記事という有用な情報まで失われる。単純な「重複したら消す」では不十分。

各 item には `<source url="...">読売新聞</source>` があり、媒体名が独立して取得できる。title 末尾の `" - 媒体名"` はこの値と一致する。

### 1.2 この形式の一般性

他のフィード (Publickey / NHK ニュース / GIGAZINE / はてなブックマーク) を実測したところ、いずれも description は記事本文の要約であり、title の反復もクラスタ構造も存在しなかった。

`<ol><li>` によるクラスタ形式と `<font color="#6f6f6f">` のようなマークアップは、Google ニュースの生成器 (`NFE/5.0`) に固有のものである。汎用処理として一般化する根拠はない。サービス固有の処理として実装する。

## 2. 全体構成

```
                    ┌─────────────── rss-proxy (単一バイナリ) ───────────────┐
                    │                                                        │
上流フィード ──────>│  巡回スケジューラ → 取得 → パース → Processor 連鎖     │
 (RSS/Atom)         │                                    ↓                   │
                    │                                 SQLite                 │
                    │                                    ↓                   │
Caddy ─────────────>│  HTTP サーバー (axum)  ── /feeds/:name  配信           │
 (reverse_proxy)    │                        ── /api/*       管理 API        │
                    │                        ── /            Web UI          │
                    │                                                        │
CLI (同一バイナリ) ─┼──> SQLite 直接アクセス                                 │
                    └────────────────────────────────────────────────────────┘
```

- 常駐プロセスは 1 つ。スケジューラと HTTP サーバーは同一プロセス内の tokio タスク。
- CLI はサーバーを経由せず SQLite を直接読み書きする。サーバー未起動でも設定管理が可能。

## 3. 技術選定

| 項目 | 選定 | 理由 |
|------|------|------|
| 言語 | Rust (edition 2024) | 常駐デーモンで GC がなく、メモリ使用量が予測可能 |
| 非同期ランタイム | tokio | axum / reqwest との統合 |
| HTTP サーバー | axum | 軽量、tower ミドルウェアが使える |
| HTTP クライアント | reqwest | 条件付き GET、タイムアウト制御 |
| フィード解析 | feed-rs | RSS 0.9x/1.0/2.0 と Atom を単一 API で扱える |
| フィード出力 | rss | 出力は RSS 2.0 に統一 |
| ストレージ | SQLite (rusqlite, WAL モード) | 単一ファイル、Web UI と CLI の同時書き込みに耐える |
| CLI 引数解析 | clap (derive) | サブコマンド構成 |
| エラー型 | thiserror (lib) / anyhow (bin) | 標準的な組み合わせ |

crate のバージョンは実装着手時に最新安定版を確認して決定する。

musl による静的リンク (12.3 節) の制約から、以下は選択の余地がない。

- reqwest は `rustls-tls` を使う (既定の native-tls / OpenSSL は musl 静的ビルドでリンクできない)
- rusqlite は `bundled` フィーチャを使う (システムの libsqlite3 を静的リンクできない)

テンプレートエンジンは導入しない。Web UI は 2 画面のみで、`include_str!` と `format!` で足りる。

## 4. 内部データモデル

### 4.1 方針: 正規化して再生成する

```
上流XML → parse → 内部 Feed/Item モデル → Processor 連鎖 → 再シリアライズ(RSS 2.0)
```

元の XML DOM を保持したまま部分的に書き換える「ロスレス方式」は採用しない。名前空間や独自拡張要素の取り回しが複雑になり、Processor の実装も XML ノード操作に引きずられる。マイナーな拡張要素が出力から欠落することは許容する。

入力は RSS/Atom 両対応、出力は RSS 2.0 に固定する。

### 4.2 モデル定義

```rust
pub struct Feed {
    pub title: String,
    pub link: Option<String>,
    pub description: Option<String>,
    pub updated: Option<DateTime<Utc>>,
    pub items: Vec<Item>,
}

pub struct Item {
    pub id: Option<String>,          // guid
    pub title: Option<String>,
    pub link: Option<String>,
    pub description: Option<String>, // HTML を含みうる
    pub published: Option<DateTime<Utc>>,
    pub authors: Vec<String>,
    pub categories: Vec<String>,
}
```

RSS の `<source>` 要素はモデルに持たない。feed-rs の RSS2 パーサがこの要素を読まないうえ、Google ニュースの場合は description 内の各要素が `<a>見出し</a>&nbsp;&nbsp;<font>媒体名</font>` の形を取るため、媒体名は description から直接得られる。

## 5. Processor パイプライン

### 5.1 インターフェース

```rust
pub trait Processor: Send + Sync {
    fn name(&self) -> &'static str;
    fn apply(&self, feed: Feed) -> Result<Feed, ProcessorError>;
}
```

- 入出力とも所有権を受け渡す純粋な変換。副作用を持たない。
- フィードごとに「順序つきのチェーン」として設定する。同じ種類の Processor を異なるパラメータで複数回登録できる。
- 各インスタンスは型名 + JSON パラメータで永続化する。

```
feed "google-news-headline":
  1. google_news_cluster
  2. dedupe { key: "link" }
```

### 5.2 v1 で実装する Processor

実測で問題を確認できたものだけを実装する。trait とレジストリさえあれば追加は 1 ファイル 30 行程度なので、必要が生じてから足す。見送った Processor の仕様は 14.2 に残す。

#### google_news_cluster

Google ニュース固有。description のクラスタ構造を解釈し、title と重複する要素だけを取り除く。

```
1. description を項目単位に分解する
   - <ol><li>...</li></ol> 形式 → 各 li を 1 項目
   - 単一の <a>...</a> 形式     → 全体を 1 項目
2. 各項目のリンクテキストを正規化する (HTML 除去 / &nbsp; / 連続空白)
3. title を正規化する (末尾の " - 媒体名" を除去。媒体名は先頭要素の <font> から得る)
4. title と一致する項目を除去する
5. 残り 0 件  → description を空にする   (単一形式はここに落ちる)
   残りあり   → 残った項目だけで <ol> を再構築する
```

入力形式によって動作が自動的に決まるため、パラメータは持たない。

しきい値による類似判定は不要だった。fixture の 70 item すべてで、正規化後の title と先頭要素が完全一致する (実測)。曖昧一致は誤検出の余地を作るだけなので採用しない。

サービス固有の処理は `src/proc/vendor/` に置き、汎用 Processor と分ける。実装上は同じ trait であり、チェーン内では他と同列に並ぶ。抽象化レイヤーは追加しない。適用の自動判定も行わない (フィードを見て利用者が明示的に登録する)。

#### dedupe

item 間の重複除去。先に出現したものを残す。巡回のたびに同一 item が再出現するフィードへの保険として v1 に含める。

パラメータ:
- `key`: `guid` / `link` / `normalized_title`。既定 `link`

## 6. 取得とスケジューリング

### 6.1 背景巡回

オンデマンド取得 (配信要求時に上流へ取りに行く) は採用しない。アクセス集中時に上流への負荷が予測不能になるため。

- フィードごとに `interval_secs` を設定する。既定 900 秒 (15 分)。
- スケジューラは 60 秒間隔の tick ループを 1 本だけ持ち、毎 tick で SQLite から `next_fetch_at <= now` のフィードを取得して処理する。フィードごとにタスクを常駐させる方式は取らない (設定変更の反映と資源管理が煩雑になるため)。
- 1 tick 内の同時取得数は上限を設ける (既定 4)。

### 6.2 条件付き GET

- 前回取得時の `ETag` と `Last-Modified` を保存し、次回リクエストで `If-None-Match` / `If-Modified-Since` を送る。
- 304 が返った場合は解析と Processor 適用をスキップし、`next_fetch_at` だけ更新する。
- User-Agent は `rss-proxy/<version>` を明示する。

### 6.3 エラー処理

- 取得失敗時は直前の成功結果を保持し続ける。配信は止めない。
- 連続失敗回数を記録し、指数バックオフで `next_fetch_at` を後ろにずらす (上限 6 時間)。
- 最終成功時刻と直近のエラーを Web UI に表示する。

### 6.4 配信

`GET /feeds/:name` は SQLite に保存済みの処理後フィードをそのまま返す。上流には触らない。常に高速で、上流障害の影響を受けない。

## 7. ストレージ

SQLite 単一ファイル、WAL モード。既定パス `/var/lib/rss-proxy/rss-proxy.db`。

```sql
CREATE TABLE feeds (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,   -- URL パスに使う識別子
    url             TEXT NOT NULL,          -- 上流フィード URL
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

CREATE TABLE processors (
    id       INTEGER PRIMARY KEY,
    feed_id  INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,      -- 適用順
    kind     TEXT NOT NULL,         -- "google_news_cluster" 等
    params   TEXT NOT NULL,         -- JSON
    enabled  INTEGER NOT NULL DEFAULT 1,
    UNIQUE(feed_id, position)
);

CREATE TABLE outputs (
    feed_id      INTEGER PRIMARY KEY REFERENCES feeds(id) ON DELETE CASCADE,
    xml          TEXT NOT NULL,     -- 処理後の RSS 2.0
    generated_at INTEGER NOT NULL
);
```

処理後の XML をそのまま保存する。item 単位で正規化して保存する設計も考えられるが、配信のたびに再構築するコストが増えるだけで利点がない。

設定ファイル (TOML) には listen アドレスと DB パスのみを置く。フィードと Processor の設定はすべて DB に持つ。Web UI からの書き込みがあるため、設定を平文ファイルに分散させると競合する。

## 8. HTTP API と Web UI

### 8.1 エンドポイント

| メソッド | パス | 用途 |
|----------|------|------|
| GET | `/feeds/:name` | 処理済みフィードの配信 (`application/rss+xml`) |
| GET | `/api/feeds` | フィード一覧 |
| POST | `/api/feeds` | フィード登録 |
| GET | `/api/feeds/:id` | フィード詳細 (Processor 連鎖を含む) |
| PATCH | `/api/feeds/:id` | フィード更新 |
| DELETE | `/api/feeds/:id` | フィード削除 |
| POST | `/api/feeds/:id/fetch` | 即時取得 |
| PUT | `/api/feeds/:id/processors` | Processor 連鎖の一括更新 (順序込み) |
| GET | `/healthz` | ヘルスチェック |

### 8.2 Web UI

サーバーサイドで HTML を生成する。SPA フレームワークもテンプレートエンジンも使わない。

画面は 2 枚。

1. フィード一覧 — 名前、最終取得、状態、item 数、配信 URL
2. フィード編集 — URL、巡回間隔、Processor 連鎖の追加・削除・並べ替え

## 9. CLI

サーバーと同一バイナリ。サブコマンド構成。

```
rss-proxy serve [--config PATH]

rss-proxy feed add <name> <url> [--interval 900]
rss-proxy feed list
rss-proxy feed show <name>
rss-proxy feed rm <name>
rss-proxy feed fetch <name>          # 即時取得

rss-proxy proc list                  # 利用可能な Processor 種別
rss-proxy proc attach <feed> <kind> [--params '{"key":"link"}'] [--at N]
rss-proxy proc detach <feed> <position>
rss-proxy proc move <feed> <from> <to>

rss-proxy preview <name>             # 適用前後を標準出力に表示
```

CLI はサーバー API を経由せず SQLite に直接アクセスする。サーバーが停止していても設定を編集でき、初期セットアップやトラブル時の復旧が容易になる。WAL モードにより稼働中のサーバーとの同時アクセスも安全。

`preview` は Processor の効果を確認する手段として v1 に含める。Web UI 側のプレビュー画面は見送る (14.3)。

## 10. ディレクトリ構成

```
src/
├── main.rs            # エントリポイント、CLI ディスパッチ
├── config.rs          # 設定ファイル読み込み
├── model.rs           # Feed, Item
├── fetch.rs           # HTTP 取得、条件付き GET
├── parse.rs           # feed-rs → model 変換
├── render.rs          # model → RSS 2.0 XML
├── store/
│   ├── mod.rs
│   ├── schema.rs      # マイグレーション
│   └── queries.rs
├── proc/
│   ├── mod.rs         # trait Processor、レジストリ、チェーン実行
│   ├── dedupe.rs
│   └── vendor/
│       ├── mod.rs
│       └── google_news.rs   # google_news_cluster
├── scheduler.rs       # 巡回ループ
├── cli/
│   ├── mod.rs
│   ├── feed.rs
│   └── proc.rs
└── web/
    ├── mod.rs         # axum ルーター
    ├── api.rs
    ├── serve.rs       # /feeds/:name
    └── ui.rs          # HTML

tests/
├── fixtures/
│   └── google_news_headline.xml   # 2026-09-07 取得、70 item
└── integration.rs

.github/workflows/
├── ci.yml
└── release.yml
```

## 11. テスト方針

TDD を前提とする。仕様 (本書) → テスト作成 → 実装 の順で進める。カバレッジは 80% 以上を目標とし、cargo-tarpaulin で測定する。

### 11.1 Processor の単体テスト

Processor はすべて `Feed -> Feed` の純粋関数であり、単体テストの対象として理想的。

`tests/fixtures/google_news_headline.xml` を固定入力とする (2026-09-07 取得、70 item、うち単一形式 7 件 / クラスタ形式 63 件)。

`google_news_cluster` の検証:

- 単一形式の item は description が空になること
- クラスタ形式の item は先頭の重複要素だけが消え、2 件目以降が残ること
- 残った要素の順序とリンクが保たれること
- title は変更されないこと
- item 数が変化しないこと
- description が空 / `<ol>` でも `<a>` でもない item を渡しても壊れないこと
- title と無関係な description は変更されないこと

`dedupe` の検証:

- 同一 link の item が 1 件に畳まれること
- key 指定ごとの挙動 (guid / link / normalized_title)
- 対象フィールドが None の item を含む場合

### 11.2 統合テスト

固定 XML を返すモックサーバーを立て、取得 → パース → Processor 連鎖 → 保存 → `/feeds/:name` 配信 までを 1 本通す。

- 304 応答時に処理がスキップされること
- 取得失敗時に直前の配信内容が維持されること

## 12. ビルド、配信、運用

### 12.1 環境

- 開発: macOS / ARM64
- 運用: Linux / AMD64、前段に Caddy (reverse_proxy)

ローカルでのクロスビルドは行わない。開発機ではネイティブビルドとテストのみを行い、配布用バイナリは CI で生成する。

### 12.2 CI (`.github/workflows/ci.yml`)

PR と main への push で実行する。

```
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo audit
```

### 12.3 リリース (`.github/workflows/release.yml`)

`v*` タグの push で起動する。

- `ubuntu-latest` 上で `x86_64-unknown-linux-musl` 向けにビルドする
- musl による完全静的リンクにより、運用環境の glibc バージョンに依存しない単一バイナリになる
- 成果物 `rss-proxy-x86_64-linux-musl.tar.gz` と SHA256 チェックサムを GitHub Releases に添付する

Docker イメージは作らない。静的単一バイナリのほうが軽量で、運用も単純になる。必要になった時点で追加する。

### 12.4 運用構成

systemd サービスとして常駐させる。

```ini
[Unit]
Description=rss-proxy
After=network-online.target

[Service]
Type=simple
User=rss-proxy
Group=rss-proxy
ExecStart=/usr/local/bin/rss-proxy serve --config /etc/rss-proxy/config.toml
Restart=on-failure
StateDirectory=rss-proxy
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes

[Install]
WantedBy=multi-user.target
```

listen は `127.0.0.1:8080` とし、外部公開は Caddy が担当する。

```
rss.example.com {
    handle /feeds/* {
        reverse_proxy 127.0.0.1:8080
    }
    handle {
        basic_auth {
            admin <bcrypt-hash>
        }
        reverse_proxy 127.0.0.1:8080
    }
}
```

配信パス `/feeds/*` は認証なし、管理画面と API は Basic 認証で保護する。管理画面はフィード登録と Processor 設定を書き換えられるため、認証なしで公開しない。

### 12.5 更新

GitHub Releases から最新版を取得し、チェックサムを検証して差し替え、サービスを再起動する更新スクリプトを用意する。

```
最新リリースのタグ取得 → tar.gz と SHA256 をダウンロード → 検証
→ /usr/local/bin/rss-proxy を置換 → systemctl restart rss-proxy
```

バイナリ自身が自己更新する機能は実装しない。運用側スクリプトで十分であり、自己書き換えは権限設計を複雑にする。

DB スキーマのマイグレーションは起動時に自動適用する。適用前に DB ファイルのバックアップを取る。

## 13. 実装順序

1. `model.rs` / `parse.rs` / `render.rs` — フィクスチャを読み込んで再出力できる状態にする
2. `proc/mod.rs` と `proc/vendor/google_news.rs` — 実害ケースをテストで固定して解決する
3. `proc/dedupe.rs`
4. `store/` — スキーマと基本クエリ
5. `fetch.rs` / `scheduler.rs` — 巡回と条件付き GET
6. `web/serve.rs` — 配信エンドポイント。**ここで中核機能が完成する**
7. `cli/` — 設定管理と `preview`
8. `web/api.rs` / `web/ui.rs` — Web UI
9. CI / リリースワークフロー / 運用スクリプト

6 の時点で「登録したフィードを整形して再配信する」が動く。7 以降は運用性の向上。

## 14. v1 では実装しないもの

判断の記録として残す。実装しない代わりに仕様は残してあるので、必要が生じた時点で本章から実装に移せる。

- **スクリプトエンジンの組み込み** (Rhai / Lua / WASM) — プラグイン ABI の維持コストに見合わない。外部プロセス連携 (14.4) で代替する
- **出力形式の複数対応** (Atom 出力、JSON Feed 出力) — 入力は両対応、出力は RSS 2.0 に固定
- **item 単位の永続化と全文検索** — 処理後 XML の保存で足りる
- **マルチユーザーとアカウント管理** — 認証は Caddy に委ねる
- **Docker イメージの配布** — 静的単一バイナリで足りる
- **自己更新機能** — 更新スクリプトで足りる

### 14.1 フィードの合成 (複数フィードのマージ)

複数の上流フィードを 1 本にまとめて配信する機能。

**見送りの判断**

解決すべき問題が確定していない。Google ニュースの重複表示は実物のフィードで再現を確認できているが、合成については「まとめたい情報源の具体的な組」がまだ存在しない。内容重複の判定しきい値も、調整対象となる実データがなければ決めようがない。

先に作った場合のコストは実装だけでは済まない。合成を成立させるには Processor チェーンを 2 段 (上流ごとの前処理 / 統合後の後処理) に分ける必要があり、CLI・API・Web UI がいずれも 2 段構成に対応しなければならない。

**後から足す場合**

現行スキーマの `feeds` テーブルは、上流を表す `sources` に必要な列 (url, interval_secs, etag, last_modified, next_fetch_at, fail_count) をすでに持っている。移行は次の形になる。

```sql
ALTER TABLE feeds RENAME TO sources;
CREATE TABLE feeds (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
ALTER TABLE sources ADD COLUMN feed_id INTEGER REFERENCES feeds(id);
-- 既存レコードを 1 対 1 で backfill
ALTER TABLE processors RENAME TO source_processors;  -- 前処理
CREATE TABLE processors (...);                       -- 統合後の後処理
```

合わせて必要になる実装:

- 統合処理 — 全 source の item を連結し、published 降順で整列する。item ごとに由来する source 名を保持する
- `dedupe` への `key: "fuzzy_title"` 追加 — 同じ出来事を扱う別媒体の記事は URL が一致しないため、タイトルの内容類似で判定する。日本語は空白区切りではないので、文字 2-gram 集合の Jaccard 係数を使う (形態素解析器は導入しない。辞書のサイズに見合わない)。しきい値は実データで調整する。誤検出は避けられないため、しきい値は高めから始めて実データを見ながら下げる

### 14.2 見送った Processor

いずれも `Processor` trait の実装 1 ファイル (20〜30 行程度) で追加できる。実際に必要な場面が出てから足す。

#### strip_redundant_description

description が title の複製でしかない item の description を除去する汎用処理。

```
1. description から HTML タグを除去してプレーンテキスト化
2. &nbsp;、全角空白、連続空白を正規化
3. title 側も正規化する (末尾の " - 媒体名" を除去)
4. 正規化後の両者の類似度が threshold 以上なら description を空にする
```

パラメータ: `threshold` (既定 0.9)

**見送りの理由**: Google ニュースの単一形式は `google_news_cluster` が処理する。他のフィード 4 種を実測した範囲では同じ問題が起きていないため、汎用版を先に持つ必要がない。Google 以外で同じ症状を確認したら追加する。

#### strip_title_suffix

title 末尾の媒体名を除去する。

パラメータ:
- `mode`: `source` / `regex`。既定 `source`
  - `source`: item の `<source>` 要素の値と一致する末尾を除去する
  - `regex`: `pattern` にマッチする末尾を除去する
- `pattern`: `mode: regex` のときの正規表現。既定 `" - [^-]+$"`

正規表現方式はタイトル本文にハイフンを含む場合に誤って切り落とすため、`<source>` を持つフィードでは `source` を使う。

**見送りの理由**: 媒体名がタイトルに付くこと自体は害ではなく、むしろ情報として有用。`google_news_cluster` は内部で同じ正規化を行うが、それは重複判定のためであり title を書き換えはしない。

#### strip_html

description から HTML タグを除去する。

パラメータ: `target` (`description` / `title` / `both`。既定 `description`)

**見送りの理由**: description の HTML は多くのリーダーが正しく描画する。`google_news_cluster` が `<ol>` を再構築する際に不正な断片は解消される。

#### filter

item を正規表現で選別する。

パラメータ: `field` (`title` / `description` / `link`)、`include`、`exclude`

#### rewrite

指定フィールドに正規表現置換を適用する。

パラメータ: `field`、`pattern`、`replacement`

#### limit

item 数を上限で切る。

パラメータ: `n`

**filter / rewrite / limit の見送り理由**: いずれも具体的な用途が確定していない。フィルタしたい条件も、置換したい文字列も、切り詰めたい件数も、実際に運用してみないと決まらない。

### 14.3 Web UI のプレビュー画面

Processor 適用前後の item を並べて表示し、効果をその場で確認できる画面。API は `GET /api/feeds/:id/preview`。

**見送りの理由**: v1 の Processor は 2 種類しかなく、設定に迷う余地が小さい。確認手段としては CLI の `rss-proxy preview` で足りる。Processor が増えて組み合わせが複雑になったら追加する。

### 14.4 exec Processor (外部プロセス連携)

外部コマンドに処理を委譲する Processor。stdin に JSON Feed 形式で書き出し、stdout から同形式を読み戻す。

```
[fetch] → dedupe → exec("jq '...'") → [serve]
```

XML ではなく JSON を使うことで、jq / awk / Python など任意のツールを接続できる。

パラメータ:
- `command`: 実行するコマンドと引数
- `timeout_secs`: 既定 10
- `on_error`: `pass_through` (元の Feed をそのまま次段へ) / `fail` (このフィードの更新を失敗扱い)。既定 `pass_through`

**見送りの理由**: 実際に接続したい外部コマンドがまだ存在しない。それに対して、この機能が持ち込むものは大きい。

- 任意コード実行の攻撃面が増える。Web UI から登録できる状態では、管理画面に到達できる者がサーバー上で任意のコードを実行できることになる
- JSON Feed 形式の定義と、それを保つ責任が発生する
- 外部プロセスのタイムアウト、異常終了、部分出力の扱いが必要になる

**追加する場合の前提**

- `exec` インスタンスの登録・変更は CLI からのみ許可し、Web UI からは既存インスタンスの有効/無効の切り替えのみとする
- サービスは専用の低権限ユーザーで実行する (12.4 の systemd 設定で満たしている)
- 実行コマンドはログに記録する
- systemd の `ProtectSystem=strict` などの制限が外部コマンドにも及ぶ点に注意する
