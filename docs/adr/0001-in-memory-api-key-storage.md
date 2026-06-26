# ADR 1: In-Memory Client API Key Storage (No Local Storage)

* **Status**: Accepted
* **Decided on**: 2026-06-25

## Context

The `cade-gui` client communicates with the CADE backend server using a Bearer token API key. Because this application runs in WebAssembly (WASM) in the user's browser, we need a mechanism to store and access this key across components.

Standard web applications often persist session tokens or API keys to `localStorage` or `IndexedDB` to enable automated login on page refreshes or new sessions. However, client-side disk storage is vulnerable to Cross-Site Scripting (XSS) token-extraction attacks. If a malicious script runs in the browser, it can easily query `localStorage` and exfiltrate the keys.

## Decision

We decided to keep the active API key entirely **in-memory** within a Dioxus `Signal<String>`, and explicitly avoid persisting it to any form of local browser disk storage (`localStorage`, `IndexedDB`, or `sessionStorage`).

The user is required to authenticate on each new browser session. During the active session, the key is passed reactively via global Dioxus Context providers.

## Consequences

### Positive (Pros)
* **High Security Margin**: API keys are completely immune to disk-scraping XSS attacks. If an XSS exploit occurs, there is no persistent storage file to easily query.
* **Ephemeral Lifetime**: Wiping browser state, refreshing the tab, or closing the browser session instantly eradicates the token from memory.

### Negative (Cons)
* **Slight UX Friction**: Users must re-authenticate (enter the API key) on page refreshes or when opening the application in a new browser window.
