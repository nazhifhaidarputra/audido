#!/bin/sh

set -eu

AUDIDO_REPOSITORY=${AUDIDO_REPOSITORY:-nazhifhaidarputra/audido}
AUDIDO_VERSION=${AUDIDO_VERSION:-}
AUDIDO_INSTALL_DIR=${AUDIDO_INSTALL_DIR:-}
AUDIDO_FROM_SOURCE=0
AUDIDO_TMP_DIR=

usage() {
    cat <<'EOF'
Install Audido from a GitHub release.

Usage: install.sh [options]

Options:
  --version VERSION    Install a specific version (for example, 0.1.1)
  --prefix DIRECTORY  Install into DIRECTORY/bin
  --install-dir DIR   Install executables directly into DIR
  --from-source       Build locally instead of using a release binary
  -h, --help          Show this help

Environment variables:
  AUDIDO_REPOSITORY   GitHub owner/repository (default: nazhifhaidarputra/audido)
  AUDIDO_VERSION      Version to install
  AUDIDO_INSTALL_DIR  Executable destination
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || { echo "--version requires a value" >&2; exit 2; }
            AUDIDO_VERSION=${2#v}
            shift 2
            ;;
        --prefix)
            [ "$#" -ge 2 ] || { echo "--prefix requires a value" >&2; exit 2; }
            AUDIDO_INSTALL_DIR=$2/bin
            shift 2
            ;;
        --install-dir)
            [ "$#" -ge 2 ] || { echo "--install-dir requires a value" >&2; exit 2; }
            AUDIDO_INSTALL_DIR=$2
            shift 2
            ;;
        --from-source)
            AUDIDO_FROM_SOURCE=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [ -z "$AUDIDO_INSTALL_DIR" ]; then
    if [ "$(id -u)" -eq 0 ]; then
        AUDIDO_INSTALL_DIR=/usr/local/bin
    else
        AUDIDO_INSTALL_DIR=${HOME:?HOME is not set}/.local/bin
    fi
fi

command -v curl >/dev/null 2>&1 || {
    echo "curl is required to install Audido" >&2
    exit 1
}
command -v tar >/dev/null 2>&1 || {
    echo "tar is required to install Audido" >&2
    exit 1
}

if [ -z "$AUDIDO_VERSION" ]; then
    AUDIDO_LATEST_URL=$(curl -fsSL -o /dev/null -w '%{url_effective}' \
        "https://github.com/${AUDIDO_REPOSITORY}/releases/latest")
    AUDIDO_VERSION=${AUDIDO_LATEST_URL##*/}
    AUDIDO_VERSION=${AUDIDO_VERSION#v}
fi

case "$AUDIDO_VERSION" in
    ''|*[!0-9A-Za-z.-]*)
        echo "Invalid version: $AUDIDO_VERSION" >&2
        exit 1
        ;;
esac

AUDIDO_OS=$(uname -s)
AUDIDO_ARCH=$(uname -m)
AUDIDO_PLATFORM=

case "$AUDIDO_OS:$AUDIDO_ARCH" in
    Linux:x86_64|Linux:amd64)
        AUDIDO_PLATFORM=linux-x86_64
        if command -v ldd >/dev/null 2>&1 && ldd --version 2>&1 | grep -qi musl; then
            AUDIDO_FROM_SOURCE=1
        fi
        ;;
    Darwin:arm64|Darwin:aarch64)
        AUDIDO_PLATFORM=macos-aarch64
        ;;
    Darwin:x86_64|Darwin:amd64)
        AUDIDO_PLATFORM=macos-x86_64
        ;;
    *)
        AUDIDO_FROM_SOURCE=1
        ;;
esac

AUDIDO_TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/audido.XXXXXX")
cleanup() {
    if [ -n "$AUDIDO_TMP_DIR" ] && [ -d "$AUDIDO_TMP_DIR" ]; then
        rm -rf "$AUDIDO_TMP_DIR"
    fi
}
trap cleanup 0 1 2 15

AUDIDO_BASE_URL="https://github.com/${AUDIDO_REPOSITORY}/releases/download/v${AUDIDO_VERSION}"

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        echo "sha256sum or shasum is required" >&2
        return 1
    fi
}

