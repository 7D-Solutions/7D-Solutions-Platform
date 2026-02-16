# Supervisord Usage for AgentCore

## Overview

AgentCore uses supervisord running **inside tmux** to manage daemon processes:
- 4 mail monitors (one per agent)
- 1 beadmonitor (stale bead detection)

**Architecture:** Supervisord runs in **foreground mode** in a dedicated tmux window (`agentcore:supervisord`), keeping everything visible and integrated with the Flywheel ecosystem.

## Quick Commands

```bash
# Start supervisord in dedicated tmux window (foreground mode)
tmux send-keys -t agentcore:supervisord "supervisord -c config/supervisord.conf -n" Enter

# Check status of all processes
supervisorctl -c config/supervisord.conf status

# Stop all processes and supervisord
# (Send Ctrl+C to the tmux window)
tmux send-keys -t agentcore:supervisord C-c

# Restart a specific monitor
supervisorctl -c /Users/james/Projects/AgentCore/config/supervisord.conf restart agentcore-monitors:mail-monitor-orangelantern

# Restart all monitors
supervisorctl -c /Users/james/Projects/AgentCore/config/supervisord.conf restart agentcore-monitors:*

# Tail logs for a process
supervisorctl -c /Users/james/Projects/AgentCore/config/supervisord.conf tail -f agentcore-monitors:mail-monitor-orangelantern
```

## Tmux Integration (Flywheel Architecture)

**Why tmux foreground mode?**
- ✅ Everything visible in one iTerm2/tmux view
- ✅ Integrated with Agent Flywheel ecosystem
- ✅ Easy monitoring alongside agents
- ✅ Follows Flywheel philosophy: tmux as command center
- ✅ Live supervisord output in dedicated window
- ✅ No hidden background daemons

**Auto-Start on Project Launch:**
- ✅ Supervisord starts automatically when you run `./start`
- ✅ Creates dedicated tmux window: `agentcore:supervisord`
- ✅ All monitors (mail + beadmonitor) start automatically
- ✅ No manual intervention needed

**Tmux Window:**
- Window name: `agentcore:supervisord`
- Mode: Foreground (`-n` flag)
- Output: Live logs visible in window

**Switch to supervisord window:**
```bash
tmux select-window -t agentcore:supervisord
```

**Manual control:**
```bash
# Stop supervisord (optional - auto-stops when session closes)
./scripts/stop-supervisord.sh
```

## Configuration

- Config file: `/Users/james/Projects/AgentCore/config/supervisord.conf`
- Log files: `/Users/james/Projects/AgentCore/logs/`
- PID file: `/Users/james/Projects/AgentCore/pids/supervisord.pid`
- Socket: `/Users/james/Projects/AgentCore/tmp/supervisor.sock`
- **Tmux window**: `agentcore:supervisord` (foreground mode)

## Process Groups

All monitors belong to the `agentcore-monitors` group:
- `mail-monitor-orangelantern`
- `mail-monitor-quietcreek`
- `mail-monitor-fuchsiadog`
- `mail-monitor-topazdeer`
- `beadmonitor`

## Auto-Restart Behavior

Supervisord automatically restarts processes that:
- Exit with non-zero status (crashes)
- Get killed (SIGKILL, SIGTERM, etc.)
- Hang/zombie (not yet - needs separate watchdog)

Configuration:
- `autorestart=true` - restart on any exit
- `startretries=5` - try up to 5 times
- `startsecs=10` - process must stay up 10s to be considered "started"

## Logs

Each process has separate log files:
- stdout: `/Users/james/Projects/AgentCore/logs/<process-name>.log`
- stderr: `/Users/james/Projects/AgentCore/logs/<process-name>.err`

Supervisor main log:
- `/Users/james/Projects/AgentCore/logs/supervisord.log`

## Migration from launchd

✅ **Completed:**
- Unloaded launchd plists (mailwatchdog, beadmonitor)
- Backed up plists to `/Users/james/Projects/AgentCore/tmp/`
- Created supervisord.conf with 5 processes
- Tested auto-restart on crash
- Verified mail notifications work
- Verified beadmonitor running

❌ **Removed:**
- Individual launchd jobs for each monitor
- Watchdog process (supervisord replaces it)

## Future Integration

Phase 2 will integrate supervisord into `./start` script:
- Auto-start supervisord when project starts
- Auto-stop on project shutdown
- Clean initialization and cleanup

## Troubleshooting

**Check if supervisord is running:**
```bash
ps aux | grep supervisord | grep -v grep
```

**Verify socket exists:**
```bash
ls -la /Users/james/Projects/AgentCore/tmp/supervisor.sock
```

**View real-time logs:**
```bash
tail -f /Users/james/Projects/AgentCore/logs/supervisord.log
```

**Restart everything:**
```bash
supervisorctl -c /Users/james/Projects/AgentCore/config/supervisord.conf shutdown
supervisord -c /Users/james/Projects/AgentCore/config/supervisord.conf
```

## Comparison: launchd vs supervisord vs tmux-integrated supervisord

| Feature | launchd | supervisord (daemon) | supervisord (tmux) |
|---------|---------|----------------------|--------------------|
| Auto-restart | ✅ (unreliable with jetsam) | ✅ (reliable) | ✅ (reliable) |
| Process grouping | ❌ | ✅ | ✅ |
| Unified logging | ❌ | ✅ | ✅ |
| Visibility | ❌ (hidden) | ❌ (background) | ✅ (tmux window) |
| Flywheel integration | ❌ | ⚠️ (separate) | ✅ (integrated) |
| Cross-platform | macOS only | Linux, macOS, etc. | Linux, macOS, etc. |
| Configuration | plist (XML) | ini format | ini format |
| Debugging | Complex | Simple | **Very simple** (live output) |
| Tmux ecosystem | ❌ | ❌ | ✅ |

## References

- [Grok's recommendation](../tmp/grok-daemon-question.md)
- [Supervisord docs](http://supervisord.org/)
- Configuration: [config/supervisord.conf](../config/supervisord.conf)
