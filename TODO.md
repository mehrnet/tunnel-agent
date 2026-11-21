# Stable release pipeline

Build each version once on GitHub Actions, qualify the exact release on
`w4r-canary`, and promote it only after the tests pass and an operator
approves. D1 controls what agents install. The GitHub `stable` tag mirrors
the D1 decision; moving the tag alone must not deploy anything.

## 1. Establish release inputs

- [ ] Use the existing [GitHub build workflow](.github/workflows/release.yml).
  Require a successful CI run for the same commit and both static release
  archives (`amd64` and `arm64`). Never rebuild for promotion.
- [ ] Record the version, commit, Actions run ID, and both archive and inventory
  SHA-256 digests. Verify downloaded assets against those digests before use.
- [ ] Fix the publishing credential that returned HTTP 401. Give the testing
  workflow credentials that can only activate testing; keep stable promotion
  credentials in a separate protected GitHub environment.
- [ ] Record the current stable version, rollback artifacts, and baseline
  latency, availability, reconnect time, CPU, and RSS. Set regression budgets
  before the candidate run.

## 2. Implement the live canary workflow

- [ ] Add a manually dispatched `canary-release` workflow that takes an exact
  version tag. Serialize runs so they cannot replace each other's candidate.
- [ ] Verify that `w4r-canary` contains only the expected enrolled nodes,
  `de-1` and `ir-6`, with no stable-cluster memberships. Record both nodes'
  current versions and manifest revisions before changing anything.
- [ ] Publish assets with [the existing publisher](tools/publish.sh), move
  only the testing pointer, and verify that stable is unchanged. Wait for
  `de-1` to update from the published archive and for `ir-6` to update through
  its relay without contacting the API. Verify the running binaries and
  signed inventories on both nodes.
- [ ] Apply fault tests only to dedicated canary lanes. Preserve customer
  listeners, use bounded timeouts, restore test changes even on failure, and
  confirm the original topology and health before ending the job.
- [ ] Add a native ARM64 install, startup, and forwarding smoke test on a
  separate test host. Both current W4R canary nodes are AMD64.

## 3. Require a passing canary report

| Check | Pass condition |
| --- | --- |
| Parallel management | HPNSSH, WSTunnel, and Rathole each deliver a valid signed configuration and a checksum-verified bundle blob. A TCP connection alone does not pass. |
| Lane failover | Stop each lane in turn, then run with each transport as the sole surviving lane. A new signed revision, heartbeat, and telemetry reach their destinations within the agreed recovery budget. Restoring a lane does not restart the others. |
| Interrupted update | Interrupt a transfer before activation. The current release remains usable; the download resumes or retries through another lane and activates only after verification. Use a test fixture or a separate update target so the test transfers data, not just cache hits. |
| Invalid or stale data | Reject a bad signature, corrupt blob, wrong inventory, and older signed revision. An old relay must not silently downgrade a redundant domestic node. |
| Rollback | Restore the previous bundle and a compatible manifest, then verify signed relay traffic and an isolated customer-path probe. Rehearse on the canary and restore the candidate before starting its soak. |
| Soak and regression | Probe each management path and an isolated customer path. Compare against the stable baseline. Use 24 hours for routine changes and seven days for management or updater changes. Missing samples, restart loops, or regressions beyond the agreed budget fail the gate. |

- [ ] Publish a report with the release identity and digests, node scope,
  timestamps, per-check results, metrics versus baseline, cleanup status,
  and ARM64 evidence. Incomplete or skipped required checks are not passes.
- [ ] Qualify `v0.5.7` using this report format. Its live W4R upgrade and signed
  three-lane checks have passed, but they are not sufficient for promotion.

## 4. Implement stable promotion and rollback

- [ ] Verify that the registered domestic mirrors serve the identical bundles
  for both architectures. Test fallback from a domestic node with the first
  mirror deliberately unavailable; verify the reconstructed archive digest.
- [ ] Add a manually dispatched `promote-stable` workflow. Before approval,
  show the candidate report and exact digests. After approval, recheck that
  the candidate still matches the testing pointer and report. Reject a newer
  candidate, a changed digest, or any missing gate.
- [ ] Make the [activation API](tunnel-api/src/bundles.ts) require the passing
  report for stable, not just a version registered on testing. Update both
  architectures atomically, audit the change, and read both records back.
- [ ] Move GitHub `stable` only after confirming D1. Make this step safely
  retryable if GitHub fails after D1 succeeds. Never move the immutable version
  tag or rebuild its assets.
- [ ] Watch adoption, service health, and telemetry after promotion. Document
  the stop and rollback procedure. A pointer rollback alone does not downgrade
  domestic nodes with multiple active lanes; the procedure must restore the
  previous verified bundle and compatible manifest explicitly.

Reference: [Release and canary runbook](docs/BOOTSTRAP.md).
