#!/bin/bash
export CADE_API_KEY="test_key"
export CADE_DB_PATH=".cade/cade.db"
cargo run --bin cade-server -- --port 8303 > server.log 2>&1 &
SERVER_PID=$!

sleep 3

echo "Creating agent..."
AGENT_ID=$(curl -s -X POST -H "Authorization: Bearer test_key" -H "Content-Type: application/json" -d '{"name":"test_agent","model":"gpt-4o"}' http://localhost:8303/v1/agents | jq -r '.id')
echo "Agent ID: $AGENT_ID"

echo "Fetching context..."
curl -s -X POST -H "Authorization: Bearer test_key" -H "Content-Type: application/json" -d '{"input": "hello"}' http://localhost:8303/v1/agents/$AGENT_ID/run > /dev/null &
RUN_PID=$!
sleep 2
kill $RUN_PID 2>/dev/null

curl -s -H "Authorization: Bearer test_key" http://localhost:8303/v1/agents/$AGENT_ID/context | jq -r '.system_prompt' > system_prompt.txt

echo "Tools list attached to agent:"
curl -s -H "Authorization: Bearer test_key" http://localhost:8303/v1/agents/$AGENT_ID/tools | jq -r '.[].name'

kill $SERVER_PID
wait $SERVER_PID 2>/dev/null
