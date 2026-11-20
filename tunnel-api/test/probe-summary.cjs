const assert = require("node:assert/strict");
const test = require("node:test");
const { probeSummary, reportState } = require("../web/lib/probe-summary.ts");

const job = { starts_at: 1000, ends_at: 1600, cancelled_at: null, reported_at: null, report: null };

test("unreceived samples are pending during the window, not missed", () => {
  assert.equal(reportState(job, 1100).title, "Awaiting first report");
  assert.equal(probeSummary(job, 1100).pending, 20);
  const partial = { ...job, reported_at: 1100, report: { sequence: 4, passed: 2, failed: 1 } };
  const { passed, failed, missed, pending } = probeSummary(partial, 1310);
  assert.deepEqual({ passed, failed, missed, pending }, { passed: 2, failed: 1, missed: 1, pending: 16 });
  assert.equal(reportState(partial, 1310).title, "Report stale");
});

test("a late final report can replace pending slots with passing results", () => {
  const relayed = { ...job, report: { sequence: 18, passed: 18, failed: 0 } };
  assert.equal(reportState(relayed, 1660).title, "Awaiting final report");
  assert.equal(probeSummary(relayed, 1660).pending, 2);
  assert.equal(probeSummary(relayed, 1800).missed, 2);
  assert.equal(reportState(relayed, 1800).title, "Completed with issues");
  const final = { ...job, report: { sequence: 20, passed: 20, failed: 0 } };
  assert.equal(reportState(final, 1660).title, "All checks passed");
});

test("cancelled runs ignore the part of the window after cancellation", () => {
  const cancelled = { ...job, cancelled_at: 1050, report: { sequence: 2, passed: 2, failed: 0 } };
  assert.equal(probeSummary(cancelled, 1700).missed, 0);
  assert.equal(probeSummary(cancelled, 1700).pending, 0);
  assert.equal(reportState(cancelled, 1700).title, "Cancelled");
});
