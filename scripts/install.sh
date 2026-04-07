#!/usr/bin/env bash

set -euo pipefail

REPO="calthejuggler/clux"
PLUGIN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${PLUGIN_DIR}/bin"

get_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "${os}" in
        Linux)  os="linux" ;;
        Darwin) os="darwin" ;;
        *)
            echo "Unsupported OS: ${os}" >&2
            return 1
            ;;
    esac

    case "${arch}" in
        x86_64|amd64)  arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *)
            echo "Unsupported architecture: ${arch}" >&2
            return 1
            ;;
    esac

    echo "${os}-${arch}"
}

download() {
    local url="$1" dest="$2"
    if command -v curl &>/dev/null; then
        curl -fsSL -o "${dest}" "${url}"
    elif command -v wget &>/dev/null; then
        wget -q -O "${dest}" "${url}"
    else
        echo "Neither curl nor wget found" >&2
        return 1
    fi
}

main() {
    local platform
    platform="$(get_platform)"
    # shellcheck disable=SC2181
    if [[ $? -ne 0 ]]; then
        exit 1
    fi

    local version_file="${BIN_DIR}/.version"
    local current_version=""
    local expected_version
    expected_version="$(grep '^version' "${PLUGIN_DIR}/Cargo.toml" | head -1 | sed 's/.*"\(.*\)"/\1/')"

    if [[ -f "${version_file}" ]]; then
        current_version="$(cat "${version_file}")"
    fi

    if [[ -f "${BIN_DIR}/clux" && "${current_version}" == "${expected_version}" ]]; then
        return 0
    fi

    local asset="clux-${platform}"
    local url="https://github.com/${REPO}/releases/latest/download/${asset}"

    echo "clux: downloading ${asset}..."
    mkdir -p "${BIN_DIR}"

    local download_exit
    download_exit=0
    # SC2310: capture exit code without using function in a condition
    set +e
    download "${url}" "${BIN_DIR}/clux"
    download_exit=$?
    set -e

    if [[ "${download_exit}" -eq 0 ]]; then
        chmod +x "${BIN_DIR}/clux"
        echo "${expected_version}" > "${version_file}"
        echo "clux: installed v${expected_version} successfully"
        return 0
    fi

    echo "clux: download failed, attempting cargo build as fallback..." >&2
    if command -v cargo &>/dev/null; then
        (cd "${PLUGIN_DIR}" && cargo build --release)
        mkdir -p "${BIN_DIR}"
        cp "${PLUGIN_DIR}/target/release/clux" "${BIN_DIR}/clux"
        echo "${expected_version}" > "${version_file}"
        echo "clux: built v${expected_version} from source successfully"
        return 0
    fi

    echo "clux: installation failed. Install from https://github.com/${REPO}/releases" >&2
    return 1
}

main
