# CADE Automated Webhook Workflows

CADE supports automated, headless execution workflows triggered by external third-party webhook integrations (e.g., GitHub Actions, GitLab CI/CD, Slack slash commands, or corporate webhooks). 

When a webhook is received at `/v1/workflows/{workflow_name}`, CADE automatically loads a corresponding configuration file on disk, creates a dedicated background agent session, persists the event payload as the initial trigger message, and runs the agent loop completely in the background.

---

## 1. How Webhook Dispatching Works

```
                        POST /v1/workflows/issue_triage
                                    │
                                    ▼
       1. Name Validation (Alphanumeric/Hyphens/Underscores Only)
                                    │
                                    ▼
    2. Load Configuration from .cade/workflows/{workflow_name}.json
                                    │
                                    ▼
  3. Resolve/Create Agent (ID: agent-workflow-{workflow_name})
                                    │
                                    ▼
 4. Create Conversation & Persist initial message with JSON Payload
                                    │
                                    ▼
             5. Create Run Record & Return 202 Accepted
                                    │
                                    ▼
   6. Spawn Asynchronous run_agent_loop in Background Task
```

---

## 2. Setting Up GitHub Actions Integration

To safely trigger CADE from a GitHub Actions workflow (such as on `EzekTec-Inc/cade`), configure the following **Repository Secrets** under `Settings -> Secrets and variables -> Actions`:

* `CADE_SERVER_URL`: The public-facing HTTP/HTTPS base URL of your hosted `cade-server` (e.g., `https://cade.ezektec.com` or your corporate IP).
* `CADE_API_KEY`: The bearer token configured on your CADE server (`CADE_API_KEY` environmental variable).

### Example GitHub Actions Workflow (`.github/workflows/cade-triage.yml`)

Create this file in your repository to automatically dispatch issue payloads to CADE:

```yaml
name: CADE Automated Issue Triage

on:
  issues:
    types: [opened]

jobs:
  triage:
    name: Dispatch Triage Webhook
    runs-on: ubuntu-latest
    steps:
      - name: Send Event Payload to CADE
        run: |
          curl -X POST \
            -H "Authorization: Bearer ${{ secrets.CADE_API_KEY }}" \
            -H "Content-Type: application/json" \
            -d '${{ toJson(github.event) }}' \
            "${{ secrets.CADE_SERVER_URL }}/v1/workflows/issue_triage" \
            --fail \
            --include
```

---

## 3. Workflow Configuration Schema

Each workflow is defined in `.cade/workflows/{workflow_name}.json`. Here is the structured JSON schema:

```json
{
  "name": "issue_triage",
  "agent": "github-triage-agent",
  "model": "openai/gpt-4o",
  "prompt": "You are a specialized GitHub issue triager. Analyze the incoming issue payload, categorize the issue, assign appropriate labels, and recommend a resolution plan."
}
```

### Schema Parameters:
* `name`: Unique identifier for the workflow (must match the filename).
* `agent`: The display name for the newly created or selected agent session.
* `model`: The specific LLM model to route the completions through (e.g., `openai/gpt-4o`, `anthropic/claude-3-5-sonnet-20241022`).
* `prompt`: The custom system prompt representing the core guidelines and guardrails for this headless agent.

---

## 4. Security Hardening & Best Practices

1. **Authentication Security:** Always run your `cade-server` with `CADE_API_KEY` set. This enforces bearer-token authentication on all Rest API surfaces, including the workflow webhook endpoint.
2. **Reverse Proxy & TLS:** Never expose raw HTTP ports to the public internet. Always front `cade-server` with a reverse proxy like **Nginx** or **Caddy** with TLS/HTTPS certificates.
3. **Private Networking:** For highly sensitive codebases, keep `cade-server` completely private and connect your GitHub self-hosted runner or corporate workflows through a secure private mesh network like **Tailscale** or **WireGuard**.
4. **Path Traversal Protection:** CADE automatically performs strict alphanumeric and hyphen/underscore validation on `workflow_name` URL parameters to prevent path-traversal attacks and block arbitrary file read/write hacks.
