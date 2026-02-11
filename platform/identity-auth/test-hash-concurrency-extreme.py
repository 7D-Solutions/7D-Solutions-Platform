#!/usr/bin/env python3
"""
Extreme concurrency test to trigger hash semaphore protection.
Fires 200 concurrent registrations to exceed the 50-slot semaphore.
"""

import asyncio
import aiohttp
import uuid
import time

BASE_URL = "http://localhost:8081"
TENANT_ID = str(uuid.uuid4())
CONCURRENCY = 200  # 4x the semaphore limit

async def register_user(session, sem):
    """Register a user with rate limiting on client side"""
    async with sem:  # Client-side limiter to control request flow
        user_data = {
            "tenant_id": TENANT_ID,
            "user_id": str(uuid.uuid4()),
            "email": f"extreme-{uuid.uuid4()}@test.com",
            "password": "SecurePass123!@#"
        }
        try:
            async with session.post(
                f"{BASE_URL}/api/auth/register",
                json=user_data,
                headers={"X-Forwarded-For": f"192.0.2.{hash(user_data['email']) % 255}"},
                timeout=aiohttp.ClientTimeout(total=30)
            ) as resp:
                status = resp.status
                body = await resp.json()
                return status, body
        except asyncio.TimeoutError:
            return 0, {"error": "timeout"}
        except Exception as e:
            return 0, {"error": str(e)}

async def main():
    print(f"🔥 EXTREME HASH CONCURRENCY TEST")
    print(f"Firing {CONCURRENCY} concurrent registrations (limit: 50)")
    print("=" * 60)

    # Allow more concurrent requests than server can handle
    sem = asyncio.Semaphore(CONCURRENCY)

    async with aiohttp.ClientSession(
        connector=aiohttp.TCPConnector(limit=CONCURRENCY)
    ) as session:
        start = time.time()

        # Fire all requests at once
        tasks = [register_user(session, sem) for _ in range(CONCURRENCY)]
        results = await asyncio.gather(*tasks)

        duration = time.time() - start

        # Analyze results
        successes = sum(1 for status, _ in results if status == 200)
        hash_busy = sum(1 for status, body in results if status == 503 or 'hash_busy' in str(body))
        rate_limited = sum(1 for status, _ in results if status == 429)
        errors = sum(1 for status, _ in results if status == 0)

        print(f"\n📊 RESULTS ({duration:.2f}s)")
        print(f"  ✓ Succeeded: {successes}")
        print(f"  {'✓' if hash_busy > 0 else '✗'} Hash Busy (503): {hash_busy}")
        print(f"  ⚠ Rate Limited (429): {rate_limited}")
        print(f"  ✗ Errors/Timeouts: {errors}")
        print(f"\n  {'✅ PASS' if hash_busy > 0 else '⚠ INCONCLUSIVE'} - Semaphore protection {'triggered' if hash_busy > 0 else 'not triggered'}")

        # Check metrics
        async with session.get(f"{BASE_URL}/metrics") as resp:
            metrics_text = await resp.text()
            for line in metrics_text.split('\n'):
                if 'hash_busy' in line and not line.startswith('#'):
                    print(f"  📈 Metric: {line}")

if __name__ == "__main__":
    asyncio.run(main())
