---
title: Problem details
description: Stable anchors for RFC 9457 problem type URIs returned by the gateway.
---

Asterel gateway errors use [RFC 9457 Problem Details](https://www.rfc-editor.org/rfc/rfc9457.html).
The `type` field points to this page with a fragment matching the machine-readable
`code` field, for example `#invalid_request`.

The code-specific fragments are stable identifiers. During pre-release, detailed
per-code prose may lag behind the runtime; prefer the response `title`, `detail`,
HTTP status, and source code for exact behavior.

## Common codes

### invalid_request

The request shape, parameters, or payload could not be accepted.

### unauthorized

The request lacks valid authentication.

### forbidden

The caller is authenticated but not allowed to perform the requested action.

### not_found

The requested resource was not found in the caller's scope.

### conflict

The request conflicts with current runtime state.

### rate_limited

The caller exceeded a rate limit or replay protection window.

### internal_error

The runtime failed unexpectedly.
