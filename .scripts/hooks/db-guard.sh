#!/bin/bash
# Hook to prevent destructive commands during tool use.
# Reads JSON payload from stdin.

PAYLOAD=$(cat)
COMMAND=$(echo "$PAYLOAD" | jq -r '.tool_input.command // empty')

if echo "$COMMAND" | grep -qiE "DROP TABLE|DELETE FROM.*(?!WHERE)|TRUNCATE"; then
    echo "Blocked destructive database command for safety." >&2
    exit 2 # Exit 2 tells CADE to block the tool and return the stderr to the LLM
fi

exit 0 # Proceed normally