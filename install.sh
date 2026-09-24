#!/usr/bin/env sh
# Install script for gaius (https://github.com/jdm64/gaius)
# Downloads the release build for this OS (Linux x86_64 or macOS arm64)
# and installs it into ~/.local/bin

set -eu

BUILD="latest"

usage() {
    echo "Usage: $0 [--build <latest|branch|version>]"
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --build)
            if [ "$#" -lt 2 ] || [ -z "$2" ]; then
                echo "error: --build requires a branch name or version." >&2
                usage >&2
                exit 1
            fi
            BUILD="$2"
            shift 2
            ;;
        --build=*)
            BUILD="${1#--build=}"
            if [ -z "$BUILD" ]; then
                echo "error: --build requires a branch name or version." >&2
                usage >&2
                exit 1
            fi
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown option: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

case "$BUILD" in
    latest)
        RELEASE="latest/download"
        ;;
    v[0-9]?*)
        RELEASE="download/${BUILD}"
        ;;
    *)
        RELEASE="download/continuous-${BUILD}"
        ;;
esac

# Platform ------------------------------------------------------------------
OS="$(uname -s)"
ARCH="$(uname -m)"

case "${OS}" in
    Linux)
        case "${ARCH}" in
            x86_64)
                ASSET="gaius-linux-x86_64"
                ;;
            *)
                echo "error: unsupported Linux architecture '${ARCH}'. Only x86_64 is supported." >&2
                exit 1
                ;;
        esac
        ;;
    Darwin)
        case "${ARCH}" in
            arm64|aarch64)
                ASSET="gaius-macos-aarch64"
                ;;
            *)
                echo "error: unsupported macOS architecture '${ARCH}'. Only Apple Silicon (arm64) is supported." >&2
                exit 1
                ;;
        esac
        ;;
    *)
        echo "error: unsupported operating system '${OS}'. Only Linux and macOS are supported." >&2
        exit 1
        ;;
esac

REPO_URL="https://github.com/jdm64/gaius/releases/${RELEASE}/${ASSET}"
BIN_NAME="gaius"
INSTALL_DIR="${HOME}/.local/bin"
INSTALL_PATH="${INSTALL_DIR}/${BIN_NAME}"

# Sanity checks -------------------------------------------------------------

command -v curl >/dev/null 2>&1 || {
    echo "error: 'curl' is required but was not found in PATH." >&2
    exit 1
}

# Download ------------------------------------------------------------------
echo "==> Creating ${INSTALL_DIR}"
mkdir -p "${INSTALL_DIR}"

TMP_FILE="$(mktemp)"
trap 'rm -f "${TMP_FILE}"' EXIT INT TERM

echo "==> Downloading ${REPO_URL}"
if ! curl --fail --location --silent --show-error \
          --output "${TMP_FILE}" "${REPO_URL}"; then
    echo "error: download failed. Check your connection or the URL." >&2
    exit 1
fi

chmod +x "${TMP_FILE}"

echo "==> Installing to ${INSTALL_PATH}"
mv -f "${TMP_FILE}" "${INSTALL_PATH}"
trap - EXIT INT TERM

# Verify --------------------------------------------------------------------
if ! command -v "${BIN_NAME}" >/dev/null 2>&1; then
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) ;;
        *)
            echo ""
            echo "note: ${INSTALL_DIR} is not in your PATH."
            echo "      Add the following to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
            echo ""
            echo "          export PATH=\"\$HOME/.local/bin:\$PATH\""
            ;;
    esac
fi

echo "==> Done! Run '${BIN_NAME} --help' to get started."
