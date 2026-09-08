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

- reqwest は `rustls` フィーチャを使う (既定の native-tls / OpenSSL は musl 静的ビルドでリンクできない)
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

### 4.3 フィードの識別子と表示名

配信 URL に使う識別子 (`slug`) と、画面に出す表示名 (`label`) を分ける。当初は 1 つの `name` が両方を兼ねていたが、日本語や空白を含む名前を付けると配信 URL が `/feeds/NHK%20主要ニュース` になり扱いづらかった。

| 項目 | 用途 | 制約 |
|------|------|------|
| `slug` | 配信 URL とコマンドラインでの指定 | 英数字と `-` `_`、3〜64 文字、一意 |
| `label` | 表示名 (任意) | 自由。空なら上流フィードの `title` を使う |

`slug` を省略した場合は 128 ビットの乱数を base64url にした 22 文字を割り当てる。URL を知っている人だけが読める状態になり、推測での発見を防げる。

ただしこれは軽い目隠しであって認証ではない。URL は RSS リーダーの同期先、ブラウザの履歴、プロキシのログ、Referer にも残る。秘匿が必要なら配信側にも認証を足すことになるが、v1 の範囲ではそこまでしない。

`label` は管理画面の表示に使うだけでなく、**配信する XML の `<title>` も置き換える**。検索条件を URL に埋め込む形のフィード (Google ニュースの検索フィードなど) は、channel の title が検索クエリそのままになり、購読すると読みづらい。

```
"when:1d 最新ニュース  -スポーツ -オリコン -競輪 -競馬 …" - Google ニュース
```

タイトル置換のために別の Processor を用意はしない。`label` は「このフィードをどう呼ぶか」であり、管理画面と配信物で違う名前を持つ理由がない。上流の title は記録として `feeds.title` に残す。

`slug` は後から変更できる。変更すると配信 URL が変わり、購読中の登録が切れる。画面と CLI の両方でその旨を示す。

#### 既存 DB からの移行

`name` を URL に使っていた形からの移行では、すでに URL に載せられていた名前はそのまま `slug` にする。購読中の URL を壊さないため。空白や日本語を含む名前は `slug` を乱数で振り直し、元の名前を `label` へ移す。

スキーマの版は SQLite の `user_version` で管理する。

### 4.4 時刻とタイムゾーン

上流フィードの時刻表記は揃っていない。実測した範囲でも以下が混在していた。

| フィード | 表記 |
|----------|------|
| Google ニュース | `Sun, 06 Sep 2026 22:35:00 GMT` |
| Publickey (Atom) | `2026-09-06T15:07:10Z` |
| NHK ニュース | `Sat, 08 Aug 2026 21:54:30 +0900` |

内部モデルは `DateTime<Utc>` で保持し、出力もすべて `+0000` で書き出す。オフセットが違っても指す瞬間は同じなので、正規化しても情報は失われない。

元の表記 (`+0900` など) は保持しない。feed-rs が `DateTime<Utc>` を返す時点でオフセットは失われており、保持するには自前のパーサが必要になる。表示上の見た目のためにそこまでする理由がない。

一方、管理画面に出す時刻 (最終取得) は運用者のローカルタイムで表示する。UTC のまま出すと日本時間との差を頭の中で足すことになる。

どのオフセットで表示しているかは列見出しに 1 度だけ示す (`最終取得 (+09:00)`)。値ごとにオフセットを繰り返しても情報は増えず、横に長くなるだけ。

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

### 5.1.1 グローバル連鎖

全フィードに適用する連鎖を 1 つ持つ。**フィード固有の連鎖より先に走る。**

```
[取得] → グローバル連鎖 → フィード固有の連鎖 → [保存]
```

順序が意味を持つ。全角の正規化を先に済ませておけば、フィード側の判定を半角の表記で書ける。逆順だと `台風２４号` に対して `台風24号` という指定が一致しない。

**新しい DB には既定の連鎖を入れる** (migration v3)。どのフィードでも効く整形と、広告記事の除去。

```
normalize_width {"target":"both"}
exclude {"words":["【PR】","[PR]","PR:","【広告】","[広告]","<PR>","(PR)"],"target":"title"}
```

