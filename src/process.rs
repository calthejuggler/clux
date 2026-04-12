use crate::tmux::PaneInfo;
use std::collections::HashMap;

#[cfg(target_os = "linux")]
pub struct ProcessTree;

#[cfg(target_os = "linux")]
impl ProcessTree {
    pub const fn build() -> Self {
        Self
    }
}

#[cfg(target_os = "linux")]
fn get_ppid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rsplit_once(')')?.1;
    after_comm.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(target_os = "linux")]
pub fn find_tmux_pane<'map>(
    pid: u32,
    pane_map: &'map HashMap<u32, PaneInfo>,
    _tree: &ProcessTree,
) -> Option<&'map PaneInfo> {
    let mut current = pid;
    for _ in 0..20 {
        let ppid = get_ppid(current)?;
        if let Some(value) = pane_map.get(&ppid) {
            return Some(value);
        }
        if ppid <= 1 {
            return None;
        }
        current = ppid;
    }
    None
}

#[cfg(target_os = "macos")]
pub struct ProcessTree(HashMap<u32, u32>);

#[cfg(target_os = "macos")]
impl ProcessTree {
    pub fn build() -> Self {
        let mut tree = HashMap::new();
        let Ok(output) = std::process::Command::new("ps")
            .args(["-eo", "pid,ppid"])
            .output()
        else {
            return Self(tree);
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines().skip(1) {
            let mut fields = line.split_whitespace();
            if let (Some(pid_str), Some(ppid_str)) = (fields.next(), fields.next())
                && let (Ok(pid), Ok(ppid)) = (pid_str.parse::<u32>(), ppid_str.parse::<u32>())
            {
                let _ = tree.insert(pid, ppid);
            }
        }
        Self(tree)
    }
}

#[cfg(target_os = "macos")]
pub fn find_tmux_pane<'map>(
    pid: u32,
    pane_map: &'map HashMap<u32, PaneInfo>,
    tree: &ProcessTree,
) -> Option<&'map PaneInfo> {
    let mut current = pid;
    for _ in 0..20 {
        let ppid = tree.0.get(&current)?;
        if let Some(value) = pane_map.get(ppid) {
            return Some(value);
        }
        if *ppid <= 1 {
            return None;
        }
        current = *ppid;
    }
    None
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests {
    use super::*;

    #[test]
    fn find_tmux_pane_direct_parent() {
        let mut pane_map = HashMap::new();
        let current_pid = std::process::id();
        let ppid = get_ppid(current_pid).expect("should have a parent");

        let _ = pane_map.insert(
            ppid,
            PaneInfo {
                session_name: "test".to_owned(),
                target: "test:0.0".to_owned(),
            },
        );
        let tree = ProcessTree::build();

        let result = find_tmux_pane(current_pid, &pane_map, &tree);
        assert!(result.is_some());
        assert_eq!(result.expect("pane").session_name, "test");
    }

    #[test]
    fn find_tmux_pane_not_found() {
        let pane_map = HashMap::new();
        let tree = ProcessTree::build();
        let result = find_tmux_pane(std::process::id(), &pane_map, &tree);
        assert!(result.is_none());
    }

    #[test]
    fn find_tmux_pane_stops_at_init() {
        let pane_map = HashMap::new();
        let tree = ProcessTree::build();
        let result = find_tmux_pane(1, &pane_map, &tree);
        assert!(result.is_none());
    }
}
