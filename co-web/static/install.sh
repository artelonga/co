#!/usr/bin/env sh
# CO install script — downloads the latest co binary for the current platform.
# Usage: curl -fsSL https://co.artelonga.com.br/install.sh | sh
set -e

REPO="artelonga/co"
INSTALL_DIR="/usr/local/bin"

OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS" in
  darwin)
    case "$ARCH" in
      x86_64) TARGET="x86_64-apple-darwin" ;;
      arm64)  TARGET="aarch64-apple-darwin" ;;
      *)      echo "Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    ;;
  linux)
    case "$ARCH" in
      x86_64)  TARGET="x86_64-unknown-linux-gnu" ;;
      aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
      arm64)   TARGET="aarch64-unknown-linux-gnu" ;;
      *)       echo "Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    ;;
  *)
    echo "Unsupported OS: $OS"
    echo "Windows users: download from https://github.com/${REPO}/releases"
    exit 1
    ;;
esac

VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
  | grep '"tag_name"' | sed 's/.*"tag_name": "\(.*\)".*/\1/')
if [ -z "$VERSION" ]; then
  echo "Failed to fetch latest version from GitHub."; exit 1
fi

ARCHIVE="co-${VERSION}-${TARGET}.tar.gz"
URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE}"

echo "Installing co ${VERSION} for ${TARGET}..."
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

curl -fsSL "$URL" -o "$TMP/$ARCHIVE"
tar -xzf "$TMP/$ARCHIVE" -C "$TMP"
chmod +x "$TMP/co"
sudo install -m 755 "$TMP/co" "$INSTALL_DIR/co"

echo "✓ co ${VERSION} installed to ${INSTALL_DIR}/co"
echo ""
echo "  Run: co serve --open"
