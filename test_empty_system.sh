curl -s https://api.anthropic.com/v1/messages \
     -H "x-api-key: $ANTHROPIC_API_KEY" \
     -H "anthropic-version: 2023-06-01" \
     -H "content-type: application/json" \
     -d '{
       "model": "claude-3-haiku-20240307",
       "max_tokens": 10,
       "system": [
         { "type": "text", "text": "You are a helpful assistant.", "cache_control": {"type": "ephemeral"} },
         { "type": "text", "text": "" }
       ],
       "messages": [
         {"role": "user", "content": "Hello"}
       ]
     }'
