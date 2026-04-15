# Notifications Crash-Loop Health

## Purpose

Use this when `docker ps` shows the notifications container as healthy even
though the service is panicking and being restarted repeatedly.

The notifications service now records each process start in a container-local
history file and the `/api/health` endpoint fails closed when there are more
than three restarts within the last two minutes.

## Prerequisites

- Docker access on the host.
- A running notifications container.

## Procedure

1. Check the Docker health status:

```bash
docker ps --filter 'health=unhealthy' --format '{{.Names}}\t{{.Status}}'
```

2. If notifications is not listed but logs show repeated panics, inspect the
   startup history from inside the container:

```bash
docker exec <notifications-container> cat /tmp/notifications-startup-history.log
```

3. Confirm the service is failing closed:

```bash
curl -sf http://localhost:8089/api/health
```

Expected on a crash-loop: `503 Service Unavailable` and a JSON body with
`status: "unhealthy"` plus `recent_starts`.

## Verification

- `docker ps --filter 'health=unhealthy'` lists the notifications container.
- `curl -f http://localhost:8089/api/health` fails while the service is in a
  crash-loop.

## Rollback

If the health file becomes unreadable, remove it from the container and restart
the service so a fresh startup history is recorded on the next boot.
