Run this exact Bash command and nothing else:

if [ -f .no-exit ] && grep -q "on" .no-exit 2>/dev/null; then echo "off" > .no-exit; echo "Agent auto-exit: ON (agents exit after one bead)"; else echo "on" > .no-exit; echo "Agent auto-exit: OFF (agents run in REPL loop)"; fi
