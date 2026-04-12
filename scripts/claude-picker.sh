#!/usr/bin/env bash

PLUGIN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BINARY="${PLUGIN_DIR}/bin/clux"

LINES=$("${BINARY}" list 2>/dev/null)

if [[ -z "${LINES}" ]]; then
    tmux display-message "clux: no Claude sessions found"
    exit 0
fi

format_rows() {
    while IFS=$'\t' read -r target state mode tasks agents summary cwd session_name; do
        if [[ ${#summary} -gt 40 ]]; then
            summary="${summary:0:37}..."
        fi
        if [[ ${#cwd} -gt 25 ]]; then
            cwd="${cwd:0:22}..."
        fi
        printf "%s\t%-7s  %-11s  %5s  %6s  %-40s  %-25s  %s\n" \
            "${target}" "${state}" "${mode}" "${tasks}" "${agents}" "${summary}" "${cwd}" "${session_name}"
    done <<< "${LINES}"
}

pick_with_fzf() {
    local header
    header=$(printf "%-7s  %-11s  %5s  %6s  %-40s  %-25s  %s" \
        "STATE" "MODE" "TASKS" "AGENTS" "SUMMARY" "CWD" "SESSION")

    local rows
    rows=$(format_rows) || true

    local selected
    selected=$(echo "${rows}" | fzf-tmux -p 80%,50% \
        --delimiter=$'\t' \
        --with-nth=2.. \
        --header="${header}" \
        --no-preview \
        --reverse) || true

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
