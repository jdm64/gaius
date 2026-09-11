#!/usr/bin/env sh
# Install script for gaius (https://github.com/jdm64/gaius)
# Downloads the latest release build for linux-x86_64 and
# installs it into ~/.local/bin

set -eu

REPO_URL="https://github.com/jdm64/gaius/releases/latest/download/gaius-linux-x86_64"
BIN_NAME="gaius"
INSTALL_DIR="${HOME}/.local/bin"
INSTALL_PATH="${INSTALL_DIR}/${BIN_NAME}"

# Sanity checks -------------------------------------------------------------
if [ "$(uname -s)" != "Linux" ]; then
    echo "error: this script only supports Linux." >&2
    exit 1
fi

if [ "$(uname -m)" != "x86_64" ]; then
    echo "error: this script only supports x86_64 architectures." >&2
    exit 1
fi

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
