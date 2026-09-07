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

`label` は管理画面の表示だけでなく、**配信する XML の `<title>` も置き換える**。Google ニュースの検索フィードは channel のタイトルが検索クエリそのままになるため、購読前に読みやすい名前を付けられる。

識別子を省略すると `/feeds/k7Rm2xQvB8nT4wLpZaYcDf` のような推測しにくい URL になる。ただしこれは軽い目隠しであって認証ではない。URL は RSS リーダーの同期先やプロキシのログにも残る。

識別子は後から `feed set --new-slug` や管理画面で変更できる。**変更すると配信 URL が変わり、購読中の登録が切れる。**

## Processor

順序つきの連鎖として登録する。同じ種別を異なるパラメータで複数回登録してもよい。

連鎖は 2 段ある。**全フィード共通の連鎖が先に走り、その後でフィード固有の連鎖が走る。**

```
[取得] → 全フィード共通 → フィード固有 → [配信]
```

順序が効く。全角の正規化を共通側で済ませておけば、フィード側の除外条件を半角で書ける。

新しい DB には既定の共通連鎖が入る。全角の正規化と、広告記事の除去。

```console
$ rss-proxy global show
0. normalize_width {"target":"both"}
1. exclude {"words":["【PR】","[PR]","PR:","【広告】","[広告]","<PR>","(PR)"],"target":"title"}
```

不要なら管理画面か `rss-proxy global detach` で外せる。

| 種別 | 内容 | パラメータ |
|------|------|-----------|
| `google_news_cluster` | Google ニュースの description から、title と重複する先頭要素を取り除く。関連記事は残す | なし |
| `exclude` | 指定した語を含む item を取り除く | `words`: 語の配列<br>`target`: `title` / `description` / `both` (既定 `title`) |
| `max_age` | 指定した時間より古い item を落とす | `hours` (既定 24) |
| `normalize_width` | 全角の英数字と記号を半角に直す。カギ括弧・句読点・なかてん・波ダッシュはそのまま | `target` (既定 `title`) |
| `dedupe` | item 間の重複除去 | `key`: `guid` / `link` / `normalized_title` (既定 `link`) |

```console
$ rss-proxy proc attach gnews dedupe --params '{"key":"link"}'
$ rss-proxy feed show gnews
```

Web UI では 1 行に 1 つ「種別 パラメータ(JSON)」の形式で書く。行の並びが適用順になる。

```
google_news_cluster
exclude {"words":["スポーツ","競馬","ABEMA"]}
max_age {"hours":24}
normalize_width
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
rss-proxy proc show <feed>
rss-proxy proc attach <feed> <kind> [--params JSON] [--at N]
rss-proxy proc detach <feed> <position>
rss-proxy proc move <feed> <from> <to>

rss-proxy global show                # 全フィード共通の連鎖
rss-proxy global attach <kind> [--params JSON] [--at N]
rss-proxy global detach <position>
rss-proxy global move <from> <to>
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
$ cargo llvm-cov --summary-only    # カバレッジ
```

カバレッジに数値目標は置いていない。目標にするとテストが実装をなぞる方向に寄り、リファクタのたびに壊れて変更を妨げる。「壊れたときに取り返しがつくか」でテストの要否を決めている。詳細は [DESIGN.md](DESIGN.md) の 11 章。

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
