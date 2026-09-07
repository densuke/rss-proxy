#!/bin/sh
# Linux コンテナ内で musl 静的ビルドを行い、テストと常駐時メモリを確認する。
#
# Apple Silicon では linux/arm64 が QEMU なしで動くため、速度もメモリ測定も
# 信用できる。CI (linux/amd64) に投げる前の事前確認として使う。
# amd64 と arm64 で挙動が変わりうるのはページサイズとコードサイズくらいで、
# musl のリンク可否や Linux 上の動作はここで捕まえられる。
#
# 環境変数で上書きできる: PLATFORM / IMAGE / MEMORY / CPUS / RUNTIME
set -eu

PLATFORM="${PLATFORM:-linux/arm64}"
IMAGE="${IMAGE:-docker.io/library/rust:alpine}"
# 静的リンクは ld が数 GB 使う。1GB では OOM で kill される
MEMORY="${MEMORY:-8g}"
CPUS="${CPUS:-6}"

# Apple の container、docker、podman のいずれでも動く
RUNTIME="${RUNTIME:-}"
if [ -z "$RUNTIME" ]; then
    for candidate in container docker podman; do
        if command -v "$candidate" >/dev/null 2>&1; then
            RUNTIME="$candidate"
            break
        fi
    done
fi
[ -n "$RUNTIME" ] || {
    echo "コンテナランタイムが見つかりません (container / docker / podman)" >&2
    exit 1
}

echo "runtime=$RUNTIME platform=$PLATFORM memory=$MEMORY cpus=$CPUS"

# ホスト (macOS) のビルド成果物と混ざらないよう、target ディレクトリを分ける
exec "$RUNTIME" run --rm --platform "$PLATFORM" -m "$MEMORY" -c "$CPUS" \
    -v "$(git rev-parse --show-toplevel):/work" -w /work \
    -e CARGO_TARGET_DIR=/tmp/target \
    "$IMAGE" sh -eu -c '
# busybox-extras は httpd 用。fixture をローカル配信して上流の代わりにする
apk add --no-cache musl-dev curl busybox-extras >/dev/null

echo "=== 環境"
uname -m
echo "page size: $(getconf PAGE_SIZE) bytes"
echo "available memory: $(awk "/MemAvailable/ {printf \"%.1f GB\", \$2/1024/1024}" /proc/meminfo)"

echo "=== テスト"
cargo test --quiet

echo "=== リリースビルド (musl 静的)"
cargo build --release --quiet
ls -l /tmp/target/release/rss-proxy

echo "=== 常駐時メモリ"
# fixture をローカルに配って上流の代わりにする。外部ネットワークに依存させない
mkdir -p /tmp/www
cp tests/fixtures/google_news_headline.xml /tmp/www/feed.xml
busybox-extras httpd -p 127.0.0.1:8081 -h /tmp/www

DB=$(mktemp -d)/rss.db
RP=/tmp/target/release/rss-proxy
$RP --db "$DB" feed add local http://127.0.0.1:8081/feed.xml >/dev/null
$RP --db "$DB" proc attach local google_news_cluster >/dev/null
$RP --db "$DB" proc attach local dedupe >/dev/null

$RP --db "$DB" serve --listen 127.0.0.1:8080 >/dev/null 2>&1 &
PID=$!

# 起動直後の巡回でフィードを取得・処理させ、配信できるまで待つ
i=0
while [ $i -lt 30 ]; do
    curl -sf -o /tmp/out.xml http://127.0.0.1:8080/feeds/local && break
    i=$((i + 1))
    sleep 1
done
[ -s /tmp/out.xml ] || { echo "配信されませんでした" >&2; exit 1; }

echo "配信サイズ: $(wc -c < /tmp/out.xml) bytes"
echo "item 数: $(grep -o "<item>" /tmp/out.xml | wc -l)"
echo "取得・処理・配信を 1 巡したあとの常駐プロセス:"
grep -E "VmRSS|VmHWM" "/proc/$PID/status"
kill $PID
'
