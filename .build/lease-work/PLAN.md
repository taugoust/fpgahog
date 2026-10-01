# Per-resource lease implementation plan

1. Add an isolated lease store in `src/leases.rs`, separate from legacy `DiskState`: configurable state path for tests, flock-guarded read/modify/write, private directory/files, temp-file + fsync + rename + directory fsync.
2. Define versioned JSON records with opaque resource IDs, random token, numeric authenticated UID, session metadata, mode, timestamps, expiry. Prune expired records on operations only; no hardware action. Use OS randomness via `/dev/urandom` and kernel UID; reject malformed state and invalid input.
3. Add additive `lease acquire|renew|status|release` CLI emitting stable JSON v1 on stdout; errors JSON with deterministic exit classes. UID/token jointly authorize mutation; unique token per session. Document advisory nature and unsafe-expiry limitation.
4. Add unit/CLI tests using isolated temporary paths for conflict, parallel acquisition, independent IDs, authorization, renew/expiry, malformed state, JSON protocol and durability/migration isolation. Run Nix devshell checks, inspect diff, commit/push task-only changes on current branch.