`normalize_width` の対象を `both` にしているのは、title だけを直すと title と description を突き合わせる処理 (`google_news_cluster`) が一致しなくなるため。実際にこの組み合わせで 1 件取りこぼす不具合が起きた。

`exclude` の語は広告であることが明示された表記だけを対象にする。「広告」単体は広告業界のニュースまで落とすので入れない。正規化が先に走るので `［PR］` は `[PR]` になった状態で判定される。

既定が不要なら画面か CLI から空にできる。グローバル連鎖を変えると全フィードが作り直しの対象になる。既定を入れる移行でも同じ扱いにする。検証子が残っていると 304 で処理がスキップされ、入れたばかりの連鎖が反映されないため。

### 5.1.2 有料記事の判定

判定には記事ページの取得が必要で、`Feed -> Feed` の純粋関数では扱えない。そこで**判定と処理を分ける**。

```
[取得] → 有料判定 (I/O・キャッシュ) → グローバル連鎖 → フィード固有 → [保存]
              ↑ scheduler の仕事              ↑ Processor は純粋なまま
```

判定結果は `Item::paywalled` に入る (`Some(true)` 有料 / `Some(false)` 無料 / `None` 判定不能)。`paywall` Processor はこの値を見るだけで、通信しない。

**判定ルールは媒体ごと。** 共通規格の schema.org `isAccessibleForFree` を出す媒体もあれば、独自の値しか持たない媒体もある。`src/paywall.rs` に 1 か所へまとめ、媒体が増えたら足す。

| 媒体 | 目印 |
|------|------|
| 読売新聞 | `isAccessibleForFree` (`false` が有料) |
| 日本経済新聞 | `paywallProps.isLockedArticle` (`true` が有料) |

`isPaidUserOnlyArticle` は使わない。日経では「完全会員限定」だけを指し、途中まで読める従量型では `false` になる。

**文字列の一致では判定できない。** 記事ページには関連記事の一覧が載るため、「有料」を示す語や class 名は無料記事のページにも現れる。実測で読売の無料記事にも `data-icon-type="key-locked"` が含まれていた。判定にはその記事自身を指す構造化データだけを使う。

#### 費用を抑える仕組み

1 件につき記事ページを 1 回取りに行くので、次の順で絞る。

1. **ルールのある媒体の item だけ**を対象にする。Publickey や NHK には 1 回も通信しない
2. **判定済みならキャッシュを使う** (`paywall_cache`)。1 記事につき 1 回だけ
3. **1 回の巡回で取りに行く数に上限を置く** (20 件)。新着が大量にあっても媒体を叩き続けない。残りは次の巡回で判定する

キャッシュは 30 日で捨てる。配信から消えた記事の判定結果は使われない。

判定に失敗しても巡回は続ける。有料かどうかは配信の可否ではない。

#### 媒体を増やす手順

フィードを追加するとき、その媒体に有料記事があれば合わせてルールを足す。作業は 30 分程度。

**1. 有料記事と無料記事を 1 件ずつ特定する**

媒体の一覧ページには、有料記事に印が付いていることが多い (読売なら `data-icon-type="key-locked"`)。それを手がかりに両方の記事 URL を得る。印がなければ、実際に読んで判断する。

**2. 両方のページを取得して差分を探す**

探すのはその記事自身の状態を示す構造化データ。以下の順で当たる。

- `isAccessibleForFree` — schema.org の共通規格。出していれば最優先で使う
- `paywall` / `locked` / `premium` / `subscription` を含む JSON のキー
- `<meta>` タグ

**3. 落とし穴を 2 つ確認する**

- **関連記事の巻き込み** — 記事ページには他の記事の一覧が載る。無料記事のページにも「有料」を示す語や class 名が現れる。実測で読売の無料記事にも `key-locked` が含まれていた。**無料記事側でも一致してしまう目印は使えない**
- **意味の取り違え** — 日経の `isPaidUserOnlyArticle` は「完全会員限定」だけを指し、途中まで読める従量型では `false` になる。名前だけで選ばず、有料記事で実際に期待した値になるか確かめる
- 値が空の場合もある。毎日新聞の `cXenseParse:mai-fee-charging` は JavaScript が埋めるため、サーバー側からは常に空

