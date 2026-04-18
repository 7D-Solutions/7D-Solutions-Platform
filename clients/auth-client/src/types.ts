export interface LoginResponse {
  token_type: string;
  access_token: string;
  expires_in_seconds: number;
  refresh_token?: string;
}

export interface Session {
  session_id: string;
  tenant_id: string;
  user_id: string;
  device_info: Record<string, unknown>;
  issued_at: string;
  last_used_at: string;
  expires_at: string;
  absolute_expires_at: string;
}

export interface AuthClientOptions {
  baseUrl: string;
  tenantId: string;
  onLogout?: () => void;
}

export interface AuthClient {
  login(username: string, password: string): Promise<LoginResponse>;
  logout(): Promise<void>;
  refresh(): Promise<string>;
  getAccessToken(): string | null;
  setAccessToken(token: string | null): void;
  listSessions(): Promise<Session[]>;
  revokeSession(sessionId: string): Promise<void>;
}
