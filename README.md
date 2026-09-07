# rss-proxy

RSS/Atom フィードを取得し、登録した処理を適用したうえで再配信するプロキシ。

配信元のフィードには、そのままでは読みづらい構造上の問題があることがある。代表例が Google ニュースで、1 つの item の中で `<title>` と `<description>` が同じ内容を持つため、多くの RSS リーダーで同じ文章が 2 回表示される。

rss-proxy はフィードごとに処理を登録し、修正済みのフィードを配信する。

```
上流フィード ──> 取得 (定期巡回) ──> Processor 連鎖 ──> SQLite ──> /feeds/:name
```

- 単一バイナリ。常駐して定期巡回し、配信は保存済みの結果を返すだけなので上流障害の影響を受けない
- CLI と Web UI の両方から設定できる
- 条件付き GET (ETag / If-Modified-Since) に対応し、変化がなければ処理を省く

設計の詳細と、実装しなかったものの判断理由は [DESIGN.md](DESIGN.md) を参照。

## 使い方

```console
$ rss-proxy feed add gnews "https://news.google.com/rss/topics/...?hl=ja&gl=JP&ceid=JP:ja"
登録しました: gnews

$ rss-proxy proc attach gnews google_news_cluster
gnews の 0 番目に google_news_cluster を追加しました

$ rss-proxy fetch gnews
取得しました: gnews

$ rss-proxy serve
listening on 127.0.0.1:8080
```

RSS リーダーには `http://127.0.0.1:8080/feeds/gnews` を登録する。管理画面は `http://127.0.0.1:8080/` にある。

DB のパスは `--db` で指定する (既定 `rss-proxy.db`)。

## Processor

フィードごとに順序つきの連鎖として登録する。同じ種別を異なるパラメータで複数回登録してもよい。

| 種別 | 内容 | パラメータ |
|------|------|-----------|
| `google_news_cluster` | Google ニュースの description から、title と重複する先頭要素を取り除く。関連記事は残す | なし |
| `dedupe` | item 間の重複除去 | `key`: `guid` / `link` / `normalized_title` (既定 `link`) |

```console
$ rss-proxy proc attach gnews dedupe --params '{"key":"link"}'
$ rss-proxy feed show gnews
```

Web UI では 1 行に 1 つ「種別 パラメータ(JSON)」の形式で書く。行の並びが適用順になる。

```
google_news_cluster
dedupe {"key":"link"}
```

## コマンド

```
rss-proxy serve [--listen 127.0.0.1:8080]   巡回と配信を開始する
rss-proxy fetch <name>                      指定フィードを今すぐ取得する
rss-proxy preview <name>                    保存済みの配信内容を表示する

rss-proxy feed add <name> <url> [--interval 900]
rss-proxy feed list
rss-proxy feed show <name>
rss-proxy feed rm <name>

rss-proxy proc list
rss-proxy proc attach <feed> <kind> [--params JSON] [--at N]
rss-proxy proc detach <feed> <position>
rss-proxy proc move <feed> <from> <to>
```

## 運用

Linux / AMD64 向けの静的リンク済みバイナリを GitHub Releases で配布している。glibc のバージョンに依存しない。

```console
$ REPO=<owner>/rss-proxy deploy/update.sh
```

systemd unit と Caddy の設定例は [deploy/](deploy/) にある。

- 配信パス `/feeds/*` は認証なし
- 管理画面は Caddy 側で Basic 認証をかける。**認証なしで公開しない** (フィード登録と処理設定を書き換えられる)
- rss-proxy 自身は `127.0.0.1` のみで listen する

## 開発

```console
$ cargo test
$ cargo fmt --all && cargo clippy --all-targets -- -D warnings
```

テストは実際に取得した Google ニュースのフィード (`tests/fixtures/`) を固定入力に使う。Processor はいずれも `Feed -> Feed` の純粋関数なので、単体テストで挙動を固定できる。

## ライセンス

MIT
