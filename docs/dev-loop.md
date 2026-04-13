# Dev Loop Runbook

This is the everyday operational guide for how the Rust dev loop works across the four projects (7D-Solutions Platform, Fireproof-ERP, TrashTech, RanchOrbit). It is the companion to the Rust Service Container Spec at `docs/rust-service-container-spec.md` — the spec is the reference, this is the how-to.

If you are reading this as an agent inside one of those projects and you need to know what commands you can run, what files you can edit, and what to do when something seems stuck, everything you need is below.

## How the dev loop works in one paragraph

You edit a Rust source file in one of the project directories and commit the change. Within thirty seconds, a process running in the background on the host picks up the new commit, compiles the Rust code into a binary targeting the container's Linux architecture, writes the binary into the project's `target/` directory, and then tells Docker to restart the container that uses that binary. The container restarts with the new binary mounted in, takes a few seconds to come back up, and now the running service is the code you just wrote. Everything between "you commit" and "the new binary is serving requests" happens automatically. Your only job is to make the commit.

That's the entire dev loop for the common case. The rest of this document is how to handle the uncommon cases and how to avoid accidentally breaking the common case.

## The three ways to get a change into a running container

There are three and only three. Anything else is either a bug, an emergency, or a mistake. In order of how often you should reach for them:

**1. Commit the change and wait.** This is the normal path. You save your file, run git commit, and the background watcher takes over. Latency is about thirty seconds of polling plus however long the cross-compile takes — typically three to five minutes for a full workspace rebuild. When the build finishes, the container restarts automatically. Use this for all normal feature work and anything that belongs in version control.

**2. Run the cross-compile script directly for a fast local loop.** When you're iterating tightly on a single service — making a change, testing, making another change, testing again — committing every version is wasteful. In this case, you can run the cross-compile script yourself without committing. It builds the binary and puts it in the target directory just like the commit-driven path would, but skips the git commit wait. Inside the container, a small watcher is always polling the binary file for changes, and as soon as the new binary lands it restarts the service. You get your change live in about as long as the compile takes, with no commit required. Use this for tight iteration; commit your work when you're done experimenting.

**3. Restart the container for a config-only change.** When you've edited a config file (not Rust source) that the service reads at startup, and no code change is needed, you use an explicit override command to restart just that one container. The override is named `AGENTCORE_WATCHER_OVERRIDE=1` and it's a prefix you add to a docker restart command. It's the only way to touch docker directly as an agent, and it's logged to an audit file every time you use it. Don't use it for code changes — use path 1 or 2 for those.

**Nothing else is allowed.** No `docker compose up`, no `docker compose down`, no `docker compose build`, no `docker build`, no `docker compose restart`, no `docker restart` without the override. The host has defenses in place that will block any of those from an agent session. If you find yourself typing one of them, stop — you almost certainly want path 1, 2, or 3 above.

## Commands you can run

For reading state and diagnosing problems, the following are always allowed:

- `docker ps` — see what containers are running.
- `docker compose ps` — same thing scoped to the project's compose file.
- `docker compose logs <service>` — read recent logs from a container.
- `docker inspect <container>` — deep detail on one container.
- `docker exec <container> <read-only-command>` — run a read-only command inside a container. Things like `cat`, `ls`, `ps`, `grep`, `sha256sum`, `supervisorctl status`, `stat` all work. Don't try to run commands that modify the container.
- `docker stats` — live resource usage for all running containers.

For making a change effective inside a container, the allowed commands are (per the three restart paths above):

- `git commit` followed by waiting thirty seconds — path 1.
- `./scripts/cargo-slot.sh build -p <package>` — path 2.
- `AGENTCORE_WATCHER_OVERRIDE=1 docker restart <container>` — path 3, config reload only.

That's the complete list.

## Commands you cannot run

Any of these, in any form, will be blocked:

- `docker compose up` — in any variation. The watcher already keeps containers running.
- `docker compose down` — never. This recreates containers and loses state.
- `docker compose build` — never. Image builds don't happen in the dev loop.
- `docker compose restart` (multi-container form) — never. Use the single-container override path if you truly need to restart one container.
- `docker build` and `docker buildx build` — never. Images are built only by the orchestrator, only under explicit user approval.
- `docker restart <container>` without the `AGENTCORE_WATCHER_OVERRIDE=1` prefix — never.
- `docker kill`, `docker rm`, `docker rmi`, `docker stop` — never.
- `docker run`, `docker create` — never. All containers are defined in compose files.
- `docker system prune` — never.

The hook server intercepts any agent attempt to run these and returns a clear error. If you see the error, don't try to work around it by wrapping the command in a shell subshell, using `eval`, setting environment variables to obscure it, or piping through bash — those are all detected too.

## Files you cannot edit

Agents cannot write to any of these files in any project, ever, without the user explicitly flipping a bypass file:

- Any `Dockerfile*` or `docker-compose*.yml` file.
- `watch-binary.sh`, `dev-entrypoint.sh`, `supervisord-dev.conf`, `supervisord.conf`.
- `dev-cross-supervised.sh`, `cargo-slot.sh`, `docker-health-poller.sh`.
- `generate-supervisord-conf.sh` if it still exists.

