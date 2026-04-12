#!/usr/bin/env bash

set -euo pipefail

REPO="calthejuggler/clux"

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

install_standalone() {
    local platform
    platform="$(get_platform)"

    local dest="${1:-${HOME}/.local/bin/clux}"
    local dest_dir
    dest_dir="$(dirname "${dest}")"

    local asset="clux-${platform}"
    local url="https://github.com/${REPO}/releases/latest/download/${asset}"

    echo "clux: downloading ${asset}..."
    mkdir -p "${dest_dir}"

    local download_exit
    download_exit=0
    set +e
    download "${url}" "${dest}"
    download_exit=$?
    set -e

    if [[ "${download_exit}" -eq 0 ]]; then
        chmod +x "${dest}"
        echo "clux: installed to ${dest}"
        return 0
    fi

    echo "clux: download failed. Install from https://github.com/${REPO}/releases" >&2
    return 1
}

install_plugin() {
    local plugin_dir="$1"
    local bin_dir="${plugin_dir}/bin"

    local platform
    platform="$(get_platform)"

    local version_file="${bin_dir}/.version"
    local current_version=""
    local expected_version
    expected_version="$(grep '^version' "${plugin_dir}/Cargo.toml" | head -1 | sed 's/.*"\(.*\)"/\1/')"

    if [[ -f "${version_file}" ]]; then
        current_version="$(cat "${version_file}")"
    fi

    if [[ -f "${bin_dir}/clux" && "${current_version}" == "${expected_version}" ]]; then
        return 0
    fi

    local asset="clux-${platform}"
    local url="https://github.com/${REPO}/releases/latest/download/${asset}"

    echo "clux: downloading ${asset}..."
    mkdir -p "${bin_dir}"

    local download_exit
    download_exit=0
    set +e
    download "${url}" "${bin_dir}/clux"
    download_exit=$?
    set -e

    if [[ "${download_exit}" -eq 0 ]]; then
        chmod +x "${bin_dir}/clux"
        echo "${expected_version}" > "${version_file}"
        echo "clux: installed v${expected_version} successfully"
        return 0
    fi

    echo "clux: download failed, attempting cargo build as fallback..." >&2
    if command -v cargo &>/dev/null; then
        (cd "${plugin_dir}" && cargo build --release)
        mkdir -p "${bin_dir}"
        cp "${plugin_dir}/target/release/clux" "${bin_dir}/clux"
        echo "${expected_version}" > "${version_file}"
        echo "clux: built v${expected_version} from source successfully"
        return 0
    fi

    echo "clux: installation failed. Install from https://github.com/${REPO}/releases" >&2
    return 1
}

main() {
    local script_dir
    script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    local plugin_dir="${script_dir}/.."

    if [[ -f "${plugin_dir}/Cargo.toml" ]]; then
        install_plugin "${plugin_dir}"
    else
        install_standalone "${1:-}"
    fi
}

main "$@"
