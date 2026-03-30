---
name: Deployment Verification
description: Steps to verify the build before deployment
triggers: [deploy, release, build, handoff, verify]
---

# Pre-Deployment Verification (Handoff Verify)
Before approving a deployment or completing a feature, you must run:
1. `cargo check`
2. `cargo test --all-features`
3. `cargo clippy -- -D warnings`

If any step fails, halt the deployment process, return the exact errors, and wait for them to be fixed.