You can read any of them. You can propose changes in mail or in a child bead. You cannot make the edit yourself. This protection exists because a surprising amount of the pain the dev loop used to cause came from agents editing these files with good intentions and accidentally making the system inconsistent. The lockdown ensures that the only way any of these files change is through a deliberate orchestrator action under user supervision.

## What to do when something seems stuck

Before you assume anything is broken and reach for a nuclear option, walk through this decision tree in order. In practice, the answer is usually at step one or two and you just need to wait.

**Step 1 — Is there a build still running?** Run the build-status check. If cargo is still compiling, your change hasn't landed yet. The correct action is to wait. Full workspace rebuilds take three to five minutes; incremental builds take tens of seconds. Don't do anything — just wait and try again.

**Step 2 — Did the last build fail?** Check the cross-watcher log for the project. If the last build error'd out, your code has a compile error and the container is still running the previous working binary. Fix the compile error, save, and the next build will pick it up. No container action needed.

**Step 3 — Does the binary on the host match the binary in the container?** Compute the sha256 of the binary in the project's target directory on the host, then compute the sha256 of `/app/service` inside the running container. If they match, the binary synced correctly and the issue is something else (a runtime bug, a database problem, a dependency that's down). If they differ, the volume mount is doing its job but the in-container process might be running a stale binary — the primary fix is the commit-driven path's restart, which happens automatically, so wait a few more seconds for the watcher to catch it.

**Step 4 — Is the service process running?** Use `docker exec` to run `supervisorctl status service` inside the container. If it reports RUNNING, the process is fine; your issue is elsewhere. If it reports FATAL, STOPPED, or BACKOFF, the binary is probably crashing on startup or the container is in a degraded state. Check the container's logs for the crash reason. If you need to force a restart to recover, use path 3 above with the override prefix.

**Step 5 — If none of the above diagnose it, mail the orchestrator.** Don't try to force anything. Don't run `docker compose down`. Don't try to rebuild the runtime image. Don't touch any of the locked files. Mail the orchestrator with what you tried and what you observed. The orchestrator has additional paths available that agents don't.

## When to mail the orchestrator

Mail the orchestrator whenever you hit any of these:

- Step 5 of the decision tree above — you tried everything and it's still broken.
- You need to change a file that's on the locked list and you have a legitimate reason.
- You need to do something that's on the blocked command list and the override path doesn't cover your case.
- Something is physically destroyed or corrupted — binary is half-written, container crash-loops even on a known-good binary, supervisor itself won't start.
- Anything that needs the hook bypass flipped. Agents don't flip it themselves.

Don't mail the orchestrator for normal diagnostic questions that can be answered by reading logs. Do mail the orchestrator whenever you're uncertain and about to touch something that might cascade.

## Known behaviors that look weird but are correct

These are not bugs. If you see any of them, the dev loop is working as designed.

- **An empty git commit causes a full workspace rebuild.** The cross-watcher polls git for new commit hashes, not for content changes. Any new commit — including an empty one — triggers cargo-slot, which starts fresh and rebuilds. This takes the normal three to five minutes even though nothing actually changed.
- **Binary checksums are different between builds of the same source.** Rust debug builds embed timestamps, build paths, and other non-deterministic content. Even if you rebuild the exact same source twice, you'll get two different sha256 values. Don't use "same sha = same code" as a correctness check.
- **The first build after a cold cache takes many minutes.** Cargo-slot nukes its slot directory between builds to keep disk usage under control. That means the first build in a given slot starts from scratch and compiles all dependencies. Subsequent builds in the same slot are incremental and fast. This is a design trade-off.
- **The in-container watcher rarely fires in the normal path.** Because the cross-watcher on the host calls docker restart directly as part of the commit-driven path, the container cold-starts with the new binary already in place. The in-container watcher that polls `/app/service` for checksum changes is a safety net for cases where the cross-watcher didn't notice — it's not the primary restart mechanism. If you're debugging and you never see the in-container watcher fire, that's correct.
- **Docker for Mac bind mount updates can lag by a few seconds.** Virtiofs (the filesystem layer between your Mac and the Docker VM) sometimes caches file metadata briefly. A binary you just wrote on the host might take two or three extra seconds to become visible with a new mtime inside the container. The watcher's polling interval handles this — it's why the polling algorithm waits for checksum stability over two consecutive reads before accepting a change.

## Where the full spec lives

Everything in this runbook is distilled from `docs/rust-service-container-spec.md` in AgentCore. When you need technical detail beyond what's here — the exact Dockerfile contents, the supervisord config, the watcher algorithm, the conformance checks, the onboarding checklist for new projects — read the spec directly. When the spec and this runbook disagree, the spec wins.

When the runbook itself needs updating (new failure mode observed, new escalation path, new known behavior), that's an orchestrator change under user supervision. Don't edit this file yourself as an agent.
