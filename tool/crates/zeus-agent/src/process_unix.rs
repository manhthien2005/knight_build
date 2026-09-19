//! Process supervision on Linux: one JVM per account, started, watched, killed as a tree.
//!
//! Target path: `Tool/tool/crates/zeus-agent/src/process_unix.rs`
//!
//! This replaces `zeus-core`'s `process_adapter` / `session_supervisor` / `process_launch_spec`,
//! all three of which are `#[cfg(windows)]` and none of which is ported. Porting them would mean
//! carrying Windows job objects and command-line quoting rules into a container that has neither.
//! `process_launch_spec.rs` alone has 50 `cfg` sites, and every one of them exists to model
//! `MAX_WINDOWS_COMMAND_LINE_UNITS` and sealed paths — on Linux argv is a vector and there is
//! nothing to quote.
//!
//! Two responsibilities are easy to get wrong and both are called out below:
//!
//! 1. **The agent is PID 1**, so it must reap *every* zombie, not just its own children.
//!    A container whose init does not reap accumulates defunct processes until the PID space
//!    or the process table is exhausted. This is the classic PID-1 bug.
//! 2. **Exactly one supervisor may touch a PID.** `docker-build/bin/entrypoint.sh` currently has
//!    its own `while :; do sleep 10` loop that restarts any dead tab. If both run, a Stop from
//!    the web is undone 10 s later, and two restarts racing produce two JVMs sharing one
//!    `-Duser.home` — which corrupts the record stores and logs one account in twice.
//!    A5.1 deletes that loop; this file is the only supervisor.

use std::os::unix::process::CommandExt;
use std::process::Command;
use std::time::{Duration, Instant};

/// PID of a supervised JVM, plus what is needed to stop it as a tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Child {
    pub pid: i32,
    /// The child called `setsid()`, so its pid *is* its process-group id. Killing `-pgid` takes
    /// the JVM and anything it spawned, which a bare `kill(pid)` does not.
    pub pgid: i32,
    pub started_at: Instant,
}

impl Child {
    /// Non-blocking liveness probe.
    ///
    /// `kill(pid, 0)` is not enough: it also succeeds for a **zombie**, and a stopped child is
    /// always a zombie first — the agent is its parent and has not called `waitpid` yet. Reading
    /// only `kill` would make `stop` spin its entire grace period and then escalate to SIGKILL on
    /// a child that died on the first SIGTERM, which is both a wrong `StopOutcome` and a shutdown
    /// that costs the full budget instead of milliseconds. So consult procfs for the real state.
    ///
    /// Still paired with `waitpid` reaping — see `reap` — because any pid can be recycled to an
    /// unrelated process once ours is gone.
    pub fn alive(&self) -> bool {
        // SAFETY: signal 0 performs no delivery; it only sets errno.
        if unsafe { libc::kill(self.pid, 0) } != 0 {
            return false;
        }
        // procfs unavailable but kill succeeded: assume alive and let `stop`'s deadline decide,
        // rather than declaring a live JVM dead and restarting it into a second account session.
        self.procfs_state() != Some(b'Z')
    }

    /// State character from field 3 of `/proc/<pid>/stat`, or `None` when procfs has nothing to say.
    ///
    /// The comm field can contain spaces and parentheses, so split from the LAST `)` rather than
    /// by whitespace — the same reason `cpu_ticks` parses that file this way.
    fn procfs_state(&self) -> Option<u8> {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", self.pid)).ok()?;
        let after_comm = stat.rsplit_once(')')?.1.trim();
        after_comm
            .split_whitespace()
            .next()?
            .as_bytes()
            .first()
            .copied()
    }