**4. フィクスチャとルールを足す**

有料・無料それぞれのページから目印の周辺を切り出して `tests/fixtures/paywall/` に置く。`src/paywall.rs` の `RULES` に 1 行足し、判定関数を書く。テストは既存のものをコピーして 4 行。

判定できない媒体なら、ルールを足さない。`Unknown` として残るだけで害はない。

#### Google ニュース経由のフィードでは使えない

Google ニュースの `<link>` はリダイレクタで、元記事の URL を得るには記事ごとに 590KB のページ取得と非公開 RPC が必要になる (調査済み)。そこまでして判定する費用に見合わない。

ただし title の末尾に媒体名が入る (`… - 日本経済新聞`) ため、**媒体単位で落とすなら既存の `exclude` で足りる**。

```
exclude {"words":["- 日本経済新聞"],"target":"title"}
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

#### exclude

指定した語を含む item を取り除く。上流の検索条件で除外を書けるフィードでも、書ききれなかった語や後から気づいた語をこちら側で落とせる。

パラメータ:
- `words`: 語の配列。1 つでも一致すれば取り除く。大文字小文字は区別しない。既定 `[]` (何も落とさない)
- `target`: `title` / `description` / `both`。既定 `title`

`description` を見る場合は HTML を落としてから探す。加えて空白を除いた形でも照合する。`<b>スポ</b>ーツ` のようにタグで分断された語を取りこぼさないため。

`description` は既定で見ない。Google ニュースのクラスタ形式では関連記事のタイトルが並ぶため、そこまで見ると意図より多く落ちる。

#### max_age

指定した時間より古い item を落とす。上流が過去分を載せ続けるフィードで、リーダーの一覧が古い記事で埋まるのを防ぐ。

パラメータ:
- `hours`: この時間より前の item を落とす。既定 24

日時を持たない item は古いかどうか判断できないので残す。未来の日時 (上流のタイムゾーン誤りなど) も古くはないので残る。

#### normalize_width

全角の英数字と記号を半角に直す。報道系のフィードは数字を全角で書くことが多く (`岐阜のケーキ店３人死亡火災`、`レベル５`)、読みづらい。実測では Google ニュースのフィード 70 件中 31 件の title に全角の英数字記号が含まれていた。

パラメータ:
- `target`: `title` / `description` / `both`。既定 `title`

**ASCII に対応がある全角文字はすべて半角にする。** 対象は U+FF01〜U+FF5E (全角 ASCII) の範囲。全角コロン (`：`) も全角括弧 (`（）`) も含む。これらは日本語固有の記号ではなく ASCII の異体字なので、半角に揃えるほうが読みやすく検索もしやすい。

カギ括弧 (`「」『』`)、句読点 (`、。`)、なかてん (`・`)、波ダッシュ (`〜`) はいずれもこの範囲の外にあるため、何もしなくてもそのまま残る。

唯一の例外が全角チルダ (`～` U+FF5E)。範囲内にあるが、日本語では区間を表す記号として使われるため変換しない。

#### dedupe

item 間の重複除去。先に出現したものを残す。巡回のたびに同一 item が再出現するフィードへの保険として v1 に含める。

パラメータ:
- `key`: `guid` / `link` / `normalized_title`。既定 `link`

### 5.5 Processor カタログ

種別ごとの説明とパラメータの仕様を `proc::catalog()` に 1 か所だけ持つ。CLI の `proc list` と Web UI のヘルプは同じ定義から生成し、説明が二重管理になるのを避ける。

各パラメータについて、名前・説明・既定値・取りうる値を持つ。

Processor は自身の実効パラメータを JSON で返せる (`Processor::params`)。省略された項目を既定値で埋めた形が得られるので、これを保存に使う。

## 6. 取得とスケジューリング

### 6.1 背景巡回

オンデマンド取得 (配信要求時に上流へ取りに行く) は採用しない。アクセス集中時に上流への負荷が予測不能になるため。

- フィードごとに `interval_secs` を設定する。既定 900 秒 (15 分)。
- スケジューラは 60 秒間隔の tick ループを 1 本だけ持ち、毎 tick で SQLite から `next_fetch_at <= now` のフィードを取得して処理する。フィードごとにタスクを常駐させる方式は取らない (設定変更の反映と資源管理が煩雑になるため)。
- 1 tick 内の処理は逐次で行う。SQLite の接続は `Sync` ではなく、共有したまま並行実行できない。巡回間隔に対してフィード数が十分少ないうちは逐次で足りる。必要になったら接続プールに置き換える。

### 6.2 条件付き GET

- 前回取得時の `ETag` と `Last-Modified` を保存し、次回リクエストで `If-None-Match` / `If-Modified-Since` を送る。
- 304 が返った場合は解析と Processor 適用をスキップし、`next_fetch_at` だけ更新する。
- **設定を変えたときは検証子 (`etag` / `last_modified`) を捨て、`next_fetch_at` を 0 にする。**

  検証子が残っていると 304 で解析も Processor 適用もスキップされ、上流が更新されるまで配信内容が古いままになる。次回取得時刻が先のままだと、最大で巡回間隔ぶん反映が遅れる。どちらも「設定したのに効いていない」ように見える。

  対象は表示名・識別子の変更、Processor 連鎖の更新、即時取得。設定変更後は次の tick (最大 60 秒) で配信内容が作り直される。
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
    name            TEXT NOT NULL UNIQUE,   -- 移行で slug へ改名 (4.3)
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

CREATE TABLE paywall_cache (
    url        TEXT PRIMARY KEY,
    access     TEXT NOT NULL,   -- paid / free / unknown
    checked_at INTEGER NOT NULL
);

CREATE TABLE global_processors (
    id       INTEGER PRIMARY KEY,
    position INTEGER NOT NULL UNIQUE,
    kind     TEXT NOT NULL,
    params   TEXT NOT NULL
);

CREATE TABLE outputs (
    feed_id      INTEGER PRIMARY KEY REFERENCES feeds(id) ON DELETE CASCADE,
    xml          TEXT NOT NULL,     -- 処理後の RSS 2.0
    generated_at INTEGER NOT NULL
);
```

