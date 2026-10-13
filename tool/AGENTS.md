# Rust Tool — Module-specific agent rules

Inherits repository root `AGENTS.md`. Apply only under `tool/`.

- Workspace in `Cargo.toml`: `crates/zeus-core`, `crates/zeus-ui`, `crates/zeus-agent`; Rust edition 2024, rust-version 1.98. Confirm actual toolchain and target OS before suggesting commands.
- Trace the data/control contract between the Rust tool, Zeus snapshot/config transport, launcher and Windows process lifecycle. Source of truth is code and verified schemas; do not infer behavior from file names.
- Keep state transitions and file-write semantics consistent with Java reader constraints. Validate atomicity, paths, encoding, refresh policy, stale snapshot and fail-closed behavior when editing transport.
- UI must not silently take over login, activate Boss Hunt or kill a running MicroEmulator. User/operator approval gates from root `AGENTS.md` always apply.
- When changing a Rust crate, run the smallest relevant package/build tests and inspect downstream consumers. Do not claim a Windows runtime behavior from CI on a different OS.
- Prefer a minimal fix in the responsible crate over duplicating business logic in Java and Rust. Treat protocol/message-format changes as a two-sided compatibility task.
