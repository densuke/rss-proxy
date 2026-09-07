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
$ rss-proxy feed add "https://news.google.com/rss/topics/...?hl=ja&gl=JP&ceid=JP:ja" --slug gnews
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

## 識別子と表示名

配信 URL に使う識別子 (`slug`) と画面の表示名 (`label`) は別物。

- `slug` — 英数字と `-` `_`、3〜64 文字。省略すると 22 文字の乱数になる
- `label` — 自由。日本語も空白も使える。省略すると上流フィードのタイトルを使う

識別子を省略すると `/feeds/k7Rm2xQvB8nT4wLpZaYcDf` のような推測しにくい URL になる。ただしこれは軽い目隠しであって認証ではない。URL は RSS リーダーの同期先やプロキシのログにも残る。

識別子は後から `feed set --new-slug` や管理画面で変更できる。**変更すると配信 URL が変わり、購読中の登録が切れる。**

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

rss-proxy feed add <url> [--slug X] [--label Y] [--interval 900]
rss-proxy feed list
rss-proxy feed show <slug>
rss-proxy feed set <slug> [--new-slug X] [--label Y] [--interval N]
rss-proxy feed rm <slug>

rss-proxy proc list
rss-proxy proc attach <feed> <kind> [--params JSON] [--at N]
rss-proxy proc detach <feed> <position>
rss-proxy proc move <feed> <from> <to>
```

## バージョンの確認

```console
$ curl -s http://127.0.0.1:8080/healthz
{"status":"ok","version":"0.2.0"}
```

管理画面のフッターにも表示される。配信する XML の `<generator>` には、その出力を処理したバージョンが入る。バイナリを更新しても次の巡回までは保存済みの出力が配信されるため、両者を比べると再処理がまだのフィードが分かる。

## 運用

GitHub Releases で以下のバイナリを配布している。

| ターゲット | 用途 |
|-----------|------|
| `x86_64-unknown-linux-musl` | Linux / AMD64。静的リンク済みで glibc のバージョンに依存しない |
| `aarch64-unknown-linux-musl` | Linux / ARM64 (Raspberry Pi、Graviton など) |
| `aarch64-apple-darwin` | macOS / Apple Silicon |

Linux 環境の更新は同梱のスクリプトで行う。最新リリースを取得し、SHA256 を検証してから差し替え、サービスを再起動する。

```console
$ sudo deploy/update.sh   # REPO / DEST / SERVICE で上書きできる
```

```console
$ rss-proxy hash-password
管理画面のパスワード:
もう一度入力:
RSS_PROXY_ADMIN_USER=admin
RSS_PROXY_ADMIN_PASSWORD_HASH='$argon2id$v=19$m=19456,t=2,p=1$...'
```

出力の 2 行を systemd の `EnvironmentFile` に置く。

資格情報が未設定の場合、管理画面はループバックからのみ利用できる (それ以外は 403)。**リバースプロキシ配下では必ず設定すること。** Caddy 経由の接続は接続元が `127.0.0.1` に見えるため、未設定だとループバック判定を通過してしまう。

systemd unit と Caddy の設定例は [deploy/](deploy/) にある。

- 配信パス `/feeds/*` と `/healthz` は認証なし
- 管理画面は Basic 認証で保護する。資格情報は環境変数で渡す
- rss-proxy 自身は `127.0.0.1` のみで listen し、公開は Caddy が担当する

## 開発

```console
$ cargo test
$ cargo fmt --all && cargo clippy --all-targets -- -D warnings
```

Linux 上での挙動は、CI に投げる前にコンテナで確認できる。

```console
$ ./scripts/check-linux.sh
```

musl 静的ビルド、テスト、常駐時の RSS を Linux コンテナ内で確認する。Apple Silicon では linux/arm64 が QEMU なしで動くため、速度もメモリ測定も信用できる。`container` / `docker` / `podman` のいずれかがあれば動く。

実測値 (linux/arm64、musl 静的、70 item のフィードを取得・処理・配信したあと):

| 項目 | 値 |
|------|-----|
| バイナリサイズ | 8.3 MiB |
| 常駐時 RSS | 7.6 MB |
| ピーク (VmHWM) | 8.0 MB |

静的リンクは ld が数 GB を使う。コンテナのメモリ割り当てが少ないと OOM で kill されるため、既定で 8GB を割り当てている (`MEMORY` で変更できる)。

### リリース

`v*` タグを push すると、両ターゲットのバイナリと SHA256 が Releases に添付される。

依存の更新は Dependabot が毎週 PR を出す。それを main にマージすると `Cargo.lock` が変わり、パッチバージョンを 1 つ上げたタグが自動で打たれてリリースが作り直される。同梱される依存が変わればバイナリの中身も変わるため、バージョンで区別できるようにしている。

テストは実際に取得した Google ニュースのフィード (`tests/fixtures/`) を固定入力に使う。Processor はいずれも `Feed -> Feed` の純粋関数なので、単体テストで挙動を固定できる。

## ライセンス

MIT