処理後の XML をそのまま保存する。item 単位で正規化して保存する設計も考えられるが、配信のたびに再構築するコストが増えるだけで利点がない。

設定ファイルは持たない。listen アドレスと DB パスの 2 つしか外部設定がなく、いずれもコマンドライン引数 (`--listen` / `--db`) で既定値付きで指定できる。フィードと Processor の設定はすべて DB に持つ。

## 8. HTTP API と Web UI

### 8.1 エンドポイント

| メソッド | パス | 用途 |
|----------|------|------|
| GET | `/feeds/:name` | 処理済みフィードの配信 (`application/rss+xml`) |
| GET | `/` | フィード一覧 |
| GET | `/ui/feeds/:slug` | フィード編集 |
| POST | `/ui/feeds` | フィード登録 |
| POST | `/ui/global-processors` | 全フィード共通の連鎖の更新 |
| POST | `/ui/feeds/:slug/rename` | 識別子・表示名・巡回間隔の変更 |
| POST | `/ui/feeds/:slug/delete` | フィード削除 |
| POST | `/ui/feeds/:slug/processors` | Processor 連鎖の一括更新 |
| POST | `/ui/feeds/:slug/fetch` | 即時取得 (次回取得時刻を過去にして tick に拾わせる) |
| GET | `/healthz` | ヘルスチェック。稼働中のバージョンを JSON で返す |

JSON API (`/api/*`) は用意しない。当初は Web UI がそれを呼ぶ想定だったが、フォーム POST で直接処理すれば足りる。API を消費するものが現れてから追加する。

### 8.2 Web UI

サーバーサイドで HTML を生成する。SPA フレームワークもテンプレートエンジンも JavaScript も使わない。すべてフォーム POST で完結させる。

画面は 2 枚。

1. フィード一覧 (`/`) — 識別子、表示名、間隔、最終取得、状態、配信 URL。全フィード共通の連鎖の編集、登録フォーム。最終取得の列見出しにローカルのオフセットを添える
2. フィード編集 (`/ui/feeds/:slug`) — Processor 連鎖の編集、配信中の内容の一覧、識別子と表示名の変更、即時取得、削除

