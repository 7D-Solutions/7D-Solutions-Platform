import { useState, useEffect, useCallback, type ReactNode } from "react";
import type { Session } from "@7d/auth-client";
import { useAuth } from "./useAuth.js";

function formatDate(iso: string): string {
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function renderDeviceInfo(info: Record<string, unknown>): ReactNode {
  const entries = Object.entries(info);
  if (entries.length === 0) return <span>Unknown device</span>;
  return (
    <ul style={{ margin: 0, padding: 0, listStyle: "none" }}>
      {entries.map(([k, v]) => (
        <li key={k}>
          {k}: {String(v)}
        </li>
      ))}
    </ul>
  );
}

export function SessionsPanel() {
  const { client } = useAuth();
  const [sessions, setSessions] = useState<Session[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadSessions = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await client.listSessions();
      setSessions(data);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load sessions");
    } finally {
      setLoading(false);
    }
  }, [client]);

  useEffect(() => {
    void loadSessions();
  }, [loadSessions]);

  const revokeSession = async (sessionId: string) => {
    try {
      await client.revokeSession(sessionId);
      await loadSessions();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to revoke session");
    }
  };

  // Heuristic: the current session has the latest last_used_at.
  // Revoke all others, preserving the most recently used one.
  const signOutOthers = async () => {
    try {
      const all = await client.listSessions();
      const sorted = [...all].sort(
        (a, b) =>
          new Date(b.last_used_at).getTime() - new Date(a.last_used_at).getTime(),
      );
      const others = sorted.slice(1);
      await Promise.all(others.map((s) => client.revokeSession(s.session_id)));
      await loadSessions();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to revoke other sessions");
    }
  };

  if (loading) return <div>Loading sessions…</div>;
  if (error) return <div>Error: {error}</div>;
  if (sessions.length === 0) return <div>No active sessions.</div>;

  return (
    <div>
      <table>
        <thead>
          <tr>
            <th>Device</th>
            <th>Signed in</th>
            <th>Last used</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {sessions.map((s) => (
            <tr key={s.session_id} data-session-id={s.session_id}>
              <td>{renderDeviceInfo(s.device_info)}</td>
              <td>{formatDate(s.issued_at)}</td>
              <td>{formatDate(s.last_used_at)}</td>
              <td>
                <button
                  data-testid={`revoke-${s.session_id}`}
                  onClick={() => void revokeSession(s.session_id)}
                >
                  Revoke
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {sessions.length > 1 && (
        <button onClick={() => void signOutOthers()}>
          Sign out all other devices
        </button>
      )}
    </div>
  );
}
