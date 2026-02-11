#!/usr/bin/env python3
"""
Pressure test for auth-rs-v1_4 production hardening features.

Tests:
1. JWKS endpoint under high concurrency
2. Hash concurrency limiting (Argon2 semaphore)
3. Replay detection with client IP logging
4. Rate limiting (per-email, per-token)
5. Metrics validation
"""

import asyncio
import aiohttp
import json
import uuid
import time
from dataclasses import dataclass
from typing import List, Dict, Optional
import sys

BASE_URL = "http://localhost:8081"
TENANT_ID = str(uuid.uuid4())

@dataclass
class TestResult:
    name: str
    success: bool
    duration: float
    details: str
    metrics: Optional[Dict] = None

class LoadTester:
    def __init__(self, base_url: str):
        self.base_url = base_url
        self.results: List[TestResult] = []

    async def get_metrics(self, session: aiohttp.ClientSession) -> Dict:
        """Fetch current Prometheus metrics"""
        async with session.get(f"{self.base_url}/metrics") as resp:
            text = await resp.text()
            metrics = {}
            for line in text.split('\n'):
                if line.startswith('auth_'):
                    if '{' in line:
                        # Parse metric with labels
                        name = line.split('{')[0]
                        value = float(line.split()[-1])
                        if name not in metrics:
                            metrics[name] = []
                        metrics[name].append(value)
                    elif not line.startswith('#'):
                        # Simple metric
                        parts = line.split()
                        if len(parts) == 2:
                            metrics[parts[0]] = float(parts[1])
            return metrics

    async def test_jwks_endpoint_load(self, session: aiohttp.ClientSession, concurrency: int = 100):
        """Test 1: JWKS endpoint under high load"""
        print(f"\n🔑 Test 1: JWKS endpoint ({concurrency} concurrent requests)")
        start = time.time()

        async def fetch_jwks():
            async with session.get(f"{self.base_url}/.well-known/jwks.json") as resp:
                return await resp.json()

        tasks = [fetch_jwks() for _ in range(concurrency)]
        results = await asyncio.gather(*tasks, return_exceptions=True)

        duration = time.time() - start
        successes = sum(1 for r in results if isinstance(r, dict) and 'keys' in r)

        sample_jwk = results[0] if results and isinstance(results[0], dict) else None

        result = TestResult(
            name="JWKS Load Test",
            success=successes == concurrency,
            duration=duration,
            details=f"{successes}/{concurrency} requests succeeded in {duration:.2f}s ({concurrency/duration:.1f} req/s)"
        )

        if sample_jwk:
            print(f"  ✓ JWKS structure: {list(sample_jwk.keys())}")
            if 'keys' in sample_jwk and sample_jwk['keys']:
                key = sample_jwk['keys'][0]
                print(f"  ✓ JWK fields: {list(key.keys())}")
                print(f"  ✓ Algorithm: {key.get('alg')}, Key Type: {key.get('kty')}")

        print(f"  {'✓' if result.success else '✗'} {result.details}")
        self.results.append(result)

    async def test_hash_concurrency_limit(self, session: aiohttp.ClientSession, beyond_limit: int = 60):
        """Test 2: Hash concurrency limiting (configured for 50 concurrent)"""
        print(f"\n⚡ Test 2: Hash Concurrency Limit ({beyond_limit} concurrent registrations)")

        # Get baseline metrics
        metrics_before = await self.get_metrics(session)

        start = time.time()

        async def register_user():
            user_data = {
                "tenant_id": TENANT_ID,
                "user_id": str(uuid.uuid4()),
                "email": f"load-{uuid.uuid4()}@test.com",
                "password": "SecurePass123!@#"
            }
            try:
                async with session.post(
                    f"{self.base_url}/api/auth/register",
                    json=user_data,
                    headers={"X-Forwarded-For": f"192.0.2.{hash(user_data['email']) % 255}"}
                ) as resp:
                    return resp.status, await resp.json()
            except Exception as e:
                return 0, {"error": str(e)}

        # Fire all requests concurrently (should exceed semaphore limit)
        tasks = [register_user() for _ in range(beyond_limit)]
        results = await asyncio.gather(*tasks)

        duration = time.time() - start

        successes = sum(1 for status, _ in results if status == 200)
        hash_busy = sum(1 for status, body in results if status == 503 or 'hash_busy' in str(body))

        # Get metrics after
        metrics_after = await self.get_metrics(session)

        result = TestResult(
            name="Hash Concurrency Limiting",
            success=hash_busy > 0,  # We expect SOME requests to hit the limit
            duration=duration,
            details=f"{successes} succeeded, {hash_busy} hit hash_busy in {duration:.2f}s"
        )

        print(f"  {'✓' if hash_busy > 0 else '⚠'} {result.details}")
        print(f"  {'✓' if hash_busy > 0 else '⚠'} Semaphore protection {'activated' if hash_busy > 0 else 'NOT triggered (may need higher concurrency)'}")

        self.results.append(result)

    async def test_replay_detection_with_ip(self, session: aiohttp.ClientSession):
        """Test 3: Replay detection logs client IP and user-agent"""
        print(f"\n🔐 Test 3: Refresh Token Replay Detection (with client IP logging)")

        # Register and login to get a refresh token
        user_id = str(uuid.uuid4())
        email = f"replay-test-{uuid.uuid4()}@test.com"
        password = "ReplayTest123!"

        # Register
        reg_data = {
            "tenant_id": TENANT_ID,
            "user_id": user_id,
            "email": email,
            "password": password
        }
        async with session.post(f"{self.base_url}/api/auth/register", json=reg_data) as resp:
            if resp.status != 200:
                print(f"  ✗ Registration failed: {await resp.text()}")
                return

        # Login
        login_data = {"tenant_id": TENANT_ID, "email": email, "password": password}
        async with session.post(
            f"{self.base_url}/api/auth/login",
            json=login_data,
            headers={"X-Forwarded-For": "203.0.113.42", "User-Agent": "TestClient/1.0"}
        ) as resp:
            login_result = await resp.json()
            refresh_token = login_result.get("refresh_token")

        if not refresh_token:
            print(f"  ✗ No refresh token received")
            return

        # First refresh (should succeed)
        refresh_data = {"tenant_id": TENANT_ID, "refresh_token": refresh_token}
        async with session.post(
            f"{self.base_url}/api/auth/refresh",
            json=refresh_data,
            headers={"X-Forwarded-For": "203.0.113.42"}
        ) as resp:
            first_refresh = resp.status

        # Second refresh (should fail with replay detection)
        start = time.time()
        async with session.post(
            f"{self.base_url}/api/auth/refresh",
            json=refresh_data,
            headers={"X-Forwarded-For": "198.51.100.99", "User-Agent": "EvilClient/0.1"}
        ) as resp:
            replay_status = resp.status
            replay_body = await resp.text()

        duration = time.time() - start

        # Check logs for IP and user-agent
        with open('/tmp/auth-rs-v1_4.log', 'r') as f:
            logs = f.read()
            has_replay_log = 'refresh_replay_detected' in logs
            has_client_ip = '198.51.100.99' in logs or 'client_ip' in logs
            has_user_agent = 'EvilClient' in logs or 'user_agent' in logs

        result = TestResult(
            name="Replay Detection with Client IP",
            success=replay_status == 401 and has_replay_log and has_client_ip,
            duration=duration,
            details=f"Replay {'blocked' if replay_status == 401 else 'NOT blocked'}, IP logged: {has_client_ip}, UA logged: {has_user_agent}"
        )

        print(f"  {'✓' if first_refresh == 200 else '✗'} First refresh: {first_refresh}")
        print(f"  {'✓' if replay_status == 401 else '✗'} Replay attempt: {replay_status}")
        print(f"  {'✓' if has_replay_log else '✗'} Replay detection logged")
        print(f"  {'✓' if has_client_ip else '✗'} Client IP logged (198.51.100.99)")
        print(f"  {'✓' if has_user_agent else '✗'} User-Agent logged (EvilClient)")

        self.results.append(result)

    async def test_rate_limiting(self, session: aiohttp.ClientSession):
        """Test 4: Rate limiting per-email and per-token"""
        print(f"\n🚦 Test 4: Rate Limiting (LOGIN_PER_MIN_PER_EMAIL=5)")

        email = f"ratelimit-{uuid.uuid4()}@test.com"
        password = "RateLimit123!"

        # Register user
        reg_data = {
            "tenant_id": TENANT_ID,
            "user_id": str(uuid.uuid4()),
            "email": email,
            "password": password
        }
        async with session.post(f"{self.base_url}/api/auth/register", json=reg_data) as resp:
            if resp.status != 200:
                print(f"  ✗ Registration failed")
                return

        # Attempt 10 logins rapidly (limit is 5 per minute)
        login_data = {"tenant_id": TENANT_ID, "email": email, "password": password}

        start = time.time()
        statuses = []
        for i in range(10):
            async with session.post(f"{self.base_url}/api/auth/login", json=login_data) as resp:
                statuses.append(resp.status)
                await asyncio.sleep(0.1)  # Small delay to ensure sequential

        duration = time.time() - start

        successes = sum(1 for s in statuses if s == 200)
        rate_limited = sum(1 for s in statuses if s == 429)

        result = TestResult(
            name="Rate Limiting",
            success=rate_limited > 0,
            duration=duration,
            details=f"{successes} succeeded, {rate_limited} rate-limited (expected after 5th request)"
        )

        print(f"  {'✓' if rate_limited > 0 else '⚠'} {result.details}")
        print(f"  Status sequence: {statuses}")

        self.results.append(result)

    async def validate_metrics(self, session: aiohttp.ClientSession):
        """Test 5: Validate metrics are being recorded"""
        print(f"\n📊 Test 5: Metrics Validation")

        metrics = await self.get_metrics(session)

        required_metrics = [
            'auth_register_total',
            'auth_login_total',
            'auth_refresh_total',
            'auth_http_request_duration_seconds'
        ]

        found_metrics = {}
        for metric in required_metrics:
            if metric in metrics:
                found_metrics[metric] = metrics[metric]

        result = TestResult(
            name="Metrics Validation",
            success=len(found_metrics) >= 3,
            duration=0,
            details=f"{len(found_metrics)}/{len(required_metrics)} required metrics found",
            metrics=found_metrics
        )

        print(f"  {'✓' if result.success else '✗'} {result.details}")
        for metric, value in found_metrics.items():
            if isinstance(value, list):
                print(f"    - {metric}: {len(value)} entries")
            else:
                print(f"    - {metric}: {value}")

        self.results.append(result)

    async def run_all_tests(self):
        """Run all pressure tests"""
        print("=" * 60)
        print("🚀 AUTH-RS-V1_4 PRODUCTION PRESSURE TESTS")
        print("=" * 60)

        async with aiohttp.ClientSession() as session:
            # Test 1: JWKS load
            await self.test_jwks_endpoint_load(session, concurrency=100)

            # Test 2: Hash concurrency limiting
            await self.test_hash_concurrency_limit(session, beyond_limit=60)

            # Test 3: Replay detection with IP
            await self.test_replay_detection_with_ip(session)

            # Test 4: Rate limiting
            await self.test_rate_limiting(session)

            # Test 5: Metrics validation
            await self.validate_metrics(session)

        # Summary
        print("\n" + "=" * 60)
        print("📋 TEST SUMMARY")
        print("=" * 60)

        total_tests = len(self.results)
        passed = sum(1 for r in self.results if r.success)

        for i, result in enumerate(self.results, 1):
            status = "✅ PASS" if result.success else "❌ FAIL"
            print(f"{i}. {status} - {result.name}")
            print(f"   {result.details}")
            if result.duration > 0:
                print(f"   Duration: {result.duration:.2f}s")

        print("\n" + "=" * 60)
        print(f"Overall: {passed}/{total_tests} tests passed ({passed/total_tests*100:.0f}%)")
        print("=" * 60)

        return passed == total_tests

async def main():
    tester = LoadTester(BASE_URL)
    success = await tester.run_all_tests()
    sys.exit(0 if success else 1)

if __name__ == "__main__":
    asyncio.run(main())