Processor 連鎖はテキストエリアで編集する。1 行に 1 つ、「種別 パラメータ(JSON)」の形式で書き、行の並びが適用順になる。追加・削除・並べ替えがすべてテキスト編集で済み、行ごとのボタンや並べ替え UI が不要になる。保存時にすべての行を組み立てて検証し、1 つでも不正なら 400 を返して何も保存しない。

**省略されたパラメータは既定値で埋めて保存する。** `dedupe` とだけ書いても `dedupe {"key":"link"}` として残る。行を消して書き直したときに、何を設定していたのかが画面から読み取れなくなるのを防ぐ。CLI の `proc attach` も同じ扱いにする。

各 Processor の説明とパラメータの一覧は 5.5 のカタログから生成し、編集画面に折りたたみで表示する。CLI の `proc list` も同じカタログを使う。

編集画面には「項目のフィールド」として、配信中の 1 件目の item が各フィールドに実際に何を持っているかを表示する。`dedupe` のキーを選ぶとき、`guid` や `link` に何が入っていて空でないかを確認できる。

一度も取得に成功していないフィードの配信 URL は 404 になる。この場合はリンクにせず `-` を表示する。

編集画面には、保存済みの配信内容を解析した item 一覧をフィードに書かれている順で表示する。Processor を付け替えた結果として実際に何が配信されるのかを、RSS リーダーに登録せずに確認できる。

適用前後の比較や Processor ごとの段階表示は行わない。それには取得直後の XML も保存する必要があり、「このフィルタを付けると何が配信されるか」を知る目的にはそこまで要らない。

バージョンは 3 か所に出す。運用中に「今どれが動いているか」「配信物がいつのバージョンで作られたか」を確認するため。

| 場所 | 形式 | 用途 |
|------|------|------|
| 画面のフッター | `rss-proxy 0.2.0` | 人が見る |
| `GET /healthz` | `{"status":"ok","version":"0.2.0"}` | 更新の有無を機械的に確認する |
| 配信 XML の `<generator>` | `rss-proxy 0.2.0` | どのバージョンが処理した出力かを配信物自体に残す |

`<generator>` は処理した時点のバージョンであり、稼働中のバージョンとは一致しないことがある。バイナリを差し替えても、次に巡回するまで保存済みの出力は作り直されないため。両者を比べれば「更新後まだ再処理されていないフィード」が分かる。

### 8.3 管理画面の認証

配信パス (`/feeds/*`) と `/healthz` は認証しない。URL を知っていれば読めてよく、認証をかけると RSS リーダーが読めなくなる。

管理画面はフィードの登録と Processor の設定を書き換えられるため保護する。方式は HTTP Basic 認証。ブラウザ標準の仕組みだけで完結し、セッションの保存も Cookie の属性も要らない。

資格情報は環境変数で渡す。

```
RSS_PROXY_ADMIN_USER=admin
RSS_PROXY_ADMIN_PASSWORD_HASH='$argon2id$v=19$m=19456,t=2,p=1$...'
```

パスワードは argon2 のハッシュで持つ。ハッシュは `rss-proxy hash-password` で生成する。検証は 1 リクエストごとに走り 100ms 前後かかるが、管理画面のアクセス頻度では問題にならず、総当たりへの抑止にもなる。

**未設定時はループバックからのみ許可する。** それ以外は 403。設定漏れがそのまま全公開になる事故を防ぎつつ、手元での試用は今までどおり動く。

**リバースプロキシ配下では資格情報の設定が必須。** Caddy 経由の接続は接続元が `127.0.0.1` に見えるため、未設定のままだとループバック判定を通過してしまう。

#### CSRF 対策

Basic 認証の資格情報はブラウザが自動で付与する。そのため外部サイトに置かれたフォームからの送信も認証済みとして届く。

状態を変える要求 (GET / HEAD 以外) は、`Origin` (なければ `Referer`) の authority が `Host` と一致することを確かめる。ブラウザは別オリジンへの POST に必ず `Origin` を付けるため、これで防げる。scheme は比較しない。前段が TLS を終端すると一致しないため。

