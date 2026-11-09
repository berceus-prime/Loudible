# Prevent new jobs from entering during sign-out

**Context:** `crates/loudible-presentation/src/app_runtime.rs` runs each command in its own local task. `AppViewModel::sign_out` in `crates/loudible-presentation/src/app_view_model.rs` awaits `export.suspend()` before clearing the session. Suspension waits for jobs to finish and then clears the queue.

**Problem:** While sign-out waits, a later download or export command can still enqueue work under the signed-in session. Sign-out can then wait on that new work, stall, or clear it as part of suspension.

**Done when:** Starting sign-out closes admission to new account-bound jobs before the first await. Define how commands arriving during sign-out are handled and test the ordering with controlled tasks.
