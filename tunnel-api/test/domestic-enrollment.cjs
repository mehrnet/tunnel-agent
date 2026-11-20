const assert = require("node:assert/strict");
const { createHash, createHmac, generateKeyPairSync, randomUUID, sign } = require("node:crypto");
const { readFileSync, readdirSync } = require("node:fs");
const { test } = require("node:test");
const { build } = require("esbuild");
const { getPlatformProxy } = require("wrangler");

function signedTelegram(id, token) {
  const params = new URLSearchParams({ auth_date: String(Math.floor(Date.now()/1000)), user: JSON.stringify({ id }) });
  const secret = createHmac("sha256", "WebAppData").update(token).digest();
  params.set("hash", createHmac("sha256", secret).update([...params].map(([k,v]) => k+"="+v).join("\n")).digest("hex"));
  return params.toString();
}

function proof(pair, grant, nodeId) {
  const public_key = pair.publicKey.export({ type: "spki", format: "der" }).toString("base64");
  const payload = ["tunnel-agent-domestic-enrollment-v1",nodeId,grant,public_key].join("\n");
  return { grant, public_key, signature: sign(null,Buffer.from(payload),pair.privateKey).toString("base64") };
}

async function relaySnapshot(worker, env, nodeId, token, etag) {
  const headers = new Headers();
  if (token) headers.set("authorization", `Bearer ${nodeId}.${token}`);
  if (etag) headers.set("if-none-match", etag);
  return worker.fetch(new Request(`https://test.invalid/api/v1/nodes/${nodeId}/relay-documents`, { headers }), env, {});
}

async function checkRelayCache(worker, env, relay, domestic, secret) {
  const snapshot = await relaySnapshot(worker, env, relay, secret);
  assert.equal(snapshot.status, 200);
  const tag = snapshot.headers.get("etag");
  assert.ok(tag);
  assert.deepEqual((await snapshot.json()).documents.map(row => row.node_id), [domestic]);
  const unchanged = await relaySnapshot(worker, env, relay, secret, tag);
  assert.equal(unchanged.status, 304);
  assert.equal(await unchanged.text(), "");
  assert.equal((await relaySnapshot(worker, env, relay, null, tag)).status, 401);
  return tag;
}

