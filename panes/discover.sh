#!/bin/bash
# Fully dynamic pane discovery - no hardcoded registry
# Derives identity from tmux pane index
# Usage: discover.sh [--force] [--quiet] [--pane <pane_id>] [--all]

# Source shared project configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$SCRIPT_DIR/scripts/lib/project-config.sh"

FORCE=false
QUIET=false
SUMMARY=false
TARGET_PANE=""
DISCOVER_ALL=false

while [[ $# -gt 0 ]]; do
  case $1 in
    --force) FORCE=true; shift ;;
    --quiet) QUIET=true; shift ;;
    --summary) SUMMARY=true; shift ;;
    --pane) TARGET_PANE="$2"; shift 2 ;;
    --all) DISCOVER_ALL=true; shift ;;
    *) shift ;;
  esac
done

if [ "$SUMMARY" = true ]; then
  QUIET=true
fi

# Retry wrapper for auto-registration with exponential backoff
# Usage: retry_auto_register <target_pane_id>
retry_auto_register() {
  local target_pane="$1"
  local max_attempts=3
  local retry_delay=3
  local attempt=1

  while [ "$attempt" -le "$max_attempts" ]; do
    if [ "$QUIET" = false ] && [ "$attempt" -gt 1 ]; then
      echo "Retry attempt $attempt/$max_attempts for auto-registration..." >&2
    fi

    # Call auto-register-agent.sh
    if [ -n "$target_pane" ]; then
      TARGET_TMUX_PANE=$(tmux display-message -t "$target_pane" -p "#{pane_id}" 2>/dev/null)
      TMUX_PANE="$TARGET_TMUX_PANE" QUIET=true source "$SCRIPTS_DIR/auto-register-agent.sh"
    else
      QUIET=true source "$SCRIPTS_DIR/auto-register-agent.sh"
    fi

    local status=$?

    # Check if registration succeeded
    if [ "$status" -eq 0 ]; then
      return 0
    fi

    # If this was the last attempt, fail
    if [ "$attempt" -eq "$max_attempts" ]; then
      return 1
    fi

    # Wait before retry
    [ "$QUIET" = false ] && echo "Auto-registration failed, waiting ${retry_delay}s before retry..." >&2
    sleep "$retry_delay"
    attempt=$((attempt + 1))
  done

  return 1
}

# If --all, discover all panes and cleanup stale files
if [ "$DISCOVER_ALL" = true ]; then
  # Discover all panes first
  tmux list-panes -a -F "#{session_name}:#{window_index}.#{pane_index}" | while read pane; do
    ARGS="--pane $pane"
    [ "$FORCE" = true ] && ARGS="$ARGS --force"
    [ "$QUIET" = true ] && ARGS="$ARGS --quiet"
    bash "$0" $ARGS
  done

  # Cleanup stale identity files
  ARCHIVE_DIR="$PROJECT_ROOT/archive/panes"
  mkdir -p "$ARCHIVE_DIR"

  for identity_file in "$PANES_DIR/"*.identity; do
    [ -f "$identity_file" ] || continue

    # Check if this pane still exists in tmux
    pane_id=$(jq -r '.pane // empty' "$identity_file" 2>/dev/null)
    if [ -n "$pane_id" ]; then
      if ! tmux list-panes -a -F "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null | grep -q "^${pane_id}$"; then
        # Pane doesn't exist, move identity file to archive
        filename=$(basename "$identity_file")
        [ "$QUIET" = false ] && echo "Archiving stale identity: $filename (pane $pane_id does not exist)"
        mv "$identity_file" "$ARCHIVE_DIR/"
      fi
    fi
  done

  # Cleanup stale agent-name files
  for agent_name_file in "$PIDS_DIR/"*.agent-name; do
    [ -f "$agent_name_file" ] || continue

    # Extract pane ID from filename (e.g., flywheel-1-6.agent-name -> flywheel:1.6)
    # Replace last hyphen with '.', then new last hyphen with ':' (greedy match)
    filename=$(basename "$agent_name_file" .agent-name)
    pane_id=$(echo "$filename" | sed 's/\(.*\)-/\1./; s/\(.*\)-/\1:/')

    if ! tmux list-panes -a -F "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null | grep -q "^${pane_id}$"; then
      [ "$QUIET" = false ] && echo "Archiving stale agent-name: $(basename $agent_name_file) (pane $pane_id does not exist)"
      mv "$agent_name_file" "$ARCHIVE_DIR/"
    fi
  done

  # Cleanup stale lock files (lock files are temporary, safe to remove all)
  find "$PANES_DIR" -name '*.lock' -type f -exec mv {} "$ARCHIVE_DIR/" \; 2>/dev/null

  exit 0