`Origin` も `Referer` もない要求は通す。ブラウザ以外からの操作であり、資格情報が自動付与される経路ではない。

### 8.4 上流フィードを信用しない

上流フィードの内容は攻撃者が決められるものとして扱う。実際に対処している点。

| 経路 | 想定 | 対処 |
|------|------|------|
| 本文のサイズ | 巨大な応答でメモリを食い潰す | 10MB で打ち切る。Content-Length も読む前に見るが、偽られても途中で止まる |
| リダイレクト | 内部アドレスへ誘導し、公開していないサービスの内容を取りに行かせる | 追従先のホストが loopback / プライベート / リンクローカル (クラウドのメタデータ) なら止める。追従は 5 回まで |
| item の `<link>` | `javascript:` や `data:` を管理画面の `href` に置き、管理者のクリックで実行させる | `http`/`https` 以外はリンクにしない。タイトルは表示する |
| title / description | 管理画面に対する格納型 XSS | HTML へ埋め込む前にエスケープする (8.5) |
| XML の実体参照 | XXE、実体の入れ子展開 (billion laughs) | feed-rs が使う quick-xml は独自実体を展開しない。`&xxe;` は文字列のまま残る (実測で確認) |

ホスト名の判定は名前解決の結果までは見ていない。DNS を内部アドレスへ向ける手口は防げない。防ぐには解決後の IP を見る必要があり、そこまでは実装していない。

### 8.5 出力のエスケープ

フィードのタイトルと直近のエラー文言は上流フィード由来の文字列であり、それが管理画面に表示される。悪意のあるフィードのタイトルは管理画面に対する格納型 XSS になりうる。HTML へ埋め込む前に必ずエスケープする。

## 9. CLI

サーバーと同一バイナリ。サブコマンド構成。

```
rss-proxy serve [--listen 127.0.0.1:8080] [--db PATH]

rss-proxy feed add <name> <url> [--interval 900]
rss-proxy feed list
rss-proxy feed show <name>
rss-proxy feed rm <name>
rss-proxy fetch <name>               # 即時取得 (サーバー停止中でも実行できる)

rss-proxy proc list                  # 利用可能な Processor 種別と説明
rss-proxy proc attach <feed> <kind> [--params '{"key":"link"}'] [--at N]
rss-proxy proc detach <feed> <position>
rss-proxy proc move <feed> <from> <to>

rss-proxy preview <name>             # 保存済みの配信内容を標準出力に表示
```

CLI はサーバー API を経由せず SQLite に直接アクセスする。サーバーが停止していても設定を編集でき、初期セットアップやトラブル時の復旧が容易になる。WAL モードにより稼働中のサーバーとの同時アクセスも安全。

`preview` は保存済みの配信内容を表示する。Processor の効果はこれで確認する。Web UI 側のプレビュー画面は見送る (14.3)。

## 10. ディレクトリ構成

```
src/
├── main.rs            # エントリポイント、引数解析、serve の起動
├── model.rs           # Feed, Item
├── fetch.rs           # HTTP 取得、条件付き GET
├── parse.rs           # feed-rs → model 変換
├── render.rs          # model → RSS 2.0 XML
├── paywall.rs         # 有料記事の判定ルール (媒体ごと)
├── slug.rs            # 配信 URL に使う識別子の生成と検証
├── store.rs           # SQLite (スキーマ、マイグレーション、クエリ)
├── proc/
│   ├── mod.rs         # trait Processor、レジストリ、チェーン実行
│   ├── dedupe.rs
│   └── vendor/
│       ├── mod.rs
│       └── google_news.rs   # google_news_cluster
├── scheduler.rs       # 巡回ループ、バックオフ
├── cli.rs             # feed / proc サブコマンド
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

deploy/
├── rss-proxy.service    # systemd unit
├── Caddyfile.example
└── update.sh            # リリース取得、検証、差し替え、再起動
```

## 11. テスト方針

TDD を前提とする。仕様 (本書) → テスト作成 → 実装 の順で進める。

### 11.0 何をテストするか

