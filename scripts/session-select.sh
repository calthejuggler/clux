#!/usr/bin/env bash

PLUGIN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BINARY="${PLUGIN_DIR}/bin/clux"
FILTER="${1:-all}"

"${BINARY}" update "${FILTER}" 2>/dev/null

FILTER_FLAG=()
if [[ "${FILTER}" != "all" ]]; then
    FILTER_FLAG=(-f '#{@clux_visible}')
fi

tmux choose-tree -s -Z "${FILTER_FLAG[@]}" -F '#{?pane_format,#{pane_current_command} "#{pane_title}",#{?window_format,#{window_name}#{window_flags} (#{window_panes} panes)#{?#{==:#{window_panes},1}, "#{pane_title}",},#{session_windows} windows#{?session_grouped, (group #{session_group}: #{session_group_list}),}#{?session_attached, (attached),}#{@clux_info}}}'