fi

if [ -n "$TARGET_PANE" ]; then
  MY_PANE="$TARGET_PANE"
  MY_INDEX=$(echo "$MY_PANE" | grep -oE '\.[0-9]+$' | tr -d '.')
  MY_CMD=$(tmux display-message -t "$TARGET_PANE" -p "#{pane_current_command}" 2>/dev/null)
else
  MY_PANE=$(tmux display-message -p "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null)
  MY_INDEX=$(tmux display-message -p "#{pane_index}" 2>/dev/null)
  MY_CMD=$(tmux display-message -p "#{pane_current_command}" 2>/dev/null)
fi

if [ -z "$MY_PANE" ]; then
  [ "$QUIET" = false ] && echo "ERROR: Not in tmux"
  exit 1
fi

# Check if identity already exists (skip unless --force)
SAFE_PANE=$(echo "$MY_PANE" | tr ':.' '-')
IDENTITY_FILE="$PANES_DIR/$SAFE_PANE.identity"

if [ -f "$IDENTITY_FILE" ] && [ "$FORCE" = false ]; then
  # If the pane exists but isn't registered, register it now
  EXISTING_MAIL=$(jq -r '.agent_mail_name // empty' "$IDENTITY_FILE" 2>/dev/null)
  if [ -z "$EXISTING_MAIL" ]; then
    if [ -f "$SCRIPTS_DIR/auto-register-agent.sh" ]; then
      if ! retry_auto_register "$TARGET_PANE"; then
        echo "Warning: auto-register failed for pane $MY_PANE after 3 attempts" >&2
        exit 1
      fi
      if [ "$SUMMARY" = true ]; then
        NEW_MAIL=$(jq -r '.agent_mail_name // empty' "$IDENTITY_FILE" 2>/dev/null)
        if [ -n "$NEW_MAIL" ]; then
          echo "Registered $MY_PANE -> $NEW_MAIL"
        fi
      fi
    fi
  fi
  [ "$QUIET" = false ] && echo "Identity exists: $(cat $IDENTITY_FILE)"

  # Always update tmux variables from identity file
  AGENT_MAIL=$(jq -r ".agent_mail_name // empty" "$IDENTITY_FILE" 2>/dev/null)
  LLM_NAME=$(jq -r ".name // empty" "$IDENTITY_FILE" 2>/dev/null)
  if [ -n "$AGENT_MAIL" ]; then
    if [ -n "$TARGET_PANE" ]; then
      tmux set-option -p -t "$TARGET_PANE" @agent_name "$AGENT_MAIL" 2>/dev/null || true
    else
      tmux set-option -p -t "$MY_PANE" @agent_name "$AGENT_MAIL" 2>/dev/null || true
    fi
  fi
  if [ -n "$LLM_NAME" ]; then
    if [ -n "$TARGET_PANE" ]; then
      tmux set-option -p -t "$TARGET_PANE" @llm_name "$LLM_NAME" 2>/dev/null || true
    else
      tmux set-option -p -t "$MY_PANE" @llm_name "$LLM_NAME" 2>/dev/null || true
    fi
  fi
  exit 0
fi

CREATED_NEW_IDENTITY=false
if [ ! -f "$IDENTITY_FILE" ] || [ "$FORCE" = true ]; then
  CREATED_NEW_IDENTITY=true
fi

# Determine my name from pane index
# Get TTY for better process detection
if [ -n "$TARGET_PANE" ]; then
  MY_TTY=$(tmux display-message -t "$TARGET_PANE" -p "#{pane_tty}" 2>/dev/null)
else
  MY_TTY=$(tmux display-message -p "#{pane_tty}" 2>/dev/null)
fi

# Check if Claude, Aider, or Codex is actually running in this pane
IS_CLAUDE=false
IS_AIDER=false
IS_CODEX=false
if [ -n "$MY_TTY" ] && [ -e "$MY_TTY" ]; then
  PANE_PROCS=$(lsof -t "$MY_TTY" 2>/dev/null | xargs ps -p 2>/dev/null)
  if echo "$PANE_PROCS" | grep -q "claude"; then
    IS_CLAUDE=true
  elif echo "$PANE_PROCS" | grep -q "codex"; then
    IS_CODEX=true
  elif echo "$PANE_PROCS" | grep -q "aider"; then
    IS_AIDER=true
  fi
fi
if [ "$IS_CLAUDE" = true ]; then
  # FIRST: Check if @llm_name already set correctly by startup script
  EXISTING_LLM_NAME=$(tmux show -pv -t "${TARGET_PANE:-$MY_PANE}" @llm_name 2>/dev/null || echo "")

  if [[ "$EXISTING_LLM_NAME" =~ ^(Claude|DeepSeek)\ [0-9]+$ ]]; then
    # Label already set correctly by startup script, preserve it (including DeepSeek via Claude Code)
    MY_NAME="$EXISTING_LLM_NAME"
    MY_TYPE="claude-code"
  # THEN: Try to preserve existing Claude number from identity file
  elif [ -f "$IDENTITY_FILE" ]; then
    EXISTING_NAME=$(jq -r '.name // empty' "$IDENTITY_FILE" 2>/dev/null)
    if [[ "$EXISTING_NAME" =~ ^Claude\ ([0-9]+)$ ]]; then
      MY_NAME="$EXISTING_NAME"
      MY_TYPE="claude-code"
    else
      # No valid existing name, find next available
      # Get current session name for filtering
      CURRENT_SESSION=$(echo "$MY_PANE" | cut -d: -f1)
      MAX_CLAUDE_NUM=0
      for identity_file in "$PANES_DIR/"*.identity; do
        if [ -f "$identity_file" ]; then
          # Only count identity files for panes that actually exist
          pane_id=$(jq -r '.pane // empty' "$identity_file" 2>/dev/null)

          # Only count panes from same session (Fix #2)
          pane_session=$(echo "$pane_id" | cut -d: -f1)
          if [ "$pane_session" != "$CURRENT_SESSION" ]; then
            continue  # Skip cross-session panes
          fi

          if [ -n "$pane_id" ] && tmux list-panes -a -F "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null | grep -q "^${pane_id}$"; then
            existing_name=$(jq -r '.name // empty' "$identity_file" 2>/dev/null)
            if [[ "$existing_name" =~ ^Claude\ ([0-9]+)$ ]]; then
              num="${BASH_REMATCH[1]}"
              if [ "$num" -gt "$MAX_CLAUDE_NUM" ]; then
                MAX_CLAUDE_NUM=$num
              fi
            fi
          fi
        fi
      done
      NEXT_CLAUDE_NUM=$((MAX_CLAUDE_NUM + 1))
      MY_NAME="Claude $NEXT_CLAUDE_NUM"
      MY_TYPE="claude-code"
    fi
  else
    # No identity file exists, find next available number
    # Get current session name for filtering
    CURRENT_SESSION=$(echo "$MY_PANE" | cut -d: -f1)
    MAX_CLAUDE_NUM=0
    for identity_file in "$PANES_DIR/"*.identity; do
      if [ -f "$identity_file" ]; then
        # Only count identity files for panes that actually exist
        pane_id=$(jq -r '.pane // empty' "$identity_file" 2>/dev/null)

        # Only count panes from same session (Fix #2)
        pane_session=$(echo "$pane_id" | cut -d: -f1)
        if [ "$pane_session" != "$CURRENT_SESSION" ]; then
          continue  # Skip cross-session panes
        fi

        if [ -n "$pane_id" ] && tmux list-panes -a -F "#{session_name}:#{window_index}.#{pane_index}" 2>/dev/null | grep -q "^${pane_id}$"; then
          existing_name=$(jq -r '.name // empty' "$identity_file" 2>/dev/null)
          if [[ "$existing_name" =~ ^Claude\ ([0-9]+)$ ]]; then
            num="${BASH_REMATCH[1]}"
            if [ "$num" -gt "$MAX_CLAUDE_NUM" ]; then
              MAX_CLAUDE_NUM=$num
            fi
          fi
        fi
      fi
    done
    NEXT_CLAUDE_NUM=$((MAX_CLAUDE_NUM + 1))
    MY_NAME="Claude $NEXT_CLAUDE_NUM"
    MY_TYPE="claude-code"
  fi
elif [ "$IS_CODEX" = true ]; then
  # Codex CLI agent detected
  # Check if @llm_name already set correctly by startup script
  EXISTING_LLM_NAME=$(tmux show -pv -t "${TARGET_PANE:-$MY_PANE}" @llm_name 2>/dev/null || echo "")

  if [[ "$EXISTING_LLM_NAME" =~ ^Codex\ [0-9]+$ ]]; then
    # Label already set correctly by startup script, preserve it
    MY_NAME="$EXISTING_LLM_NAME"
    MY_TYPE="codex"
  else
    # Default to Codex naming
    MY_NAME="Codex $MY_INDEX"
    MY_TYPE="codex"
  fi
elif [ "$IS_AIDER" = true ]; then
  # Aider agent detected
  # Check if @llm_name already set correctly by startup script
  EXISTING_LLM_NAME=$(tmux show -pv -t "${TARGET_PANE:-$MY_PANE}" @llm_name 2>/dev/null || echo "")

  if [[ "$EXISTING_LLM_NAME" =~ ^Codex\ [0-9]+$ ]]; then
    # Label already set correctly by startup script, preserve it
    MY_NAME="$EXISTING_LLM_NAME"
    MY_TYPE="aider"
  else
    # Default to Codex naming for aider
    MY_NAME="Codex $MY_INDEX"
    MY_TYPE="aider"
  fi
else
  # Check if @llm_name already set by startup script
  EXISTING_LLM_NAME=$(tmux show -pv -t "${TARGET_PANE:-$MY_PANE}" @llm_name 2>/dev/null || echo "")

  if [[ "$EXISTING_LLM_NAME" =~ ^(Claude|DeepSeek|Codex)\ [0-9]+$ ]]; then
    # Label already set correctly, preserve it
    MY_NAME="$EXISTING_LLM_NAME"
    if [[ "$EXISTING_LLM_NAME" =~ ^(Claude|DeepSeek) ]]; then
      MY_TYPE="claude-code"
    else
      MY_TYPE="codex"
    fi
  else
    # Not set or invalid, default to Terminal
    MY_NAME="Terminal $MY_INDEX"
    MY_TYPE="terminal"
  fi
fi

if [ "$QUIET" = false ]; then
  echo "=== MY IDENTITY ==="
  echo "Pane: $MY_PANE"
  echo "Name: $MY_NAME"
  echo "Type: $MY_TYPE"
  echo ""
  echo "=== ALL PANES ==="
  tmux list-panes -a -F "#{session_name}:#{window_index}.#{pane_index} | #{pane_current_command}" | while read line; do
    pane=$(echo "$line" | cut -d'|' -f1 | xargs)
    cmd=$(echo "$line" | cut -d'|' -f2 | xargs)
    idx=$(echo "$pane" | grep -oE '\.[0-9]+$' | tr -d '.')

    if [[ "$cmd" == "claude" ]]; then
      name="Claude $idx"
    else
      name="Terminal $idx"
    fi

    if [[ "$pane" == "$MY_PANE" ]]; then
      echo "  $pane -> $name ($cmd) <- YOU"
    else
      echo "  $pane -> $name ($cmd)"
    fi
  done
fi

# Store identity (preserve existing fields like agent_mail_name)
NEW_IDENTITY="{\"pane\":\"$MY_PANE\",\"name\":\"$MY_NAME\",\"type\":\"$MY_TYPE\"}"

# Check for agent_mail_name in existing identity file
if [ -f "$IDENTITY_FILE" ]; then
  EXISTING_MAIL=$(jq -r '.agent_mail_name // empty' "$IDENTITY_FILE" 2>/dev/null)
  if [ -n "$EXISTING_MAIL" ]; then
    NEW_IDENTITY=$(echo "$NEW_IDENTITY" | jq --arg mail "$EXISTING_MAIL" '. + {agent_mail_name: $mail}')
  fi
fi

# Also check for agent name in pids/<pane-id>.agent-name file
AGENT_NAME_FILE="$PIDS_DIR/${SAFE_PANE}.agent-name"
if [ -f "$AGENT_NAME_FILE" ]; then
  AGENT_NAME=$(cat "$AGENT_NAME_FILE" 2>/dev/null | tr -d '\n')
  if [ -n "$AGENT_NAME" ]; then
    NEW_IDENTITY=$(echo "$NEW_IDENTITY" | jq --arg mail "$AGENT_NAME" '. + {agent_mail_name: $mail}')
  fi
fi

# Store LLM name in tmux variable
if [ -n "$TARGET_PANE" ]; then
  tmux set-option -p -t "$TARGET_PANE" @llm_name "$MY_NAME" 2>/dev/null || true
else
  tmux set-option -p -t "$MY_PANE" @llm_name "$MY_NAME" 2>/dev/null || true
fi

# Write with flock protection to prevent race conditions
LOCK_FILE="$PANES_DIR/.${SAFE_PANE}.lock"
TEMP_FILE="${IDENTITY_FILE}.tmp.$$"

# Use flock if available (prefer Homebrew version on macOS)
FLOCK_CMD="flock"
if [ ! -x "$(command -v flock)" ] && [ -x "/opt/homebrew/opt/util-linux/bin/flock" ]; then
  FLOCK_CMD="/opt/homebrew/opt/util-linux/bin/flock"
fi

{
  # Acquire exclusive lock (wait max 2 seconds)
  $FLOCK_CMD -x -w 2 200 || exit 1

  # Write to temp file then atomically rename
  echo "$NEW_IDENTITY" > "$TEMP_FILE"
  mv "$TEMP_FILE" "$IDENTITY_FILE"

} 200>"$LOCK_FILE"

# Auto-register mail name for this pane if missing (all pane types, not just Claude)
CURRENT_MAIL=$(jq -r '.agent_mail_name // empty' "$IDENTITY_FILE" 2>/dev/null)
if [ -z "$CURRENT_MAIL" ]; then
  if [ -f "$SCRIPTS_DIR/auto-register-agent.sh" ]; then
    if ! retry_auto_register "$TARGET_PANE"; then
      echo "Warning: auto-register failed for pane $MY_PANE after 3 attempts" >&2
      exit 1
    fi
  fi
fi

[ "$QUIET" = false ] && echo "" && echo "Identity saved: $IDENTITY_FILE"

# Update tmux pane title with agent mail name if available
AGENT_MAIL=$(jq -r '.agent_mail_name // empty' "$IDENTITY_FILE" 2>/dev/null)
LLM_NAME=$(jq -r '.name // empty' "$IDENTITY_FILE" 2>/dev/null)
PANE_TYPE=$(jq -r '.type // "unknown"' "$IDENTITY_FILE" 2>/dev/null)

# Summary output for newly created identities
if [ "$SUMMARY" = true ] && [ "$CREATED_NEW_IDENTITY" = true ] && [ -n "$AGENT_MAIL" ]; then
  echo "Registered $MY_PANE -> $AGENT_MAIL"
fi

# Set agent name variable
if [ -n "$AGENT_MAIL" ]; then
  if [ -n "$TARGET_PANE" ]; then
    tmux set-option -p -t "$TARGET_PANE" @agent_name "$AGENT_MAIL" 2>/dev/null || true
  else
    tmux set-option -p -t "$MY_PANE" @agent_name "$AGENT_MAIL" 2>/dev/null || true
  fi
fi

# Set LLM name variable
if [ -n "$LLM_NAME" ]; then
  if [ -n "$TARGET_PANE" ]; then
    tmux set-option -p -t "$TARGET_PANE" @llm_name "$LLM_NAME" 2>/dev/null || true
  else
    tmux set-option -p -t "$MY_PANE" @llm_name "$LLM_NAME" 2>/dev/null || true
  fi
fi
