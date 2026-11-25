//! `corral report-activity`, which the overlay starts in every sandbox: what the sandbox uses,
//! from its own cgroup and network namespace, reported to Corral at the interval Corral asks
//! for. Corral decides from it whether the sandbox is at work.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::client::{Client, Corral};
use crate::sandbox_file::SandboxFile;

/// cgroup v2, then v1.
const CPU_FILES: [(&str, CpuUnit); 2] = [
    ("/sys/fs/cgroup/cpu.stat", CpuUnit::StatMicros),
    ("/sys/fs/cgroup/cpuacct/cpuacct.usage", CpuUnit::Nanos),
];
const NETWORK_FILE: &str = "/proc/net/dev";
/// After Corral did not take a report, as at a boot before it records the sandbox's token, or
/// while it restarts.
const RETRY_EVERY: Duration = Duration::from_secs(5);
/// When Corral's answer does not say.
const REPORT_EVERY: Duration = Duration::from_mins(1);
const UNREADABLE_AGAIN: Duration = Duration::from_mins(10);

#[derive(Clone, Copy)]
enum CpuUnit {
    /// `usage_usec` in a `cpu.stat`.
    StatMicros,
    Nanos,
}

/// The sandbox's counters at one moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub cpu_us: u64,
    pub network_bytes: u64,
}

impl Sample {
    /// The counters now, or why they cannot be read.
    pub fn now() -> Result<Self, String> {
        let cpu_us = CPU_FILES
            .iter()
            .find_map(|(path, unit)| {
                let text = std::fs::read_to_string(path).ok()?;
                cpu_micros(&text, *unit)
            })
            .ok_or("cannot read the sandbox's processor use from its cgroup")?;
        let network = std::fs::read_to_string(NETWORK_FILE)
            .map_err(|error| format!("cannot read {NETWORK_FILE}: {error}"))?;
        let network_bytes = network_bytes(&network)
            .ok_or_else(|| format!("cannot read the traffic counters in {NETWORK_FILE}"))?;
        Ok(Self {
            cpu_us,
            network_bytes,
        })
    }
}

fn cpu_micros(text: &str, unit: CpuUnit) -> Option<u64> {
    match unit {
        CpuUnit::StatMicros => text
            .lines()
            .find_map(|line| line.strip_prefix("usage_usec "))?
            .trim()
            .parse()
            .ok(),
        CpuUnit::Nanos => text.trim().parse::<u64>().ok().map(|nanos| nanos / 1000),
    }
}

/// Bytes received and sent over every interface but loopback.
fn network_bytes(text: &str) -> Option<u64> {
    let mut total: u64 = 0;
    for line in text.lines().skip(2) {
        let (interface, counters) = line.split_once(':')?;
        if interface.trim() == "lo" {
            continue;
        }
        let counters: Vec<&str> = counters.split_whitespace().collect();
        let received: u64 = counters.first()?.parse().ok()?;
        let sent: u64 = counters.get(8)?.parse().ok()?;
        total = total.wrapping_add(received).wrapping_add(sent);
    }
    Some(total)
}

/// What the sandbox used between two samples, as Corral takes it.
pub fn report(from: Sample, to: Sample, interval: Duration) -> serde_json::Value {
    json!({
        "intervalMs": u64::try_from(interval.as_millis()).unwrap_or(u64::MAX),
        "cpuUs": to.cpu_us.saturating_sub(from.cpu_us),
        "networkBytes": to.network_bytes.wrapping_sub(from.network_bytes),
    })
}

/// Reports to Corral forever. The first report of a boot covers no time: it says the sandbox
/// reports at all. A sandbox whose counters cannot be read reports nothing, so Corral keeps it
/// awake rather than put it to sleep at work.
pub fn run(home: &Path) -> ! {
    let mut last = loop {
        match Sample::now() {
            Ok(sample) => break (sample, Instant::now()),
            Err(error) => {
                eprintln!("corral report-activity: {error}; trying again in a while");
                std::thread::sleep(UNREADABLE_AGAIN);
            }
        }
    };
    let mut corral = None;
    let mut said = None;
    let mut sent = send(home, &mut corral, &report(last.0, last.0, Duration::ZERO));
    loop {
        let wait = match sent {
            Ok(wait) => {
                said = None;
                wait
            }
            Err(error) => {
                if said.as_ref() != Some(&error) {
                    eprintln!("corral report-activity: {error}");
                    said = Some(error);
                }
                RETRY_EVERY
            }
        };
        std::thread::sleep(wait);
        let Ok(sample) = Sample::now() else {
            sent = Err("cannot read the sandbox's counters".to_owned());
            continue;
        };
        let now = Instant::now();
        sent = send(home, &mut corral, &report(last.0, sample, now - last.1));
        last = (sample, now);
    }
}

/// Sends one report, and answers how long until the next, or why it failed. The client, kept
/// between reports so its connection is too, is made again from the sandbox file after a failure.
fn send(
    home: &Path,
    corral: &mut Option<Client>,
    report: &serde_json::Value,
) -> Result<Duration, String> {
    let client = match corral {
        Some(client) => client,
        None => corral.insert(Client::new(&SandboxFile::read(home)?)),
    };
    let reply = client.post("/api/sandbox/activity", report);
    let answered = match reply {
        Ok(reply) if reply.status == 200 => Ok(reply.body["afterSecs"]
            .as_u64()
            .filter(|secs| *secs > 0)
            .map_or(REPORT_EVERY, Duration::from_secs)),
        Ok(reply) => Err(reply.error()),
        Err(error) => Err(error),
    };
    if answered.is_err() {
        *corral = None;
    }
    answered
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET_DEV: &str = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 14102117923 4996554    0    0    0     0          0         0 14102117923 4996554    0    0    0     0       0          0
  eth0: 5269 38914338    0 1076    0     0          0         0 2285 5464906    0   65    0     0       0          0
  eth1:100 1 0 0 0 0 0 0 20 1 0 0 0 0 0 0
";

    #[test]
    fn network_counts_every_interface_but_loopback() {
        assert_eq!(network_bytes(NET_DEV), Some(5269 + 2285 + 100 + 20));
        assert_eq!(network_bytes("Inter-|\n face |\n"), Some(0));
        assert_eq!(network_bytes("Inter-|\n face |\n eth0: x\n"), None);
    }

    #[test]
    fn cpu_reads_cgroup_v2_and_v1() {
        let stat = "usage_usec 120707796\nuser_usec 90578332\nsystem_usec 30129464\n";
        assert_eq!(cpu_micros(stat, CpuUnit::StatMicros), Some(120_707_796));
        assert_eq!(cpu_micros("2500000\n", CpuUnit::Nanos), Some(2500));
        assert_eq!(cpu_micros("user_usec 1\n", CpuUnit::StatMicros), None);
    }

    #[test]
    fn a_report_is_the_difference_between_two_samples() {
        let from = Sample {
            cpu_us: 1_000,
            network_bytes: 500,
        };
        let to = Sample {
            cpu_us: 4_000,
            network_bytes: 1_700,
        };
        assert_eq!(
            report(from, to, Duration::from_secs(60)),
            json!({"intervalMs": 60_000, "cpuUs": 3_000, "networkBytes": 1_200})
        );
        assert_eq!(
            report(to, to, Duration::ZERO),
            json!({"intervalMs": 0, "cpuUs": 0, "networkBytes": 0})
        );
    }

    #[test]
    fn this_machine_can_be_sampled() {
        if Path::new(CPU_FILES[0].0).exists() {
            Sample::now().unwrap();
        }
    }
}
