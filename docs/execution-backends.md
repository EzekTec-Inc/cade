# Execution Backends

Tools that run shell commands (`bash`, `run_subagent`'s shell access,
file operations) execute through an **ExecutionBackend** abstraction.
Switching backend at runtime sandboxes the agent without changing any
other behaviour.

## Available backends

| Backend | When to use | Feature flag |
|---|---|---|
| `local` | Default — runs on the host filesystem | always on |
| `docker` | Sandbox tools inside a Docker container | `backend-docker` |
| `ssh` | Run tools on a remote machine over SSH | `backend-ssh` |
| `readonly` | Wrap any backend; deny mutations | always on |

Switch live:

```bash
/backend                  # show current
/backend docker           # switch to Docker
/backend ssh              # switch to SSH
/backend local            # back to default
/backend readonly         # wrap current with read-only guard
```

Or pin in `~/.cade/settings.json`:

```json
{
  "execution": {
    "backend": "docker",
    "docker_image": "ubuntu:22.04",
    "docker_flags": ["--network", "host"]
  }
}
```

## Local

Runs commands directly on the host. Path protection still applies (see
[permissions.md](permissions.md)). Plan mode and YOLO behave the same way
they do under any backend.

## Docker

Runs every shell command inside a fresh container.

```json
{
  "execution": {
    "backend": "docker",
    "docker_image": "ubuntu:22.04",
    "docker_flags": ["--rm", "-v", "$PWD:/workspace", "-w", "/workspace"]
  }
}
```

| Setting | Purpose |
|---|---|
| `docker_image` | Image to use (default `ubuntu:22.04`) |
| `docker_flags` | Extra flags appended to `docker run` |

Requires the `backend-docker` feature flag at compile time. Without it,
selecting `docker` falls back to `local` with a tracing warning.

**Caveats:**

- Each tool call is a fresh `docker run` — startup overhead per call
- File edits go to the **host** filesystem (CADE's edit tools never
  travel through the container; only `bash` and other shell tools do)
- Use bind-mounts in `docker_flags` to make the workspace visible

## SSH

Runs every shell command on a remote host over SSH.

```json
{
  "execution": {
    "backend": "ssh",
    "ssh_host": "build.example.com",
    "ssh_user": "ci",
    "ssh_key_path": "~/.ssh/id_ed25519",
    "ssh_port": 22
  }
}
```

| Setting | Default |
|---|---|
| `ssh_host` | (required) |
| `ssh_user` | `$USER` (or `$LOGNAME`, `$USERNAME`, `root`) |
| `ssh_key_path` | None (uses ssh-agent) |
| `ssh_port` | `22` |

Env vars:

| Variable | Effect |
|---|---|
| `CADE_SSH_ACCEPT_NEW` | Auto-accept unknown host keys (`true`/`false`) |

**Caveats:**

- Same as Docker — file edits stay on the host. Only shell tools go
  remote.
- Host-key verification follows your normal `ssh` config.
- For round-tripping files, pair with `rsync` or `scp` invoked through
  the shell tool.

## ReadOnly

Wraps any other backend and refuses every command that has side-effects.
Used internally when permission mode is `plan`. You rarely select it
directly; `/plan` does it for you.

## Programmatic API

```rust
use cade_agent::backends::backend_from_profile;
use cade_core::settings::{ExecutionProfile, ExecutionBackendKind};

let profile = ExecutionProfile {
    backend: ExecutionBackendKind::Docker,
    docker_image: Some("rust:1.78".into()),
    docker_flags: vec!["--memory".into(), "2g".into()],
    ..Default::default()
};

let backend = backend_from_profile(&profile);
backend.run("cargo test", &cwd).await?;
```

Test coverage for backend selection lives in
`crates/cade-agent/src/backends/`.

## Recommended setup

| Scenario | Recommendation |
|---|---|
| Day-to-day local dev | `local` + `default` permission mode |
| Untrusted prompts (auto-runs, scheduled) | `docker` + `acceptEdits` |
| Air-gapped review of someone else's prompt | `local` + `plan` |
| Running on a beefy remote box from a laptop | `ssh` |
| YOLO mode | **always** pair with `docker` or `ssh` to a sacrificial host |
