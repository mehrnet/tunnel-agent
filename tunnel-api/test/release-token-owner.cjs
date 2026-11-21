const assert = require("node:assert/strict");

async function verifyReleaseTokens(worker, env, telegram) {
  env.RELEASE_OPERATOR_TELEGRAM_IDS = "101";
  const send = (path, headers, body) => worker.fetch(new Request("https://test.invalid/api/v1/" + path, {
    method: "POST", headers: { ...headers, "content-type": "application/json" }, body: JSON.stringify(body),
  }), env, {});
  const change = (user, action) => send("web/release-token", user ? { "x-telegram-init-data": telegram(user) } : {}, { action });
  const activate = token => send("admin/bundles/activate", { authorization: "Bearer " + token }, {});
  assert.equal((await change(null, "rotate")).status, 401);
  assert.equal((await change(102, "rotate")).status, 403);
  const issue = async () => {
    const response = await change(101, "rotate");
    assert.equal(response.status, 201);
    assert.equal(response.headers.get("cache-control"), "no-store");
    return (await response.json()).token;
  };
  const first = await issue();
  const stored = await env.DB.prepare("SELECT secret_hash FROM release_api_tokens").first();
  assert.notEqual(stored.secret_hash, first.split(".")[1]);
  assert.equal((await activate(first)).status, 400);
  const rotated = await issue();
  assert.equal((await activate(first)).status, 401);
  env.RELEASE_OPERATOR_TELEGRAM_IDS = "102";
  assert.equal((await activate(rotated)).status, 401);
  env.RELEASE_OPERATOR_TELEGRAM_IDS = "101";
  assert.equal((await change(101, "revoke")).status, 200);
  assert.equal((await activate(rotated)).status, 401);
  const expiring = await issue();
  await env.DB.prepare("UPDATE release_api_tokens SET expires_at=0").run();
  assert.equal((await activate(expiring)).status, 401);
  assert.equal((await activate("former-shared-publisher-token")).status, 401);
  return issue();
}

module.exports = { verifyReleaseTokens };
