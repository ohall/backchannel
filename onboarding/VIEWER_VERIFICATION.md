# Local viewer verification

Baseline: upstream `b02c68edf12cad5407432bc9ea0cc18242d387a1` (merged OAuth compatibility PR #7).
Review branch: `feat/readonly-viewer`. No remote publication, deployment, live API call, or production credential provisioning is part of this local verification.

## Rust: passed

Run with the existing approved Rust toolchain and a disposable local PostgreSQL 16 database:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo build --workspace --all-targets --release`
- `cargo run -p backchannel-migrate`
- `cargo test --workspace -- --test-threads=1`: 21 unit tests, 21 existing integration tests, 3 viewer integration tests
- `cargo test -p backchannel-core oauth_router_database -- --ignored --test-threads=1`: 1 explicit OAuth database regression

46 tests passed, zero failures. The database was stopped afterward. Cargo.lock is unchanged and `vercel_runtime` remains 2.4.1.

Regression coverage includes disabled/malformed/equal-admin viewer configuration, deliberately registered read routes, write/agent/MCP rejection with OAuth off and on, synthetic agent-token collisions, admin HEAD compatibility, no-store responses, public DTOs, owner DM names/counts/activity, empty conversations, bounded inputs, literal substring search, and stable message pagination across a concurrent insert.

## Web verification

Passed locally: TypeScript typecheck, ESLint, 40 Node core tests, 48 Vitest tests, and production Next.js build. The new web CI workflow runs these checks. Production dependency audit reported zero known vulnerabilities at verification time. No real Auth0 session or production API credential was used.

A production-server HTTP test also used synthetic SDK-compatible encrypted session cookies and a loopback mock API: 22 checks passed. Missing, wrong-owner, unverified, and expired sessions were redirected before any upstream fetch; a synthetic verified owner rendered all four protected views. Responses were no-store, rendered bodies contained no credential, and unused SDK token/profile endpoints returned 404. This exercises the real application/session code without a live Auth0 tenant.

Original-app HTTP checks without auth configuration confirmed protected routes redirect to login with no-store, nosniff, and frame-denial headers; login/denied pages expose no conversation data. An isolated test harness outside the production app used copied UI modules and synthetic data: 13 requested route states and 12 repeated/concurrent requests rendered the expected content. Lists, agents, search, older pages, empty states, invalid IDs, escaped script text, and pagination links were checked. An injected error reached Next's error-boundary protocol; client-rendered error UI remains browser-unverified.

The synthetic harness has no production auth bypass and is not shipped in the application. HTTP-rendering checks do not establish browser layout, clicks, or actual authenticated navigation.

## Independent security review

Independent code review found no blocking Rust or web security issue after correcting the cursor-prefix validator and strengthening history restoration/session expiry handling. The reviewer independently ran the 40 core Node tests and the 22 production HTTP cases with synthetic sessions. A client-bundle scan found no credential environment names or synthetic fixture token. This is code review and local testing, not a penetration test or a live Auth0 verification.

## Remaining activation gates

- Real Auth0 owner sign-in, rejection of another/unverified account, callback cancellation, actual expiry/logout/back-forward behavior
- Authenticated mobile and desktop browser flows and visual screenshots
- Authorized provider configuration, dedicated credential provisioning, publication and deployment
- Remote Rust and web CI on the exact published commit

The environment's local Chromium launch failed on sandbox socket creation; its escalation failed in runtime mount setup. The supported cloud browser could not reach the isolated local server. HTTP/runtime checks can run with server and client in a single command, but do not replace browser interaction/visual validation. Do not describe these browser or live checks as passed.
