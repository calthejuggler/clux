#!/usr/bin/env bash

CURRENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${CURRENT_DIR}/bin/clux"

if [[ ! -f "${BINARY}" ]]; then
    "${CURRENT_DIR}/scripts/install.sh" || {
        tmux display-message "clux: failed to install binary. Run ${CURRENT_DIR}/scripts/install.sh manually."
        exit 1
    }
fi

KEY=$(tmux show-option -gqv @clux-key)
KEY="${KEY:-s}"

tmux bind-key "${KEY}" run-shell "${CURRENT_DIR}/scripts/session-select.sh"
