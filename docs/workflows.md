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

* `CADE_SERVER_URL`: The public-facing HTTP/HTTPS base URL of your hosted `cade-server` (e.g., `https://cade.ezektec.com` or your corporate IP). This server must be publicly reachable from GitHub's runner IP ranges or accessible via a private tunnel.
* `CADE_API_KEY`: The bearer token configured on your CADE server (`CADE_API_KEY` environmental variable).

### 2.1 Prerequisites & Verification Checks

Before attempting to trigger a workflow, verify that your environment, secrets, and CLI are correctly set up:

1. **Verify GitHub CLI Auth & Scopes:**
   Ensure your local GitHub CLI is authenticated and has the required `workflow` scope:
   ```bash
   gh auth status
   ```
   If the `workflow` scope is missing, re-authenticate:
   ```bash
   gh auth refresh -s workflow
   ```

2. **Verify Repository Secrets:**
   Ensure the required secrets exist on your repository (it should return `CADE_SERVER_URL` and `CADE_API_KEY`):
   ```bash
   gh secret list --repo EzekTec-Inc/cade
   ```

3. **Check Workflow Config Existence:**
   The CADE server will reject any webhook if the corresponding configuration does not exist in `.cade/workflows/{workflow_name}.json` on the server's local file system.

### 2.2 Example GitHub Actions Workflow (`.github/workflows/cade-triage.yml`)

Create this file in your repository to automatically dispatch issue payloads to CADE on issue creation, or manually via `workflow_dispatch` with a custom JSON payload:

```yaml
name: CADE Automated Issue Triage

on:
  issues:
    types: [opened]
  workflow_dispatch:
    inputs:
      custom_payload:
        description: 'Manual JSON payload for CADE workflow'
        required: false
        default: '{"trigger_type": "manual", "issue_number": 42}'

jobs:
  triage:
    name: Dispatch Triage Webhook
    runs-on: ubuntu-latest
    steps:
      - name: Formulate Workflow Payload
        id: payload
        run: |
          if [ "${{ github.event_name }}" = "issues" ]; then
            JSON_BODY=$(cat <<EOF
          {
            "repository": "${{ github.repository }}",
            "issue_number": ${{ github.event.issue.number }},
            "issue_title": $(echo "${{ github.event.issue.title }}" | jq -R .),
            "sender": "${{ github.event.sender.login }}",
            "trigger_type": "github_issue_opened"
          }
          EOF
          )
          else
            JSON_BODY='${{ github.event.inputs.custom_payload }}'
          fi
          
          # Write safely to outputs
          echo "body<<EOF" >> $GITHUB_OUTPUT
          echo "$JSON_BODY" >> $GITHUB_OUTPUT
          echo "EOF" >> $GITHUB_OUTPUT
        shell: bash

      - name: Send Event Payload to CADE
        run: |
          curl -X POST \
            -H "Authorization: Bearer ${{ secrets.CADE_API_KEY }}" \
            -H "Content-Type: application/json" \
            -d '${{ steps.payload.outputs.body }}' \
            "${{ secrets.CADE_SERVER_URL }}/v1/workflows/issue_triage" \
            --fail \
            --include
```

### 2.3 Triggering and Monitoring Workflows

Once the workflow is committed to your repository's default branch, you can trigger and observe it using the following commands:

* **List available workflows:**
  ```bash
  gh workflow list --repo EzekTec-Inc/cade
  ```

* **Manually trigger the dispatch workflow:**
  ```bash
  gh workflow run "CADE Automated Issue Triage" \
    --repo EzekTec-Inc/cade \
    -f custom_payload='{"trigger_type": "manual", "issue_number": 99}'
  ```

* **Monitor active workflow run logs:**
  ```bash
  gh run list --workflow="CADE Automated Issue Triage" --repo EzekTec-Inc/cade
  gh run watch <run_id> --repo EzekTec-Inc/cade
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