    /// Resident set size in KiB, read from procfs. `VmRSS` is the number the container is
    /// actually charged for, unlike the JVM's own notion of heap "used".
    pub fn rss_kib(&self) -> Option<u64> {
        let status = std::fs::read_to_string(format!("/proc/{}/status", self.pid)).ok()?;
        status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            // No `trim()` before this: `split_whitespace` already skips leading blanks, and the
            // extra trim is a clippy `trim_split_whitespace` lint.
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|value| value.parse().ok())
    }

    /// CPU time in clock ticks (user + system), for a rate over a window.
    ///
    /// Deliberately *not* `ps -o pcpu`: that averages over the whole process lifetime, which
    /// buries the effect of a paint-mode change behind JVM startup cost. `mod/potato/bench.sh`
    /// uses this same field for the same reason.
    pub fn cpu_ticks(&self) -> Option<u64> {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", self.pid)).ok()?;
        // Fields 14 (utime) and 15 (stime), 1-indexed. The comm field can contain spaces and
        // parentheses, so split from the LAST ')' rather than by whitespace.
        let after_comm = stat.rsplit_once(')').map(|(_, rest)| rest)?.trim();
        let fields: Vec<&str> = after_comm.split_whitespace().collect();
        // After comm, field 3 is state, so utime is index 11 and stime is index 12.
        let utime: u64 = fields.get(11)?.parse().ok()?;
        let stime: u64 = fields.get(12)?.parse().ok()?;
        Some(utime + stime)
    }
}

/// Prepares a `Command` so its child becomes a session leader.
///
/// Must be called instead of `Command::spawn` directly. Without `setsid` the child shares the
/// agent's process group, so a `kill(-pgid)` meant for one account would take the agent with it.
pub fn prepare(command: &mut Command) {
    // SAFETY: the closure runs in the child after fork and before exec, so it may only call
    // async-signal-safe functions. `setsid` is one. No allocation, no locking, no I/O.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

/// Spawns and returns the handle.
///
/// The `std::process::Child` is deliberately leaked: the agent reaps through `waitpid` in the
/// main loop (see `reap`), and holding both a std handle and our own `Child` would mean two owners
/// of one pid — the std handle's `Drop` would `wait()` on a child the main loop already reaped.
///
/// An agent crash does not orphan the JVMs, and not because of a drop hook: the child called
/// `setsid()` in `prepare`, and when the agent dies the container dies with it. (A drop hook could
/// not do this anyway — `panic = "abort"` runs no destructors, and `mem::forget` above skips
/// `Drop` by design.) What actually prevents orphans across a *restart* is that nothing reuses a
/// `-Duser.home` while its JVM lives; `stop` kills the whole process group first.
pub fn spawn(mut command: Command) -> std::io::Result<Child> {
    prepare(&mut command);
    let child = command.spawn()?;
    let pid = child.id() as i32;
    std::mem::forget(child);
    Ok(Child {
        pid,
        pgid: pid,
        started_at: Instant::now(),
    })
}

/// Reaps every finished child, non-blocking.
///
/// Call once per main-loop tick. Returns the pids that exited, with their status, so the caller
/// can tell a clean shutdown from a crash and drive the backoff accordingly.
pub fn reap() -> Vec<(i32, ExitStatus)> {
    let mut exited = Vec::new();
    loop {
        let mut status = 0i32;
        // SAFETY: `status` is a valid out-pointer and `WNOHANG` guarantees no blocking, so this
        // cannot stall the loop that also services realtime events.
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if pid <= 0 {
            break; // 0 = nothing finished yet; -1 = no children (ECHILD)
        }
        exited.push((
            pid,
            // WIFEXITED/WEXITSTATUS/WIFSIGNALED/WTERMSIG are safe fns in libc (they decode a
            // status word, they do not syscall), so no `unsafe` block belongs around them.
            if libc::WIFEXITED(status) {
                ExitStatus::Exited(libc::WEXITSTATUS(status))
            } else if libc::WIFSIGNALED(status) {
                ExitStatus::Signaled(libc::WTERMSIG(status))
            } else {
                ExitStatus::Other
            },
        ));
    }
    exited
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    Exited(i32),
    Signaled(i32),
    Other,
}

impl ExitStatus {
    /// A JVM killed by SIGTERM during an orderly stop is not a crash, and must not advance the
    /// backoff — otherwise a planned restart starts the next account 60 s late.
    pub fn is_crash(&self) -> bool {
        !matches!(self, Self::Exited(0) | Self::Signaled(libc::SIGTERM))
    }
}

/// Stops one account's whole tree, escalating.
///
/// The budget matters: Railway measures shutdown, and the image currently stops in ~7 s
/// (measured claim in `docker-build/README.md`, **not yet reproduced** — V2.5). With N accounts
/// the stops run concurrently, not serially, or N×7 s exceeds any grace period.
pub fn stop(child: &Child, grace: Duration) -> StopOutcome {
    if child.pgid <= 0 {
        return StopOutcome::Failed;
    }

    // SAFETY: negative pid signals the whole process group.
    let sent = unsafe { libc::kill(-child.pgid, libc::SIGTERM) };
    if sent != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) {
            // Already gone (ESRCH). Reap will collect it.
            return StopOutcome::AlreadyGone;
        } else {
            eprintln!("[process_unix] kill(-{}, SIGTERM) failed: {err}", child.pgid);
            return StopOutcome::Failed;
        }
    }

    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if !child.alive() {
            return StopOutcome::Terminated;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    if !child.alive() {
        return StopOutcome::Terminated;
    }

    // Still alive past the grace period: the JVM is wedged, not slow. Escalate.
    // SAFETY: negative pid signals the whole process group.
    let kill_sent = unsafe { libc::kill(-child.pgid, libc::SIGKILL) };
    if kill_sent != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) {
            if !child.alive() {
                return StopOutcome::Killed;
            }
        }
        eprintln!("[process_unix] kill(-{}, SIGKILL) failed: {err}", child.pgid);
        return StopOutcome::Failed;
    }

    // SIGKILL sent: verify process death within bounded deadline
    let kill_deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < kill_deadline {
        if !child.alive() {
            return StopOutcome::Killed;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    if !child.alive() {
        StopOutcome::Killed
    } else {
        eprintln!(
            "[process_unix] process pgid={} still alive after SIGKILL escalation",
            child.pgid
        );
        StopOutcome::Failed
    }
}

pub use crate::supabase_rest::StopOutcome;

/// Crash backoff. Replaces the flat `restartDelaySeconds` in the current web schema.
///
/// A flat delay either restarts too fast (a jar that dies on startup becomes a hot loop that
/// burns the whole vCPU budget) or too slow (one transient network drop costs a minute of
/// farming). The ladder gets both: fast recovery from a blip, bounded cost from a hard failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    pub attempt: u32,
    pub next_allowed: Instant,
}

