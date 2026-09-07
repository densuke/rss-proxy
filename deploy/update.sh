#!/bin/sh
# 最新リリースを取得し、チェックサムを検証して差し替え、サービスを再起動する。
set -eu

REPO="${REPO:-densuke/rss-proxy}"
DEST="${DEST:-/usr/local/bin/rss-proxy}"
SERVICE="${SERVICE:-rss-proxy}"
NAME="rss-proxy-x86_64-unknown-linux-musl.tar.gz"

TAG=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" |
      sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p')
[ -n "$TAG" ] || { echo "最新リリースを特定できません" >&2; exit 1; }
echo "installing ${TAG}"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
BASE="https://github.com/${REPO}/releases/download/${TAG}"

curl -fsSL -o "${WORK}/${NAME}" "${BASE}/${NAME}"
curl -fsSL -o "${WORK}/${NAME}.sha256" "${BASE}/${NAME}.sha256"
(cd "$WORK" && sha256sum -c "${NAME}.sha256")

tar -C "$WORK" -xzf "${WORK}/${NAME}"
install -m 0755 "${WORK}/rss-proxy" "$DEST"
systemctl restart "$SERVICE"
echo "done: $("$DEST" --version)"
