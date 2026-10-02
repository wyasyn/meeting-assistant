//! Process watch through `sysinfo`, on every OS (FR-1.1).

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use super::{ProcessInfo, ProcessSource};

pub struct SysinfoProcesses {
    system: System,
}

impl SysinfoProcesses {
    pub fn new() -> Self {
        Self {
            system: System::new(),
        }
    }
}

impl ProcessSource for SysinfoProcesses {
    fn list(&mut self) -> Vec<ProcessInfo> {
        // Names and pids only; no threads, CPU, memory or command lines.
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().without_tasks(),
        );
        self.system
            .processes()
            .iter()
            .map(|(pid, process)| ProcessInfo {
                pid: pid.as_u32(),
                name: process.name().to_string_lossy().into_owned(),
            })
            .collect()
    }
}