**カバレッジの数値目標は置かない。** 目標にすると、テストが実装をなぞる方向に寄る。そうしたテストはリファクタのたびに壊れて変更を妨げるうえ、「通っているから安全」という誤った安心感を生む。カバレッジは計器として測るが、CI の合否には使わない。

代わりに「**壊れたときに取り返しがつくか**」で決める。

**テストを必ず書く (壊れても気づけない、または戻せない)**

| 対象 | 壊れたときに起きること |
|------|----------------------|
| `proc/*` | 出力が壊れても誰も気づかないまま購読者に配信され続ける |
| `store.rs` のマイグレーション | データが失われたら戻せない |
| `auth.rs` / `web/guard.rs` | 認証を回避されると管理画面が公開される |
| `parse.rs` / `render.rs` | 全フィードに波及する |
| `slug.rs` | URL に載せられない識別子が通ると配信できなくなる |

**テストを必須にしない (壊れてもすぐ気づいて直せる)**

- CLI の出力文字列、HTML の細部、`main.rs` の起動処理
- 巡回ループ本体 (`scheduler::run_loop` / `tick`) — 時間経過に依存し、手間に見合わない。1 フィードを処理する `refresh` は個別にテストする

テストの実行時間は制約にならない。実測で 121 件が 2.05 秒であり、CI 全体 2〜3 分のうち 99% はコンパイルが占める。テストを増やすことのコストは実行時間ではなく保守。

### 11.0.1 カバレッジの測り方

`cargo-llvm-cov` で測る。tarpaulin より速く、macOS ARM でも動く。

```console
$ cargo llvm-cov --summary-only
```

CI では計測して結果をジョブサマリに出すだけで、しきい値による失敗はさせない。数値が下がったこと自体は問題ではなく、**上の表の対象が無防備になっていないか**を見るために使う。

導入時の実測 (2026-09-07、121 件): 全体 82.8% (regions) / 87.3% (lines)。`main.rs` を除くと 92% 前後。

このとき計測が実際に見つけた穴は 1 つだけだった。`Processor::params` (省略された設定を既定値で埋めて保存する機能) を検証していたのが `dedupe` と `google_news_cluster` だけで、`max_age` / `normalize_width` / `exclude` は素通りだった。壊れると DB に誤った設定が残る箇所であり、テストを足す価値があった。

対応はカタログを総当たりするテスト 1 本。種別が増えても自動的に対象になる。これで `proc/*` は 64% → 100%、76% → 98% と上がった。**数を増やさずに穴だけ塞ぐ**のが狙いどおりの使い方。

意図的に空けている箇所:

