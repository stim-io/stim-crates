use std::{
    collections::{HashMap, VecDeque},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, SystemTime},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnRequest {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<std::path::PathBuf>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSnapshot {
    pub command: String,
    pub pid: u32,
    pub ppid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopProcessResult {
    pub already_stopped: bool,
    pub forced_pids: Vec<u32>,
    pub matched_pids: Vec<u32>,
    pub remaining_pids: Vec<u32>,
    pub stopped_pids: Vec<u32>,
}

pub fn spawn_background(request: SpawnRequest) -> std::io::Result<Child> {
    let mut command = Command::new(request.command);
    command
        .args(request.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if let Some(cwd) = request.cwd {
        command.current_dir(cwd);
    }

    for (name, value) in request.env {
        command.env(name, value);
    }

    command.spawn()
}

pub fn list_process_snapshots() -> std::io::Result<Vec<ProcessSnapshot>> {
    if cfg!(target_os = "windows") {
        return Ok(Vec::new());
    }

    let output = Command::new("ps")
        .args(["-axo", "pid=,ppid=,command="])
        .output()?;

    if !output.status.success() {
        return Err(std::io::Error::other("ps command failed"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    Ok(stdout.lines().filter_map(parse_ps_line).collect())
}

pub fn collect_process_tree_pids(processes: &[ProcessSnapshot], root_pids: &[u32]) -> Vec<u32> {
    let mut children_by_parent: HashMap<u32, Vec<u32>> = HashMap::new();

    for process in processes {
        children_by_parent
            .entry(process.ppid)
            .or_default()
            .push(process.pid);
    }

    let mut queue = root_pids.iter().copied().collect::<VecDeque<_>>();
    let mut visited = Vec::<u32>::new();

    while let Some(pid) = queue.pop_front() {
        if visited.contains(&pid) {
            continue;
        }

        visited.push(pid);

        for child_pid in children_by_parent.get(&pid).into_iter().flatten() {
            if !visited.contains(child_pid) {
                queue.push_back(*child_pid);
            }
        }
    }

    visited.sort_by(|left, right| right.cmp(left));
    visited
}

pub fn stop_processes(pids: &[u32]) -> std::io::Result<StopProcessResult> {
    let mut unique_pids = pids.to_vec();

    unique_pids.sort_by(|left, right| right.cmp(left));
    unique_pids.dedup();

    if unique_pids.is_empty() {
        return Ok(StopProcessResult {
            already_stopped: true,
            forced_pids: Vec::new(),
            matched_pids: Vec::new(),
            remaining_pids: Vec::new(),
            stopped_pids: Vec::new(),
        });
    }

    signal_pids(&unique_pids, "TERM")?;
    let remaining_after_term = wait_for_exit(&unique_pids, Duration::from_secs(5))?;

    if remaining_after_term.is_empty() {
        return Ok(StopProcessResult {
            already_stopped: false,
            forced_pids: Vec::new(),
            matched_pids: unique_pids.clone(),
            remaining_pids: Vec::new(),
            stopped_pids: unique_pids,
        });
    }

    signal_pids(&remaining_after_term, "KILL")?;
    let remaining_after_kill = wait_for_exit(&remaining_after_term, Duration::from_secs(5))?;
    let stopped_pids = unique_pids
        .iter()
        .copied()
        .filter(|pid| !remaining_after_kill.contains(pid))
        .collect::<Vec<_>>();

    Ok(StopProcessResult {
        already_stopped: false,
        forced_pids: remaining_after_term,
        matched_pids: unique_pids,
        remaining_pids: remaining_after_kill,
        stopped_pids,
    })
}

fn signal_pids(pids: &[u32], signal: &str) -> std::io::Result<()> {
    if cfg!(target_os = "windows") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "process signaling is not implemented on windows yet",
        ));
    }

    for pid in pids {
        if !process_is_alive(*pid)? {
            continue;
        }

        let status = Command::new("kill")
            .args([format!("-{signal}"), pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;

        if !status.success() && process_is_alive(*pid)? {
            return Err(std::io::Error::other(format!(
                "failed to send SIG{signal} to pid {pid}"
            )));
        }
    }

    Ok(())
}

fn wait_for_exit(pids: &[u32], timeout: Duration) -> std::io::Result<Vec<u32>> {
    let started = SystemTime::now();

    loop {
        let remaining = pids
            .iter()
            .copied()
            .filter_map(|pid| match process_is_alive(pid) {
                Ok(true) => Some(Ok(pid)),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<std::io::Result<Vec<_>>>()?;

        if remaining.is_empty() || started.elapsed().unwrap_or_default() > timeout {
            return Ok(remaining);
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn process_is_alive(pid: u32) -> std::io::Result<bool> {
    if cfg!(target_os = "windows") {
        return Ok(false);
    }

    let status = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;

    if !status.success() {
        return Ok(false);
    }

    Ok(!process_is_zombie(pid)?)
}

fn process_is_zombie(pid: u32) -> std::io::Result<bool> {
    let output = Command::new("ps")
        .args(["-o", "state=", "-p", &pid.to_string()])
        .output()?;

    if !output.status.success() {
        return Ok(false);
    }

    let state = String::from_utf8_lossy(&output.stdout);

    Ok(state.trim().starts_with('Z'))
}

pub fn parse_ps_line(line: &str) -> Option<ProcessSnapshot> {
    let mut parts = line.split_whitespace();
    let pid = parts.next()?.parse::<u32>().ok()?;
    let ppid = parts.next()?.parse::<u32>().ok()?;
    let command = parts.collect::<Vec<_>>().join(" ");

    if command.is_empty() {
        return None;
    }

    Some(ProcessSnapshot { command, pid, ppid })
}