test("domestic proof is relay-scoped; reinstall, rotation, and retirement invalidate old identities", async () => {
  const platform = await getPlatformProxy({ persist: false });
  try {
    const signingKey = generateKeyPairSync("ed25519").privateKey.export({ type: "pkcs8", format: "der" });
    const env = { ...platform.env, TELEGRAM_BOT_TOKEN: "domestic-test-bot", CAPSULE_SIGNING_PRIVATE_KEY: signingKey.toString("base64") };
    const bundle = await build({ entryPoints: ["src/worker.ts"], bundle:true, format:"esm", write:false });
    const { default: worker } = await import("data:text/javascript;base64,"+Buffer.from(bundle.outputFiles[0].contents).toString("base64"));
    for (const name of readdirSync("migrations").filter(n => n.endsWith(".sql")).sort()) {
      const sql = readFileSync("migrations/"+name,"utf8").replace(/^\s*--.*$/gm,"");
      await env.DB.batch(sql.split(";").map(s=>s.trim()).filter(Boolean).map(s=>env.DB.prepare(s)));
    }
    const call = (path, body, headers = {}) => worker.fetch(new Request("https://test.invalid/api/v1"+path,
      {method:"POST",headers,body:JSON.stringify(body)}),env,{});
    const owner = (path, body, user=113) => call("/web"+path,body,{"x-telegram-init-data":signedTelegram(user,env.TELEGRAM_BOT_TOKEN)});
    const { cluster } = await (await owner("/clusters",{name:"identity-fixture"})).json();
    const { cluster: isolated } = await (await owner("/clusters",{name:"unrelated-fixture"})).json();
    const create = async (name,role,cluster_id) => (await (await owner("/nodes",{name,role,cluster_id,address:"test.invalid"})).json()).node.id;
    const domestic = await create("domestic","domestic",cluster.id);
    const relay = await create("relay","foreign",cluster.id);
    const outsider = await create("outsider","foreign",isolated.id);
    const run = (sql,...args) => env.DB.prepare(sql).bind(...args).run();
    const snapshot = () => env.DB.prepare("SELECT status, public_key FROM nodes WHERE id=?").bind(domestic).first();
    const grantPath = "/nodes/"+domestic+"/domestic-grant";
    const issue = async key => {
      const public_key = key.publicKey.export({type:"spki",format:"der"}).toString("base64");
      const reply = await owner(grantPath,{public_key});
      assert.equal(reply.status,200);
      return (await reply.json()).grant;
    };
    const secret = randomUUID();
    for (const id of [relay, outsider]) {
      await run("UPDATE nodes SET status='enrolled' WHERE id=?",id);
      await run("INSERT INTO node_credentials VALUES(?,?,0,0)",id,createHash("sha256").update(secret).digest("hex"));
    }
    const relayCall = (id,path,body) => call("/nodes/"+id+path,body,{authorization:"Bearer "+id+"."+secret});
    const submit = (body,id=relay) => relayCall(id,"/domestic-enroll/"+domestic,body);
    const lifecycle = action => owner("/nodes/"+domestic+"/lifecycle",{action,confirm_node_id:domestic});
    assert.equal((await owner(grantPath,{},114)).status,404);
    assert.equal((await relayCall(relay,"/heartbeats",{observations:[{node_id:domestic,observed_at:Math.floor(Date.now()/1000)}]})).status,200);
    assert.equal((await snapshot()).status,"pending");
    let key = generateKeyPairSync("ed25519");
    const grant = await issue(key);
    const first = proof(key,grant,domestic);
    assert.equal((await submit(first,outsider)).status,403);
    assert.equal((await submit(proof(key,grant,outsider))).status,401);
    assert.equal((await submit(proof(generateKeyPairSync("ed25519"),grant,domestic))).status,409);
    const competing = await Promise.all([submit(first),submit(first)]);
    assert.equal(competing.filter(reply=>reply.status===200).length,1);
    assert.deepEqual(await snapshot(),{status:"enrolled",public_key:first.public_key});
    assert.equal((await submit(first)).status,401);
    const originalTag = await checkRelayCache(worker, env, relay, domestic, secret);
    await run("UPDATE node_documents SET revision=revision+1 WHERE node_id=?",domestic);
    const updated = await relaySnapshot(worker, env, relay, secret, originalTag);
    assert.equal(updated.status, 200);
    const updatedTag = updated.headers.get("etag");
    assert.notEqual(updatedTag, originalTag);
    for (const action of ["reinstall","rotate"]) {
      assert.equal((await lifecycle(action)).status,200);
      assert.deepEqual(await snapshot(),{status:"pending",public_key:null});
      const stale = await issue(key);
      assert.equal((await submit(proof(key,stale,domestic))).status,409);
      key = generateKeyPairSync("ed25519");
      const fresh = await issue(key);
      assert.equal((await submit(proof(key,fresh,domestic))).status,200);
    }
    assert.equal((await lifecycle("retire")).status,200);
    assert.deepEqual(await snapshot(),{status:"revoked",public_key:null});
    assert.equal((await owner(grantPath,{})).status,409);
    assert.equal((await submit(first)).status,403);
    const removed = await relaySnapshot(worker, env, relay, secret, updatedTag);
    assert.equal(removed.status, 200);
    assert.deepEqual((await removed.json()).documents, []);
    assert.equal(await env.DB.prepare("SELECT COUNT(*) AS n FROM cluster_nodes WHERE node_id=?").bind(domestic).first("n"),0);
  } finally { await platform.dispose(); }
});
