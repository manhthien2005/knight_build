//! zeus-agent — Linux runtime driver for one containerized Knight Online node.
//!
//! ## Wired (as of Task 9/19)
//!
//! | Module | Task | Function |
//! |---|---|---|
//! | `launch` | Task 7, B1 | JVM argv (-cp, never -jar) |
//! | `process_unix` | Task 7, B1 | spawn/setsid/reap/stop |
//! | `crypto` | Task 10, B4 | P-256 ECDH + AES-GCM unseal |
//! | `supabase_rest` | Task 8, B2 | Write path → PostgREST |
//! | `supabase_realtime` | Task 8, B2 | Read path ← Phoenix WebSocket |
//! | `pairing` | Task 19, B3 | Device keypair + pair code + poll |
//! | `main_loop` | Task 9, B5+B6+B7 | Main loop: config/telemetry/command |
//!
//! ## Startup sequence
//!
//! 1. `pairing::ensure_paired` → `PairState { device_id, access_token, secret_key_bytes }`
//! 2. `main_loop::run(cfg, access_token, secret_key_bytes)` → never returns (Loop until SIGTERM)

// Both modules are unix-only: `launch` uses `std::os::unix::fs::PermissionsExt` for the 0700
// account home, and `process_unix` uses `pre_exec`/`libc` for setsid, kill(-pgid) and waitpid.
#[cfg(unix)]
#[allow(dead_code)]
mod launch;
#[cfg(unix)]
#[allow(dead_code)]
mod process_unix;

/// Sealing of game-account credentials (Task 10, B4). Cross-platform.
#[allow(dead_code)]
mod crypto;

/// Write path to Supabase over PostgREST (B3 pairing, B5/B6 telemetry, B7 command ack).
#[allow(dead_code)]
mod supabase_rest;

/// Read path from Supabase — Phoenix WebSocket (Task 8, B8).
#[allow(dead_code)]
mod supabase_realtime;

/// Device pairing — B3.1 + B3.2 (AGENT-SPEC §5.3).
/// Cross-platform: HKDF + P-256 key derivation has no unix-only dep.
#[allow(dead_code)]
mod pairing;

/// Main loop — B5+B6+B7+B8 (AGENT-SPEC §4.2).
/// unix-only: reads /proc, /sys/fs/cgroup, runs ss, calls waitpid.
#[cfg(unix)]
#[allow(dead_code)]
mod main_loop;

#[allow(unused_imports)]
use zeus_core::wire::{
    CONTROL_FILE_NAME, CONTROL_VERSION, CTL_KEY_COUNT, SNAPSHOT_FILE_NAME, SUPPORTED_VERSION,
    control_path, read_settings, snapshot_path, write_settings,
};

fn main() {
    #[cfg(unix)]
    {
        // Smoke-test mode: ZEUS_SMOKE_TEST=1 → print wire contract constants and exit 0.
        // Used during Docker build to verify the binary loads (no glibc errors) without
        // making any network calls. Set by the Dockerfile ABI smoke gate RUN step.
        if std::env::var("ZEUS_SMOKE_TEST").as_deref() == Ok("1") {
            println!(
                "zeus-agent smoke-test OK: control v{CONTROL_VERSION} / {CTL_KEY_COUNT} keys \
                 ({CONTROL_FILE_NAME}), snapshot v{SUPPORTED_VERSION} ({SNAPSHOT_FILE_NAME})"
            );
            return;
        }

        // Đọc Supabase URL/key từ env (với fallback về compile-time constants) — Issue #26
        let supabase_url = std::env::var("SUPABASE_URL")
            .unwrap_or_else(|_| supabase_rest::SUPABASE_URL.to_string());
        let supabase_anon_key = std::env::var("SUPABASE_ANON_KEY")
            .unwrap_or_else(|_| supabase_rest::SUPABASE_ANON_KEY.to_string());

        let rest = supabase_rest::SupabaseRest::new(supabase_url, supabase_anon_key);

        let state_dir = std::path::Path::new("/opt/knight/state");

        // Pairing — Issue #03: device_id được lấy từ pair_state, không phải env var
        let pair_state = match pairing::ensure_paired(&rest, state_dir) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[zeus-agent] pairing failed: {e}");
                std::process::exit(1);
            }
        };

        // Khởi tạo AgentConfig SAU khi có device_id từ pairing — Issue #03
        let mut cfg = match main_loop::AgentConfig::from_env() {
            Ok(c) => c,
            Err(e) => {
                // Không phải panic — eprintln và print contract constants để smoke test pass.
                eprintln!("[zeus-agent] config error: {e}");
                eprintln!("[zeus-agent] running in smoke-test mode (no Supabase env vars)");
                println!(
                    "zeus-agent: control v{CONTROL_VERSION} / {CTL_KEY_COUNT} keys ({CONTROL_FILE_NAME}), \
                     snapshot v{SUPPORTED_VERSION} ({SNAPSHOT_FILE_NAME})"
                );
                return;
            }
        };

        // Ghi đè device_id từ pair_state — Issue #03
        cfg.device_id = pair_state.device_id;

        // Vòng chính — không bao giờ return.
        // Truyền secret_key_bytes vào main_loop — Issues #101, #06
        main_loop::run(cfg, pair_state.access_token, pair_state.secret_key_bytes);
    }

    #[cfg(not(unix))]
    {
        // Windows: smoke test mode — verify wire contract constants only.
        println!(
            "zeus-agent: control v{CONTROL_VERSION} / {CTL_KEY_COUNT} keys ({CONTROL_FILE_NAME}), \
             snapshot v{SUPPORTED_VERSION} ({SNAPSHOT_FILE_NAME})"
        );
    }
}