if [ "$AUDIDO_FROM_SOURCE" -eq 0 ]; then
    AUDIDO_ASSET="audido-${AUDIDO_VERSION}-${AUDIDO_PLATFORM}.tar.gz"
    echo "Downloading Audido ${AUDIDO_VERSION} for ${AUDIDO_PLATFORM}..."
    curl -fL --retry 3 -o "$AUDIDO_TMP_DIR/$AUDIDO_ASSET" "$AUDIDO_BASE_URL/$AUDIDO_ASSET"
    curl -fL --retry 3 -o "$AUDIDO_TMP_DIR/SHA256SUMS" "$AUDIDO_BASE_URL/SHA256SUMS"

    AUDIDO_EXPECTED_HASH=$(awk -v asset="$AUDIDO_ASSET" '$2 == asset || $2 == ("*" asset) {print $1; exit}' \
        "$AUDIDO_TMP_DIR/SHA256SUMS")
    AUDIDO_ACTUAL_HASH=$(sha256_file "$AUDIDO_TMP_DIR/$AUDIDO_ASSET")
    if [ -z "$AUDIDO_EXPECTED_HASH" ] || [ "$AUDIDO_EXPECTED_HASH" != "$AUDIDO_ACTUAL_HASH" ]; then
        echo "Checksum verification failed for $AUDIDO_ASSET" >&2
        exit 1
    fi

    mkdir -p "$AUDIDO_TMP_DIR/archive"
    tar -xzf "$AUDIDO_TMP_DIR/$AUDIDO_ASSET" -C "$AUDIDO_TMP_DIR/archive"
    AUDIDO_BINARY="$AUDIDO_TMP_DIR/archive/audido-tui"
else
    command -v cargo >/dev/null 2>&1 || {
        echo "Rust/Cargo is required for a source build: https://rustup.rs" >&2
        exit 1
    }
    if [ "$AUDIDO_OS" = Linux ] && ! command -v pkg-config >/dev/null 2>&1; then
        echo "A source build needs pkg-config and ALSA development headers." >&2
        echo "Examples: apt install pkg-config libasound2-dev; apk add pkgconf alsa-lib-dev" >&2
        exit 1
    fi

    AUDIDO_SOURCE_ARCHIVE="$AUDIDO_TMP_DIR/source.tar.gz"
    echo "Building Audido ${AUDIDO_VERSION} from source for ${AUDIDO_OS}/${AUDIDO_ARCH}..."
    curl -fL --retry 3 -o "$AUDIDO_SOURCE_ARCHIVE" \
        "https://github.com/${AUDIDO_REPOSITORY}/archive/refs/tags/v${AUDIDO_VERSION}.tar.gz"
    mkdir -p "$AUDIDO_TMP_DIR/source"
    tar -xzf "$AUDIDO_SOURCE_ARCHIVE" -C "$AUDIDO_TMP_DIR/source" --strip-components=1
    cargo build --release --locked --bin audido-tui \
        --target-dir "$AUDIDO_TMP_DIR/target" \
        --manifest-path "$AUDIDO_TMP_DIR/source/Cargo.toml"
    AUDIDO_BINARY="$AUDIDO_TMP_DIR/target/release/audido-tui"
fi

[ -x "$AUDIDO_BINARY" ] || {
    echo "Built package did not contain an executable audido-tui binary" >&2
    exit 1
}

mkdir -p "$AUDIDO_INSTALL_DIR"
install -m 0755 "$AUDIDO_BINARY" "$AUDIDO_INSTALL_DIR/audido-tui"
ln -sfn audido-tui "$AUDIDO_INSTALL_DIR/audido"

echo "Audido ${AUDIDO_VERSION} installed to $AUDIDO_INSTALL_DIR/audido-tui"
case ":${PATH}:" in
    *:"$AUDIDO_INSTALL_DIR":*) ;;
    *) echo "Add $AUDIDO_INSTALL_DIR to PATH to run 'audido'." ;;
esac

AUDIDO_MISSING_TOOLS=
command -v ffmpeg >/dev/null 2>&1 || AUDIDO_MISSING_TOOLS="ffmpeg"
if ! command -v yt-dlp >/dev/null 2>&1; then
    AUDIDO_MISSING_TOOLS="${AUDIDO_MISSING_TOOLS}${AUDIDO_MISSING_TOOLS:+ and }yt-dlp"
fi
if [ -n "$AUDIDO_MISSING_TOOLS" ]; then
    echo "Note: install $AUDIDO_MISSING_TOOLS to enable YouTube playback."
fi
