import { jwtDecode } from "jwt-decode";

/**
 * Canonical JWT payload issued by identity-auth (auth-rs).
 * Mirrors AccessClaims in platform/identity-auth/src/auth/jwt.rs.
 */
export interface AccessClaims {
  sub: string;         // user_id (UUID)
  iss: string;         // "auth-rs"
  iat: number;         // issued-at (Unix seconds)
  exp: number;         // expires-at (Unix seconds)
  tenant_id: string;   // UUID
  roles: string[];
  perms: string[];
  actor_type: string;  // "user" | "service"
  ver: string;
}

export function decodeClaims(token: string): AccessClaims {
  return jwtDecode<AccessClaims>(token);
}

/** Returns true when the token is within `bufferSeconds` of expiry. */
export function isExpired(claims: AccessClaims, bufferSeconds = 30): boolean {
  return Date.now() / 1000 >= claims.exp - bufferSeconds;
}
