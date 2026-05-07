use std::process::Command;

use stim_platform::process::{
    collect_process_tree_pids, parse_ps_line, stop_processes, ProcessSnapshot,
};

#[test]
fn parses_spaced_commands() {
    assert_eq!(
        parse_ps_line("123 1 stim-controller --stim-stamp-app=controller"),
        Some(ProcessSnapshot {
            pid: 123,
            ppid: 1,
            command: "stim-controller --stim-stamp-app=controller".into(),
        })
    );
}

#[test]
fn collects_children_first() {
    let processes = vec![
        ProcessSnapshot {
            command: "root".into(),
            pid: 10,
            ppid: 1,
        },
        ProcessSnapshot {
            command: "child".into(),
            pid: 11,
            ppid: 10,
        },
        ProcessSnapshot {
            command: "grandchild".into(),
            pid: 12,
            ppid: 11,
        },
    ];

    assert_eq!(
        collect_process_tree_pids(&processes, &[10]),
        vec![12, 11, 10]
    );
}

#[cfg(unix)]
#[test]
fn stops_zombie_processes() {
    let mut child = Command::new("sleep").arg("30").spawn().unwrap();
    let pid = child.id();
    let result = stop_processes(&[pid]).unwrap();

    let _ = child.wait();

    assert!(!result.already_stopped);
    assert_eq!(result.matched_pids, vec![pid]);
    assert_eq!(result.stopped_pids, vec![pid]);
    assert!(result.forced_pids.is_empty());
    assert!(result.remaining_pids.is_empty());
}
