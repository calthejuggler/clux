use crate::tmux::PaneInfo;
use std::collections::HashMap;

#[cfg(target_os = "linux")]
pub struct ProcessTree {
    proc_available: bool,
}

#[cfg(target_os = "linux")]
impl ProcessTree {
    pub fn build() -> Self {
        Self {
            proc_available: std::path::Path::new("/proc").is_dir(),
        }
    }

    pub fn is_alive(&self, pid: u32) -> bool {
        self.proc_available && std::path::Path::new(&format!("/proc/{pid}")).exists()
    }

    pub fn has_children(&self, pid: u32) -> bool {
        if !self.proc_available {
            return false;
        }
        let children_path = format!("/proc/{pid}/task/{pid}/children");
        std::fs::read_to_string(&children_path)
            .ok()
            .is_some_and(|content| !content.trim().is_empty())
    }

    pub fn descendants_of(&self, pid: u32) -> Vec<u32> {
        if !self.proc_available {
            return Vec::new();
        }
        let task_dir = format!("/proc/{pid}/task");
        let Ok(entries) = std::fs::read_dir(&task_dir) else {
            return Vec::new();
        };

        let mut result = Vec::new();
        for entry in entries.flatten() {
            let children_path = entry.path().join("children");
            if let Ok(content) = std::fs::read_to_string(&children_path) {
                for pid_str in content.split_whitespace() {
                    if let Ok(child) = pid_str.parse::<u32>() {
                        result.push(child);
                        collect_descendants_linux(child, &mut result);
                    }
                }
            }
        }
        result
    }
}

#[cfg(target_os = "linux")]
fn collect_descendants_linux(pid: u32, result: &mut Vec<u32>) {
    let children_path = format!("/proc/{pid}/task/{pid}/children");
    let Ok(content) = std::fs::read_to_string(&children_path) else {
        return;
    };
    for pid_str in content.split_whitespace() {
        if let Ok(child) = pid_str.parse::<u32>() {
            result.push(child);
            collect_descendants_linux(child, result);
        }
    }
}

#[cfg(target_os = "linux")]
fn get_ppid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rsplit_once(')')?.1;
    after_comm.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(target_os = "linux")]
pub fn find_tmux_pane<'pane>(
    pid: u32,
    pane_map: &'pane HashMap<u32, PaneInfo>,
    _tree: &ProcessTree,
) -> Option<&'pane PaneInfo> {
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
pub struct ProcessTree {
    parents: HashMap<u32, u32>,
    children: HashMap<u32, Vec<u32>>,
}

