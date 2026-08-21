# CADE TypeScript / Node.js SDK (`@ezektec/cade`)

Official TypeScript and Node.js client library for CADE. Embed autonomous coding agents, multi-agent squads, and persistent memory in JavaScript/TypeScript applications.

## Installation

```bash
npm install @ezektec/cade
```

## Quickstart

```typescript
import { AgentSession, TeamSession } from "@ezektec/cade";

const session = new AgentSession({
  serverUrl: "http://localhost:8284",
  model: "anthropic/claude-sonnet-4-5",
});

await session.setMemory("project_rule", "Strict TDD");
const response = await session.prompt("Inspect repository structure.");
console.log("Assistant:", response);
```