- `Admin::from_env` — 環境変数を読んで `Admin::new` に渡すだけ。判定ロジックは `Admin::new` 側でテスト済み。`std::env::set_var` は edition 2024 では `unsafe` で、並列テストで競合する
- `scheduler::run_loop` / `tick` — 時間経過に依存する。1 フィードを処理する `refresh` は個別にテスト済み
- `main.rs` — 引数解析とプロセス起動

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
cargo install cargo-audit && cargo audit
```

### 12.3 リリース (`.github/workflows/release.yml`)

`v*` タグの push で起動する。`auto-release.yml` からも呼び出せるよう `workflow_call` を受け付ける。

以下の 2 ターゲットをマトリクスでビルドする。

| ターゲット | ランナー | 備考 |
|-----------|---------|------|
| `x86_64-unknown-linux-musl` | ubuntu-latest | 運用環境。musl による完全静的リンクで glibc のバージョンに依存しない |
| `aarch64-unknown-linux-musl` | ubuntu-24.04-arm | ARM 系の運用環境向け。ネイティブビルドなのでクロスコンパイルは不要 |
| `aarch64-apple-darwin` | macos-latest | 開発環境で動かす用 |

musl ビルドでは事前に `apt-get install -y musl-tools` が必要になる。rusqlite の `bundled` フィーチャが SQLite の C ソースをコンパイルするため、musl 向けの C コンパイラが要る。

### 12.3.3 CI へ投げる前の Linux 確認

`scripts/check-linux.sh` が Linux コンテナ内で musl ビルド、テスト、常駐時の RSS 測定を行う。

開発機が Apple Silicon なので、linux/arm64 のコンテナは QEMU を介さずネイティブに動く。速度も RSS も実機と同じ条件で測れる。逆に linux/amd64 は QEMU 経由となり、メモリ測定の値は当てにならない。

アーキテクチャ間で挙動が変わりうるのはページサイズとコードサイズ程度で、musl のリンク可否や Linux 上の動作といった失敗しやすい部分はこの確認で捕まえられる。

実測 (linux/arm64、ページサイズ 4096、70 item のフィードを 1 巡させたあと): バイナリ 8.3MiB、常駐 RSS 7.6MB、ピーク 8.0MB。

静的リンクの ld は数 GB を消費する。コンテナのメモリ割り当てが 1GB では OOM で kill された。スクリプトは既定で 8GB を割り当てる。

成果物は `rss-proxy-<target>.tar.gz` と SHA256 チェックサムで、GitHub Releases に添付する。

リリースビルドでは LTO を有効にし、コード生成単位を 1 にまとめ、シンボルを除去する (`[profile.release]`)。常駐プロセスなのでビルド時間より生成物の質を優先する。

Docker イメージは作らない。静的単一バイナリのほうが軽量で、運用も単純になる。必要になった時点で追加する。

### 12.3.1 バージョンの付け方

`0.MINOR.PATCH` の 2 桁を用途で分ける。

| 桁 | 意味 | 誰が上げるか |
|----|------|-------------|
| MINOR | 機能追加、挙動の変更 | 手動 |
| PATCH | 依存更新に伴うリビルド、バグ修正 | auto-release / 手動 |

`auto-release.yml` は PATCH しか上げない。機能追加を PATCH に混ぜると、番号を見ただけでは依存更新なのか機能追加なのか区別できなくなる。

Cargo の 0.x の慣行では `0.1.x` と `0.2.0` は非互換として扱われるが、これはライブラリとして依存される場合の話であり、単体バイナリでは影響しない。

### 12.3.2 依存更新と自動リリース

Dependabot が cargo と github-actions の更新 PR を毎週出す。cargo 側はパッチ・マイナーをまとめて 1 本の PR にする。

依存が更新されるとバイナリの中身が変わるため、バージョンで区別できないと配布物を追跡できない。`auto-release.yml` が Dependabot 由来のコミット (`build(deps` で始まるもの) を検知してパッチバージョンを 1 つ上げ、タグを打ち、`release.yml` を呼び出す。

発火条件を `Cargo.lock` の変更だけにすると、機能追加で依存を足したときにも動いてしまう。実際に一度そうなり、機能追加が PATCH として公開された。`Cargo.lock` が変わったことは必要条件ではあるが十分条件ではない。

再帰しないよう、自身が作る `chore(release):` で始まるコミットでは起動しない。またタグ push を GITHUB_TOKEN で行うと他のワークフローが起動しないため、タグ push に反応させるのではなく `workflow_call` で直接呼び出している。

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
ExecStart=/usr/local/bin/rss-proxy serve --db /var/lib/rss-proxy/rss-proxy.db
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
    reverse_proxy 127.0.0.1:8080
}
```

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

**見送りの理由**: 置換したい具体的な文字列がまだない。`exclude` や `normalize_width` のように用途が定まった処理を個別に足すほうが、利用者が正規表現を書かずに済む。

#### limit

item 数を上限で切る。

パラメータ: `n`

**filter / rewrite / limit の見送り理由**: いずれも具体的な用途が確定していない。フィルタしたい条件も、置換したい文字列も、切り詰めたい件数も、実際に運用してみないと決まらない。

### 14.3 適用前後の比較 (差分プレビュー)

Processor 適用前後の item を並べて表示し、どの item がどう変わったかを見せる画面。

**見送りの理由**: 実現するには取得直後の XML も保存する必要がある (dedupe は情報を捨てるため逆適用できない)。一方、実際の用途は「この Processor を付けると何が配信されるか」の確認であり、それは 8.2 の item 一覧で足りる。Processor の種類が増えて組み合わせが複雑になったら再検討する。

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
