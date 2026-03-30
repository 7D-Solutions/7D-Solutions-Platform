import { importPKCS8, SignJWT } from "jose";
import { randomUUID } from "node:crypto";

async function main() {
  const pem = process.env.JWT_PRIVATE_KEY_PEM;
  if (!pem) {
    console.error("JWT_PRIVATE_KEY_PEM required");
    process.exit(1);
  }
  const pk = await importPKCS8(pem, "RS256");
  const now = Math.floor(Date.now() / 1000);
  const token = await new SignJWT({
    sub: randomUUID(),
    tenant_id: randomUUID(),
    iss: "auth-rs",
    aud: "7d-platform",
    iat: now,
    exp: now + 900,
    jti: randomUUID(),
    roles: ["admin"],
    perms: ["inventory.read", "inventory.mutate"],
    actor_type: "user",
    ver: "1",
  })
    .setProtectedHeader({ alg: "RS256" })
    .sign(pk);
  process.stdout.write(token);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
