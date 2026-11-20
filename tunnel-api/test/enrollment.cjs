const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync, readdirSync } = require("node:fs");
const { createHash, randomUUID } = require("node:crypto");
const { getPlatformProxy } = require("wrangler");
const { build } = require("esbuild");

test("only one racing grant wins and retirement prevents enrollment", async () => {
  const platform = await getPlatformProxy({ persist: false });
  try {
    const { env } = platform;
    const bundle = await build({ entryPoints: ["src/nodes.ts"], bundle: true, format: "esm", write: false });
    const { handleEnroll, authorizeNode } = await import("data:text/javascript;base64," +
      Buffer.from(bundle.outputFiles[0].contents).toString("base64"));
    for (const name of readdirSync("migrations").filter(n => n.endsWith(".sql")).sort()) {
      const sql = readFileSync("migrations/" + name, "utf8").replace(/^\s*--.*$/gm, "");
      await env.DB.batch(sql.split(";").map(s => s.trim()).filter(Boolean).map(s => env.DB.prepare(s)));
    }
    const run = (sql, ...args) => env.DB.prepare(sql).bind(...args).run();
    const owner = randomUUID(), node = randomUUID(), secret = "b".repeat(64);
    const hash = createHash("sha256").update(secret).digest("hex");
    await run("INSERT INTO owners(id,telegram_user_id,created_at) VALUES(?,?,0)", owner, "123");
    await run("INSERT INTO nodes(id,owner_id,name,node_class,created_at) VALUES(?,?,'test','foreign',0)", node, owner);
    async function grant() {
      const id = randomUUID();
      await run("INSERT INTO enrollment_grants(id,node_id,secret_hash,expires_at,created_at) VALUES(?,?,?, ?,0)",
        id, node, hash, Math.floor(Date.now()/1000)+600);
      return id+"."+secret;
    }
    const enroll = token => handleEnroll(new Request("https://test.invalid", {method:"POST",
      body:JSON.stringify({grant:token, public_key:"fixture"})}), env);
    const tokens = [await grant(), await grant()];
    const results = await Promise.all(tokens.map(enroll));
    const winners = results.filter(r => r.ok);
    assert.equal(winners.length, 1);
    const {node_token} = await winners[0].json();
    const request = new Request("https://test.invalid", {headers:{authorization:"Bearer "+node_token}});
    assert.equal(await authorizeNode(request,env,node), true);
    for (const token of tokens) assert.equal((await enroll(token)).ok,false);
    const unused = await grant();
    await run("UPDATE nodes SET status='revoked' WHERE id=?",node);
    assert.equal((await enroll(unused)).ok,false);
    assert.equal(await authorizeNode(request,env,node),false);
  } finally { await platform.dispose(); }
});
