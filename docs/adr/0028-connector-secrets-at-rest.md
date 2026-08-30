# 0028. Connector secrets are encrypted at the storage boundary

- Status: accepted
- Date: 2026-08-30

> **Numbering.** This work was requested as ADR 0020, but 0020 is already the
> accepted connector-capability model. This record takes the next free number
> rather than overwriting architectural history.

## Context

Connector configuration is durable SQLite data. Some connectors need tokens or
passwords, but the same config object also contains ordinary values that edit
forms must read back. Encrypting the whole object would prevent useful partial
redaction and field-level edit behavior; returning it unchanged would expose
credentials to every client and anyone with database access.

Loom already persists its generated JWT signing secret in the `server_config`
key-value table. ADR 0004 requires a working zero-configuration first start, so
connector encryption needs the same generate-once persistence behavior rather
than a new required environment variable or a parallel secret file.

## Decision

A connector marks a top-level string property in its JSON Schema with:

```json
{ "type": "string", "x-loom-sensitive": true }
```

The backend validates submitted plaintext through the connector factory, then
encrypts each present marked value at the database boundary with AES-256-GCM.
Every value gets a fresh random 96-bit nonce. The stored string is standard
base64 over `nonce || ciphertext || 128-bit authentication tag`.

The 256-bit master key is generated independently from the OS CSPRNG and stored
under its own `server_config` key using the same insert-if-absent pattern as the
JWT signing secret. It is never derived from, shared with, or allowed to fall
back to the JWT secret. Startup logs only whether the key was generated or
loaded.

Runtime construction always decrypts marked values into a temporary plaintext
copy before invoking a connector factory. Plaintext is neither written back to
the database nor serialized into an API response.

Instance list and detail responses omit every marked property from `config` and
include `sensitiveFieldsSet`, an array of marked keys that have a stored value.
An edit client renders those fields empty. Omitting an already-set sensitive
key from `PATCH` preserves its existing ciphertext byte-for-byte; including a
new string encrypts and replaces it. Non-sensitive fields retain their existing
replace-all semantics. An empty string is a submitted replacement, not the
preserve signal.

## Consequences

Database copies and ordinary API reads no longer reveal connector credentials.
GCM authentication makes malformed, corrupted, or tampered blobs fail cleanly
instead of producing plausible plaintext. Losing the persisted master key makes
existing sensitive values unrecoverable, so `server_config` must be backed up
with the connector rows it protects.

Key rotation is deliberately not implemented. Rotating this master key requires
decrypting and re-encrypting every stored sensitive field as one coordinated
operation. That is future work; replacing the `server_config` value alone is
not a supported rotation procedure.
