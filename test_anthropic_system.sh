curl -s https://api.anthropic.com/v1/messages \
     -H "x-api-key: $ANTHROPIC_API_KEY" \
     -H "anthropic-version: 2023-06-01" \
     -H "content-type: application/json" \
     -d '{
       "model": "claude-3-5-haiku-latest",
       "max_tokens": 100,
       "system": [
         { "type": "text", "text": "Rule 1: Always say BEEP BOOP before answering." },
         { "type": "text", "text": "Rule 2: Keep it short." }
       ],
       "messages": [
         {"role": "user", "content": "How are you?"}
       ]
     }'
