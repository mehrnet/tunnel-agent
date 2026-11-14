# tunnel-agent Engineering Rules

These rules are enforced locally by `node tools/check-policy.mjs` and in CI.
They apply to first-party code only; generated assets, lockfiles, vendored
sources, and release archives are excluded.

## Layout

- `tunnel-agent/` contains only the Rust node runtime and its examples.
- `tunnel-api/` contains only the Cloudflare Worker, D1 migrations, and API
  package metadata.
- `tunnel-files/` contains only files published by the assets-only Worker.
- `tools/` contains portable build, publication, and repository-check tooling.
- New top-level directories require a documented responsibility in `README.md`.

## Size And Complexity

- TypeScript source files are at most 300 physical lines.
- New Rust source files are at most 500 physical lines.
- Shell scripts are at most 350 physical lines.
- TypeScript functions are at most 90 physical lines.
- Rust functions are at most 120 physical lines; Clippy enforces this limit.
- `tunnel-agent/src/manifest.rs` is the sole temporary size exception. It may
  not exceed 1,850 lines. Split validation, compilation, and artifact handling
  into modules before making substantial changes there.

## Duplication And Dependencies

- Do not duplicate a non-trivial TypeScript function. Extract a shared module
  when the same normalized implementation appears more than once.
- Prefer the platform and standard library. A dependency needs a concrete
  runtime, correctness, or security justification in the pull request.
- Do not add a framework, code generator, or linter only to avoid small,
  explicit code.

## Safety And Verification

- Keep API handlers narrow: validate input, call one domain module, return a
  response. D1 access belongs in a focused module.
- Preserve capability checks, token comparisons, and input validation when
  moving code. Refactors must not widen an API route or trust boundary.
- Run `cargo fmt --check`, Clippy, Rust tests, TypeScript checks, shellcheck,
  `node tools/check-policy.mjs`, and both Wrangler dry runs before merging.
- Never commit secrets, generated bundle chunks, `node_modules`, or Wrangler
  state.
