// ============================================================
// BFF: GET/PUT /api/preferences/:key
// Per-user UI preference storage. Scoped by JWT sub + key.
// TODO: Wire to a real backend preferences service when available.
//       Currently uses in-memory store (does not survive server restart).
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';

// In-memory store: Map<"userId:prefKey", value>
// TODO: Replace with backend call to identity-auth preferences endpoint
const store = new Map<string, unknown>();

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ key: string }> }
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { key } = await params;
  const storeKey = `${auth.sub}:${key}`;
  const value = store.get(storeKey);

  if (value === undefined) {
    return NextResponse.json({ error: 'Not found' }, { status: 404 });
  }

  return NextResponse.json(value);
}

export async function PUT(
  request: NextRequest,
  { params }: { params: Promise<{ key: string }> }
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { key } = await params;
  const body = await request.json();
  const storeKey = `${auth.sub}:${key}`;

  store.set(storeKey, body.value);

  return NextResponse.json({ ok: true });
}

export async function DELETE(
  _request: NextRequest,
  { params }: { params: Promise<{ key: string }> }
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { key } = await params;
  const storeKey = `${auth.sub}:${key}`;

  store.delete(storeKey);

  return NextResponse.json({ ok: true });
}