impl Backoff {
    pub const LADDER: [Duration; 3] = [
        Duration::from_secs(5),
        Duration::from_secs(15),
        Duration::from_secs(60),
    ];
    /// A process that stayed up this long was not failing at startup, so the ladder resets.
    pub const HEALTHY_UPTIME: Duration = Duration::from_secs(600);

    pub fn new() -> Self {
        Self {
            attempt: 0,
            next_allowed: Instant::now(),
        }
    }

    /// Records a death and schedules the next attempt.
    pub fn on_exit(&mut self, uptime: Duration, status: ExitStatus) {
        if !status.is_crash() {
            // An intentional stop is not a failure. Clear the ladder so a later Start is immediate.
            self.attempt = 0;
            self.next_allowed = Instant::now();
            return;
        }
        if uptime >= Self::HEALTHY_UPTIME {
            self.attempt = 0;
        }
        // `attempt` is u32, the ladder index is usize; cast the bound rather than the field so
        // the clamp reads as "min(attempt, last rung)" without widening stored state.
        let last_rung = (Self::LADDER.len() - 1) as u32;
        let delay = Self::LADDER[self.attempt.min(last_rung) as usize];
        self.attempt = self.attempt.saturating_add(1);
        self.next_allowed = Instant::now() + delay;
    }

    pub fn may_start_now(&self) -> bool {
        Instant::now() >= self.next_allowed
    }
}

