# Agent Mail System

This project has multi-agent communication enabled via MCP Agent Mail.

## Commands

All commands use the canonical coordination tools in agentcore/tools/

### Check your agent identity
```bash
agentcore/tools/agent-mail-helper.sh whoami
```

### List all agents
```bash
agentcore/tools/agent-mail-helper.sh list
```

### Send a message
```bash
agentcore/tools/agent-mail-helper.sh send 'RecipientName' 'Subject' 'Message body'
```

### Check inbox
```bash
agentcore/tools/agent-mail-helper.sh inbox
```

### Notifications monitor (tmux banner)
```bash
agentcore/tools/mail-monitor-ctl.sh start
```

## Server check

Agent mail requires the MCP Agent Mail server to be running (port 8765).

Quick check:
```bash
docker ps | grep 8765
```

If it's not running:
```bash
cd "$MCP_AGENT_MAIL_DIR" && docker-compose up -d
```

## Troubleshooting

### Not receiving notifications (but inbox has messages)
1) Check monitor status:
```bash
agentcore/tools/mail-monitor-ctl.sh status
```
2) Restart monitor (binds to current pane):
```bash
agentcore/tools/mail-monitor-ctl.sh restart
```
3) Verify this pane has an agent name:
```bash
cat ./pids/$(tmux display-message -p "#{session_name}:#{window_index}.#{pane_index}" | tr ':.' '-').agent-name
```

### Not receiving messages at all
```bash
agentcore/tools/agent-mail-helper.sh inbox
```

## Hook Bypass Utility

For testing purposes, you can temporarily bypass Claude Code hooks.

### Enable bypass
```bash
./scripts/hook-bypass.sh on
```

### Disable bypass
```bash
./scripts/hook-bypass.sh off
```

### Check status
```bash
./scripts/hook-bypass.sh status
```

When bypass is enabled, a warning indicator will appear in the tmux pane borders and status bar.

## Examples

```bash
# See who you are
agentcore/tools/agent-mail-helper.sh whoami

# See all agents in this project
agentcore/tools/agent-mail-helper.sh list

# Send a message
agentcore/tools/agent-mail-helper.sh send 'CloudyBadger' 'Status' 'Feature X complete'

# Check recent messages
agentcore/tools/agent-mail-helper.sh inbox 5
```
