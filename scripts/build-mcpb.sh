#!/usr/bin/env bash
#
# build-mcpb.sh — package the envyou MCP server as a Claude Desktop extension
# (.mcpb). Assembles a bundle directory from packaging/mcpb/manifest.json plus a
# pre-built envyou binary, syncs the manifest version to the workspace version,
# then packs it with the official `mcpb` CLI (falling back to `zip`, since a
# .mcpb is a zip archive). Emits a SHA-256 checksum next to the artifact.
#
# Usage:
#   scripts/build-mcpb.sh --binary <path-to-envyou> --os <darwin|win32> [--out <dir>]
#
# Examples:
#   scripts/build-mcpb.sh --binary target/release/envyou --os darwin
#   scripts/build-mcpb.sh --binary target/release/envyou.exe --os win32 --out dist
#
# Notes:
# * The bundle is PER-PLATFORM: install the darwin build on macOS and the win32
#   build on Windows (each contains only that platform's binary).
# * Code signing / notarization must happen on the binary BEFORE packing — see
#   packaging/mcpb/README.md. This script does not sign.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST_SRC="$REPO_ROOT/packaging/mcpb/manifest.json"

BINARY=""
OS=""
OUT_DIR="$REPO_ROOT/dist"

while [ $# -gt 0 ]; do
  case "$1" in
    --binary) BINARY="$2"; shift 2 ;;
    --os)     OS="$2"; shift 2 ;;
    --out)    OUT_DIR="$2"; shift 2 ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$BINARY" ] || [ -z "$OS" ]; then
  echo "error: --binary and --os are required (--os darwin|win32)" >&2
  exit 2
fi
if [ ! -f "$BINARY" ]; then
  echo "error: binary not found: $BINARY" >&2
  exit 1
fi
case "$OS" in
  darwin) SERVER_BIN="server/envyou" ;;
  win32)  SERVER_BIN="server/envyou.exe" ;;
  *) echo "error: --os must be 'darwin' or 'win32' (got '$OS')" >&2; exit 2 ;;
esac

# Resolve the workspace version so the bundle version can't drift from the app.
VERSION="$(python3 - "$REPO_ROOT/Cargo.toml" <<'PY'
import re, sys
text = open(sys.argv[1], encoding="utf-8").read()
m = re.search(r'^\s*version\s*=\s*"([^"]+)"', text, re.M)
print(m.group(1) if m else "0.0.0")
PY
)"

echo "Packaging envyou $VERSION for $OS from $BINARY"

BUILD_DIR="$(mktemp -d)"
trap 'rm -rf "$BUILD_DIR"' EXIT
mkdir -p "$BUILD_DIR/server"

# 1) Copy the manifest and stamp it with the resolved version.
python3 - "$MANIFEST_SRC" "$BUILD_DIR/manifest.json" "$VERSION" <<'PY'
import json, sys
src, dst, version = sys.argv[1], sys.argv[2], sys.argv[3]
with open(src, encoding="utf-8") as f:
    m = json.load(f)
m["version"] = version
with open(dst, "w", encoding="utf-8") as f:
    json.dump(m, f, indent=2)
    f.write("\n")
PY

# 2) Place the binary at the manifest's entry point.
cp "$BINARY" "$BUILD_DIR/$SERVER_BIN"
chmod +x "$BUILD_DIR/$SERVER_BIN" 2>/dev/null || true

# 3) Optional icon.
if [ -f "$REPO_ROOT/packaging/mcpb/icon.png" ]; then
  cp "$REPO_ROOT/packaging/mcpb/icon.png" "$BUILD_DIR/icon.png"
fi

mkdir -p "$OUT_DIR"
OUT_FILE="$OUT_DIR/envyou-$OS.mcpb"
rm -f "$OUT_FILE"

# 4) Pack. Prefer the official CLI (validates the manifest); fall back to zip.
if command -v mcpb >/dev/null 2>&1; then
  mcpb validate "$BUILD_DIR/manifest.json"
  mcpb pack "$BUILD_DIR" "$OUT_FILE"
elif command -v npx >/dev/null 2>&1 && npx --no-install @anthropic-ai/mcpb --version >/dev/null 2>&1; then
  npx --no-install @anthropic-ai/mcpb validate "$BUILD_DIR/manifest.json"
  npx --no-install @anthropic-ai/mcpb pack "$BUILD_DIR" "$OUT_FILE"
else
  echo "warning: mcpb CLI not found; falling back to a plain zip (still a valid .mcpb)." >&2
  ( cd "$BUILD_DIR" && zip -q -r -X "$OUT_FILE" manifest.json server $( [ -f icon.png ] && echo icon.png ) )
fi

# 5) Checksum.
if command -v shasum >/dev/null 2>&1; then
  ( cd "$OUT_DIR" && shasum -a 256 "envyou-$OS.mcpb" > "envyou-$OS.mcpb.sha256" )
elif command -v sha256sum >/dev/null 2>&1; then
  ( cd "$OUT_DIR" && sha256sum "envyou-$OS.mcpb" > "envyou-$OS.mcpb.sha256" )
fi

echo "Built $OUT_FILE"
