import { importPKCS8, SignJWT } from "jose";
import { randomUUID } from "node:crypto";

const pem = (process.env.JWT_PRIVATE_KEY_PEM ?? "").replace(/\\n/g, "\n");
const privateKey = await importPKCS8(pem, "RS256");
const now = Math.floor(Date.now() / 1000);
const token = await new SignJWT({
  sub: randomUUID(),
  tenant_id: "00000000-0000-0000-0000-000000000000",
  iss: "auth-rs", aud: "7d-platform", iat: now, exp: now + 900,
  jti: randomUUID(), roles: ["admin"], perms: ["platform.tenants.create"],
  actor_type: "user", ver: "1",
}).setProtectedHeader({ alg: "RS256" }).sign(privateKey);

const tid = randomUUID();
let res = await fetch("http://localhost:8091/api/control/tenants", {
  method: "POST",
  headers: { "Content-Type": "application/json", "Authorization": "Bearer " + token },
  body: JSON.stringify({ tenant_id: tid, idempotency_key: "debug-" + tid, environment: "development", product_code: "starter", plan_code: "monthly", concurrent_user_limit: 10 }),
});
console.log("provision:", res.status, res.status !== 200 ? await res.text() : "ok");

const uid = randomUUID();
const email = "debug-" + uid.slice(0, 8) + "@example.com";
res = await fetch("http://localhost:8080/api/auth/register", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ tenant_id: tid, user_id: uid, email, password: "TestPass123!" }) });
console.log("register:", res.status);

res = await fetch("http://localhost:8080/api/auth/login", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ tenant_id: tid, email, password: "TestPass123!" }) });
console.log("login:", res.status);
const setCookie = res.headers.get("set-cookie");
console.log("set-cookie:", setCookie ? setCookie.slice(0, 120) : "NONE");
const body = await res.json() as { access_token?: string; refresh_token?: string };
console.log("access_token:", body.access_token ? "ok" : "NONE");
console.log("refresh_token:", body.refresh_token ? `ok (${body.refresh_token.length} chars)` : "NONE");

if (setCookie) {
  const m = setCookie.match(/refresh=([^;,\s]+)/i);
  const cookieVal = m ? m[1] : null;
  console.log("cookieVal:", cookieVal ? `ok (${cookieVal.length} chars)` : "NONE");
  if (cookieVal && body.refresh_token) {
    res = await fetch("http://localhost:8080/api/auth/refresh", { method: "POST", headers: { "Content-Type": "application/json", "Cookie": "refresh=" + cookieVal }, body: JSON.stringify({ tenant_id: tid, refresh_token: body.refresh_token }) });
    console.log("refresh:", res.status, res.ok ? "ok" : await res.text());
  }
}
