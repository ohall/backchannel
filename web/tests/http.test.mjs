// Runs only against the local production build with synthetic SDK-encrypted cookies.
// No test route, authentication override, or real Auth0 account is involved.
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import assert from "node:assert/strict";
import { encrypt } from "../node_modules/@auth0/nextjs-auth0/dist/server/cookies.js";

const secret = "ab".repeat(32);
const token = "synthetic-http-viewer-token";
let reads = 0;
const api = createServer((request, response) => {
  assert.equal(request.headers.authorization, `Bearer ${token}`);
  reads++;
  response.setHeader("content-type", "application/json");
  response.end(JSON.stringify({ items: [], next_cursor: null, has_more: false }));
});
await new Promise(resolve => api.listen(3202, "127.0.0.1", resolve));
const child = spawn(process.execPath, [
  "node_modules/next/dist/bin/next", "start", "--hostname", "127.0.0.1", "--port", "3201",
], {
  env: {
    ...process.env,
    AUTH0_SECRET: secret,
    AUTH0_DOMAIN: "fixture.auth0.invalid",
    AUTH0_CLIENT_ID: "synthetic-client",
    AUTH0_CLIENT_SECRET: "synthetic-client-secret",
    APP_BASE_URL: "http://127.0.0.1:3201",
    BACKCHANNEL_API_URL: "http://127.0.0.1:3202",
    BACKCHANNEL_VIEWER_TOKEN: token,
    ALLOWED_EMAILS: "oakley349@gmail.com",
  },
  stdio: ["ignore", "ignore", "inherit"],
});
try {
  let ready = false;
  for (let attempt = 0; attempt < 50; attempt++) {
    try {
      await fetch("http://127.0.0.1:3201/login");
      ready = true;
      break;
    } catch {
      await new Promise(resolve => setTimeout(resolve, 200));
    }
  }
  assert.ok(ready, "production server started");
  const now = Math.floor(Date.now() / 1000);
  const good = {
    user: { sub: "synthetic-user", email: "oakley349@gmail.com", email_verified: true },
    tokenSet: { accessToken: "synthetic-auth-access", expiresAt: now + 3600 },
    internal: { sid: "synthetic-session", createdAt: now - 60 },
  };
  const cases = [
    ["missing", null, 307],
    ["wrong", { ...good, user: { ...good.user, email: "wrong@example.com" } }, 307],
    ["unverified", { ...good, user: { ...good.user, email_verified: false } }, 307],
    ["expired", { ...good, internal: { ...good.internal, createdAt: now - 28801 } }, 307],
    ["valid", good, 200],
  ];
  for (const [name, session, status] of cases) {
    const before = reads;
    const cookie = session ? `__session=${await encrypt(session, secret, now + 3600)}` : "";
    for (const path of ["/", "/agents", "/search?search=hello", "/conversations/00000000-0000-0000-0000-000000000001"]) {
      const response = await fetch(`http://127.0.0.1:3201${path}`, { headers: { cookie }, redirect: "manual" });
      const body = await response.text();
      assert.equal(response.status, status, `${name} ${path}`);
      assert.match(response.headers.get("cache-control"), /no-store/);
      assert.ok(!body.includes(token));
      assert.ok(!body.includes(secret));
      console.log(name, path, status, response.headers.get("cache-control"));
    }
    assert.equal(reads - before, name === "valid" ? 4 : 0, `${name} upstream reads`);
  }
  for (const path of ["/auth/access-token", "/auth/profile"]) {
    const response = await fetch(`http://127.0.0.1:3201${path}`, { redirect: "manual" });
    assert.equal(response.status, 404);
    console.log(path, 404);
  }
  console.log("PASS: 22 production HTTP cases, upstream authorization, no-store, and secret redaction.");
} finally {
  child.kill();
  api.close();
}
