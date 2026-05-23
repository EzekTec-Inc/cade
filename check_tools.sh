#!/bin/bash
export CADE_API_KEY="test_key"
export CADE_DB_PATH=".cade/cade.db"
cargo run --bin cade-server -- --port 8301 > server.log 2>&1 &
SERVER_PID=$!

sleep 5

echo "Fetching agents..."
AGENT_ID=$(curl -s -H "Authorization: Bearer test_key" http://localhost:8301/v1/agents | jq -r '.[0].id')
echo "Agent ID: $AGENT_ID"

echo "Fetching tools..."
curl -s -H "Authorization: Bearer test_key" http://localhost:8301/v1/agents/$AGENT_ID/tools | jq -r '.[].name'

kill $SERVER_PID
wait $SERVER_PID 2>/dev/null