/// Forces a full GC so the JVM uncommits pages and RSS actually falls.
///
/// `jattach` is a 63 KB static binary already in the image, and it is the only way to trigger a
/// full GC from outside a jlink runtime, which ships no `jcmd`. Combined with the
/// `Min/MaxHeapFreeRatio` pair in `launch.rs` this is what makes the trim reclaim memory rather
/// than merely move it between heap regions.
///
/// Only call this above a threshold. A quiet tab has produced no garbage, and pausing it to
/// collect nothing is a visible stutter over noVNC for zero benefit. The image's measured
/// behaviour: at the menu the trim logged `119348kB -> 119500kB`, i.e. no change, correctly.
pub fn trim_if_above(
    child: &Child,
    jattach: &std::path::Path,
    threshold_kib: u64,
) -> Option<TrimReport> {
    let before = child.rss_kib()?;
    if before <= threshold_kib {
        return None;
    }
    // Bind the pid string first so all three arguments are `&str`; mixing an owned String with
    // two literals in one array does not unify to a single element type.
    let pid = child.pid.to_string();
    let status = Command::new(jattach)
        .args([pid.as_str(), "jcmd", "GC.run"])
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    // The uncommit happens during the collection; give it a moment before reading back.
    std::thread::sleep(Duration::from_secs(1));
    Some(TrimReport {
        before_kib: before,
        after_kib: child.rss_kib().unwrap_or(before),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrimReport {
    pub before_kib: u64,
    pub after_kib: u64,
}

/// Container-wide metrics from cgroup v2. Per-process numbers do not sum to what Railway charges
/// for, because Xvnc, openbox and websockify are not JVMs.
pub struct CgroupMetrics {
    pub memory_current_bytes: u64,
    pub memory_max_bytes: Option<u64>,
    /// Cumulative CPU microseconds; diff over a window for a percentage.
    pub cpu_usage_usec: u64,
}

pub fn read_cgroup_metrics() -> Option<CgroupMetrics> {
    let read = |path: &str| std::fs::read_to_string(path).ok();

    let memory_current_bytes = read("/sys/fs/cgroup/memory.current")?.trim().parse().ok()?;
    let memory_max_bytes =
        read("/sys/fs/cgroup/memory.max").and_then(|text| text.trim().parse::<u64>().ok()); // "max" parses to None = unlimited
    let cpu_usage_usec = read("/sys/fs/cgroup/cpu.stat")
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("usage_usec"))
                .and_then(|rest| rest.trim().parse().ok())
        })
        .unwrap_or(0);

    Some(CgroupMetrics {
        memory_current_bytes,
        memory_max_bytes,
        cpu_usage_usec,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_exit_is_not_a_crash() {
        assert!(!ExitStatus::Exited(0).is_crash());
        assert!(!ExitStatus::Signaled(libc::SIGTERM).is_crash());
        assert!(ExitStatus::Exited(1).is_crash());
        assert!(ExitStatus::Signaled(libc::SIGSEGV).is_crash());
        assert!(ExitStatus::Other.is_crash());
    }

    /// An intentional stop must not push the next start out by the ladder, or Stop-then-Start
    /// from the web feels broken for up to a minute.
    #[test]
    fn an_intentional_stop_resets_the_ladder() {
        let mut backoff = Backoff::new();
        backoff.on_exit(Duration::from_secs(300), ExitStatus::Exited(1));
        backoff.on_exit(Duration::from_secs(300), ExitStatus::Exited(1));
        assert_eq!(backoff.attempt, 2);

        backoff.on_exit(Duration::from_secs(1), ExitStatus::Signaled(libc::SIGTERM));
        assert_eq!(backoff.attempt, 0);
        assert!(backoff.may_start_now());
    }

    #[test]
    fn the_ladder_climbs_then_caps() {
        let mut backoff = Backoff::new();
        let start = Instant::now();

        backoff.on_exit(Duration::from_secs(1), ExitStatus::Exited(1));
        assert!(!backoff.may_start_now());
        assert!(backoff.next_allowed - start >= Backoff::LADDER[0]);

        backoff.on_exit(Duration::from_secs(1), ExitStatus::Exited(1));
        assert!(backoff.next_allowed - start >= Backoff::LADDER[1]);

        backoff.on_exit(Duration::from_secs(1), ExitStatus::Exited(1));
        backoff.on_exit(Duration::from_secs(1), ExitStatus::Exited(1));
        // Capped at the last rung, not growing without bound.
        assert!(backoff.next_allowed - start <= Backoff::LADDER[2] * 2);
    }

    /// A process that ran for ten minutes was not failing at startup, so the next failure
    /// should be treated as fresh rather than as rung four of an old ladder.
    #[test]
    fn long_uptime_resets_the_ladder() {
        let mut backoff = Backoff::new();
        backoff.on_exit(Duration::from_secs(1), ExitStatus::Exited(1));
        backoff.on_exit(Duration::from_secs(1), ExitStatus::Exited(1));
        assert_eq!(backoff.attempt, 2);

        backoff.on_exit(Backoff::HEALTHY_UPTIME, ExitStatus::Exited(1));
        assert_eq!(backoff.attempt, 1, "reset, then this failure is rung one");
    }

    /// procfs parsing must not panic on a process that vanished mid-read, and must not be
    /// fooled by a comm field containing spaces — `java` is fine, but a renamed binary is not.
    #[test]
    fn cpu_ticks_survives_a_missing_process() {
        let gone = Child {
            pid: i32::MAX,
            pgid: i32::MAX,
            started_at: Instant::now(),
        };
        assert_eq!(gone.cpu_ticks(), None);
        assert_eq!(gone.rss_kib(), None);
        assert!(!gone.alive());
    }

    /// `prepare` calls `setsid()`, which is what makes `stop`'s `kill(-pgid)` take down the whole
    /// tree instead of hitting the agent's own process group. Asserting `child.pgid == child.pid`
    /// would NOT pin this — `spawn` sets `pgid` to `pid` unconditionally — so the assertion reads
    /// the kernel's own view via `getpgid`, which equals the pid only for a session leader.
    ///
    /// This must stay the only process-spawning test in the crate: `reap()` calls `waitpid(-1)`,
    /// which collects *any* child of the test process, and the harness runs tests in parallel
    /// threads. A second spawning test would race for the same zombie.
    ///
    /// Runs lifecycle cases sequentially:
    /// - Case A: graceful stop & setsid verification (SIGTERM -> Terminated)
    /// - Case B: already gone detection on reaped child & nonexistent PID
    /// - Case C: escalation on SIGTERM-resistant child (SIGTERM ignored -> SIGKILL -> Killed)
    #[test]
    fn spawn_and_lifecycle_termination_cases() {
        // Case A — graceful stop & setsid verification
        // `sleep` rather than a shell loop: one process, no shell in between, so the pgid read
        // below is about the child `spawn` created and not about an intermediate's children.
        let mut command = Command::new("sleep");
        command.arg("30");
        let child = spawn(command).expect("spawn sleep");

        // SAFETY: getpgid is a read-only query on a pid we own; the child lives until stop().
        let pgid = unsafe { libc::getpgid(child.pid) };
        assert_eq!(
            pgid, child.pid,
            "child must be its own process-group leader (setsid did not run)"
        );
        // SAFETY: getpgid(0) reports the calling process's own group.
        let agent_pgid = unsafe { libc::getpgid(0) };
        assert_ne!(
            pgid, agent_pgid,
            "child must not share the agent's process group"
        );

        // Leave nothing behind: stop the tree, then reap it. `sleep` dies on SIGTERM, so the only
        // correct outcome is Terminated.
        let outcome = stop(&child, Duration::from_secs(5));
        assert_eq!(
            outcome,
            StopOutcome::Terminated,
            "SIGTERM must suffice: {outcome:?}"
        );
        let reaped = reap();
        assert!(
            reaped.iter().any(|(pid, _)| *pid == child.pid),
            "waitpid must report the child just stopped"
        );
        assert!(!child.alive(), "child must be gone after stop + reap");

        // Case B — already gone
        // B1: Process handle that was already stopped and reaped
        let outcome_reaped = stop(&child, Duration::from_secs(1));
        assert_eq!(
            outcome_reaped,
            StopOutcome::AlreadyGone,
            "reaped child must be classified as AlreadyGone: {outcome_reaped:?}"
        );
        // B2: Genuinely nonexistent process group (exercises ESRCH errno handling)
        let non_existent = Child {
            pid: i32::MAX,
            pgid: i32::MAX,
            started_at: Instant::now(),
        };
        let outcome_nonexistent = stop(&non_existent, Duration::from_secs(1));
        assert_eq!(
            outcome_nonexistent,
            StopOutcome::AlreadyGone,
            "nonexistent child must be classified as AlreadyGone: {outcome_nonexistent:?}"
        );

        // Case C — escalation (SIGTERM-resistant child)
        // Spawn a child that traps and ignores SIGTERM.
        let mut resistant = Command::new("sh");
        resistant.args(["-c", "trap '' TERM; sleep 30"]);
        let child_c = spawn(resistant).expect("spawn resistant child");
        assert!(child_c.alive(), "resistant child must be alive initially");

        // With 200ms grace, SIGTERM is ignored and stop must escalate to SIGKILL and verify death
        let outcome_c = stop(&child_c, Duration::from_millis(200));
        assert_eq!(
            outcome_c,
            StopOutcome::Killed,
            "SIGKILL escalation must succeed and confirm death: {outcome_c:?}"
        );
        let reaped_c = reap();
        assert!(
            reaped_c.iter().any(|(pid, _)| *pid == child_c.pid),
            "waitpid must report resistant child was killed"
        );
        assert!(!child_c.alive(), "resistant child must not be alive after kill");
    }
}

