Run this exact Bash command and nothing else:

if [ -f .no-clear ] && grep -q "on" .no-clear 2>/dev/null; then echo "off" > .no-clear; echo "Context clearing: ON (agents reset context between beads)"; else echo "on" > .no-clear; echo "Context clearing: OFF (agents keep context between beads)"; fi