#[cfg(target_os = "macos")]
impl ProcessTree {
    pub fn build() -> Self {
        let Ok(output) = std::process::Command::new("ps")
            .args(["-eo", "pid,ppid"])
            .output()
        else {
            return Self::from_ps_output("");
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        Self::from_ps_output(&stdout)
    }

    fn from_ps_output(output: &str) -> Self {
        let (parents, children) = parse_ps_output(output);
        Self { parents, children }
    }

    pub fn is_alive(&self, pid: u32) -> bool {
        self.parents.contains_key(&pid)
    }

    pub fn has_children(&self, pid: u32) -> bool {
        self.children.get(&pid).is_some_and(|kids| !kids.is_empty())
    }

    pub fn descendants_of(&self, pid: u32) -> Vec<u32> {
        descendants_from_map(&self.children, pid)
    }
}

#[cfg(any(target_os = "macos", test))]
fn descendants_from_map(children: &HashMap<u32, Vec<u32>>, pid: u32) -> Vec<u32> {
    let mut result = Vec::new();
    let mut stack = vec![pid];
    while let Some(current) = stack.pop() {
        if let Some(kids) = children.get(&current) {
            for kid in kids {
                result.push(*kid);
                stack.push(*kid);
            }
        }
    }
    result
}

#[cfg(any(target_os = "macos", test))]
fn parse_ps_output(output: &str) -> (HashMap<u32, u32>, HashMap<u32, Vec<u32>>) {
    let mut parents = HashMap::new();
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for line in output.lines().skip(1) {
        let mut fields = line.split_whitespace();
        if let (Some(pid_str), Some(ppid_str)) = (fields.next(), fields.next())
            && let (Ok(pid), Ok(ppid)) = (pid_str.parse::<u32>(), ppid_str.parse::<u32>())
        {
            let _ = parents.insert(pid, ppid);
            children.entry(ppid).or_default().push(pid);
        }
    }
    (parents, children)
}

#[cfg(target_os = "macos")]
pub fn find_tmux_pane<'pane>(
    pid: u32,
    pane_map: &'pane HashMap<u32, PaneInfo>,
    tree: &ProcessTree,
) -> Option<&'pane PaneInfo> {
    let mut current = pid;
    for _ in 0..20 {
        let ppid = tree.parents.get(&current)?;
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
mod tests {
    use super::*;

    #[test]
    fn parse_ps_output_basic() {
        let output = "  PID  PPID\n  100     1\n  200   100\n  300   100\n";
        let (parents, children) = parse_ps_output(output);
        assert_eq!(parents.get(&100), Some(&1));
        assert_eq!(parents.get(&200), Some(&100));
        assert_eq!(parents.get(&300), Some(&100));
        assert_eq!(children.get(&100).map(Vec::len), Some(2));
        assert!(children.get(&1).is_some_and(|kids| kids.contains(&100)));
    }

    #[test]
    fn parse_ps_output_empty() {
        let (parents, children) = parse_ps_output("");
        assert!(parents.is_empty());
        assert!(children.is_empty());
    }

    #[test]
    fn parse_ps_output_header_only() {
        let (parents, _) = parse_ps_output("  PID  PPID\n");
        assert!(parents.is_empty());
    }

    #[test]
    fn parse_ps_output_malformed_lines() {
        let output = "  PID  PPID\n  notapid  1\n  100  also_bad\n  200  1\n";
        let (parents, _) = parse_ps_output(output);
        assert_eq!(parents.len(), 1);
        assert_eq!(parents.get(&200), Some(&1));
    }

    #[test]
    fn descendants_from_map_linear_chain() {
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        children.entry(1).or_default().push(10);
        children.entry(10).or_default().push(100);
        children.entry(100).or_default().push(1000);

        let result = descendants_from_map(&children, 1);
        assert_eq!(result.len(), 3);
        assert!(result.contains(&10));
        assert!(result.contains(&100));
        assert!(result.contains(&1000));
    }

    #[test]
    fn descendants_from_map_branching() {
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        children.entry(1).or_default().extend([10, 20]);
        children.entry(10).or_default().extend([11, 12]);
        children.entry(20).or_default().push(21);

        let result = descendants_from_map(&children, 1);
        assert_eq!(result.len(), 5);
        for pid in [10, 20, 11, 12, 21] {
            assert!(result.contains(&pid));
        }
    }

    #[test]
    fn descendants_from_map_no_children() {
        let children: HashMap<u32, Vec<u32>> = HashMap::new();
        let result = descendants_from_map(&children, 999);
        assert!(result.is_empty());
    }

    #[test]
    fn descendants_from_map_leaf_node() {
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        children.entry(1).or_default().push(10);
        let result = descendants_from_map(&children, 10);
        assert!(result.is_empty());
    }

    #[cfg(target_os = "linux")]
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

    #[cfg(target_os = "linux")]
    #[test]
    fn find_tmux_pane_not_found() {
        let pane_map = HashMap::new();
        let tree = ProcessTree::build();
        let result = find_tmux_pane(std::process::id(), &pane_map, &tree);
        assert!(result.is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn find_tmux_pane_stops_at_init() {
        let pane_map = HashMap::new();
        let tree = ProcessTree::build();
        let result = find_tmux_pane(1, &pane_map, &tree);
        assert!(result.is_none());
    }
}
