# Learning Record 0003: Relationship Semantics & Protocols

- **Date**: 2026-09-08
- **Topic**: Relationship Directionality & Protocol Specification
- **Status**: Verified

## Key Insight
- Mastered relationship best practice: the arrow starts at the **initiating dependency** (caller) and points to the provider/callee (`paymentProcessingService -> fxRatesApi`).
- Mastered the dual-annotation format: an active verb phrase for business intent (`"Fetches foreign exchange rates from"`) plus a concrete transport protocol (`"HTTPS / REST"`).
- Successfully avoided common anti-patterns: reverse data-return arrows, vague descriptions (`"uses"`), and unstyled bidirectional links.

## Zone of Proximal Development
- Move to **Milestone 3: The System Landscape View**, focusing on:
  1. `enterprise { ... }` boundaries (separating internal systems from external vendors).
  2. `group { ... }` logical domains (e.g. Core Banking, Analytics, Edge Gateways).
  3. The `systemLandscape` view block with `autoLayout` routing.
