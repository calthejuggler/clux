#!/usr/bin/env bash

PLUGIN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BINARY="${PLUGIN_DIR}/bin/clux"

LINES=$("${BINARY}" list 2>/dev/null)

if [[ -z "${LINES}" ]]; then
    tmux display-message "clux: no Claude sessions found"
    exit 0
fi

pick_with_fzf() {
    local selected
    selected=$(echo "${LINES}" | fzf-tmux -p 80%,50% \
        --delimiter=$'\t' \
        --with-nth=2.. \
        --header='Claude Sessions (state | mode | tasks | agents | summary | cwd | tmux session)' \
        --no-preview \
        --reverse)

    if [[ -n "${selected}" ]]; then
        local target
        target=$(echo "${selected}" | cut -f1)
        tmux switch-client -t "${target}"
    fi
}

pick_with_menu() {
    local args=()
    local idx=0
    while IFS=$'\t' read -r target state mode tasks agents summary cwd session_name; do
        local label="${state} | ${mode} | ${tasks} tasks | ${agents} agents | ${summary} | ${cwd} (${session_name})"
        if [[ ${#label} -gt 70 ]]; then
            label="${label:0:67}..."
        fi
        args+=("${label}" "" "switch-client -t '${target}'")
        idx=$((idx + 1))
    done <<< "${LINES}"

    tmux display-menu -T "Claude Sessions" "${args[@]}"
}

if command -v fzf-tmux &>/dev/null; then
    pick_with_fzf
else
    pick_with_menu
fi
