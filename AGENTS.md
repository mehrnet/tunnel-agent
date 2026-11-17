# tunnel-agent Engineering Rules

These rules are enforced locally by `node tools/check-policy.mjs` and in CI.
They apply to first-party code only; generated assets, lockfiles, vendored
sources, and release archives are excluded.

## Layout

- `tunnel-agent/` contains only the Rust node runtime and its examples.
- `tunnel-api/` contains the Cloudflare Worker, D1 migrations, API package
  metadata, and the static SolidJS owner dashboard source in `web/`.
- `tunnel-files/` contains only files published by the assets-only Worker.
- `tools/` contains portable build, publication, and repository-check tooling.
- New top-level directories require a documented responsibility in `README.md`.

## Size And Complexity

- Rust, TypeScript, TSX, JavaScript, CSS, and HTML source files are at most
  500, 300, 300, 300, 300, and 300 physical lines respectively.
- Shell scripts are at most 350 physical lines.
- `tunnel-files/static/install.sh` has a hard 450-line maximum because it
  must remain a self-contained, signed bootstrap verifier; do not split it
  into an unauthenticated fetched helper.
- TypeScript and TSX functions are at most 80 physical lines; ESLint also
  enforces statement, parameter, depth, cyclomatic, and cognitive-complexity
  limits.
- Rust functions are at most 120 physical lines; Clippy enforces this limit.
- Shell functions are at most 100 physical lines.
- No first-party source file is exempt from a documented maximum size.

## Duplication And Dependencies

- Do not duplicate a non-trivial function. Extract a shared module when the
  same normalized implementation appears more than once.
- Prefer the platform and standard library. A dependency needs a concrete
  runtime, correctness, or security justification in the pull request.
- Do not add a framework, code generator, or linter only to avoid small,
  explicit code.
- `npm run quality` runs ESLint, Knip unused-code/dependency detection, and
  jscpd copy-paste detection. Keep the checked paths clean rather than adding
  baselines or exclusions for first-party source.

## Safety And Verification

- Keep API handlers narrow: validate input, call one domain module, return a
  response. D1 access belongs in a focused module.
- Preserve capability checks, token comparisons, and input validation when
  moving code. Refactors must not widen an API route or trust boundary.
- Run `cargo fmt --check`, Clippy, Rust tests, `npm run check`,
  `npm run web:check`, `npm run web:build`, `npm run quality`, shellcheck,
  `node tools/check-policy.mjs`, and both Wrangler dry runs before merging.
- Never commit secrets, generated bundle chunks, `node_modules`, or Wrangler
  state.
