#!/bin/bash
if [ -z "$GOOGLE_API_KEY" ]; then
    echo "No key"
    exit 1
fi
curl -s "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse&key=$GOOGLE_API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"contents":[{"role":"user","parts":[{"text":"Hello"}]}]}' | head -n 20
