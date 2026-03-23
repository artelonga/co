# ADR-0001: Cookie-Based Variant Routing

## Status

Accepted

## Context

CO-Web needs to serve different UI variants to different users for A/B/C testing.
Each variant presents a distinct approach to task management (Kanban, Table, Timeline),
and the server must route requests to the correct static files based on the user's
assigned variant.

We considered several routing mechanisms: query parameters, URL path prefixes,
HTTP headers, and cookies.

## Decision

Use a `co_variant` cookie to determine which set of static files to serve.

- The server reads the `co_variant` cookie on each request (values: `a`, `b`, `c`).
- If no cookie is set, the server assigns a random variant and sets the cookie.
- Static files are organized under `static/{variant}/` directories.
- Users can switch variants manually via the experiment widget, which updates the cookie.

## Consequences

- **Simple**: No authentication or session store required.
- **Stateless server**: Variant state lives entirely in the client cookie.
- **User control**: Users can switch variants at any time via the UI.
- **Transparent**: Variant assignment is visible and debuggable in browser dev tools.
- **Limitation**: Cookie-based routing is not tamper-proof, but this is acceptable for internal experimentation.
