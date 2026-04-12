#!/usr/bin/env bash

CURRENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

"${CURRENT_DIR}/scripts/install.sh" 2>/dev/null || {
    tmux display-message "clux: failed to install binary. Run ${CURRENT_DIR}/scripts/install.sh manually."
    exit 1
}

KEY=$(tmux show-option -gqv @clux-key)
KEY="${KEY:-s}"

tmux bind-key "${KEY}" run-shell "${CURRENT_DIR}/scripts/session-select.sh"

CLAUDE_KEY=$(tmux show-option -gqv @clux-claude-key)
CLAUDE_KEY="${CLAUDE_KEY:-a}"
tmux bind-key "${CLAUDE_KEY}" run-shell "${CURRENT_DIR}/scripts/claude-picker.sh"

FILTER_BINDS=$(tmux show-option -gqv @clux-filter-binds)
if [[ -n "${FILTER_BINDS}" ]]; then
    IFS=',' read -ra PAIRS <<< "${FILTER_BINDS}"
    for pair in "${PAIRS[@]}"; do
        bind_key="${pair%%:*}"
        filter="${pair##*:}"
        bind_key=$(echo "${bind_key}" | xargs)
        filter=$(echo "${filter}" | xargs)
        if [[ -n "${bind_key}" && -n "${filter}" ]]; then
            tmux bind-key "${bind_key}" run-shell "${CURRENT_DIR}/scripts/session-select.sh ${filter}"
        fi
    done
fi
