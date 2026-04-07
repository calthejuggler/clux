#!/usr/bin/env bash

PLUGIN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BINARY="${PLUGIN_DIR}/bin/clux"

"${BINARY}" update 2>/dev/null

tmux choose-tree -s -Z -F '#{?pane_format,#{pane_current_command} "#{pane_title}",#{?window_format,#{window_name}#{window_flags} (#{window_panes} panes)#{?#{==:#{window_panes},1}, "#{pane_title}",},#{session_windows} windows#{?session_grouped, (group #{session_group}: #{session_group_list}),}#{?session_attached, (attached),}#{@clux_info}}}'
