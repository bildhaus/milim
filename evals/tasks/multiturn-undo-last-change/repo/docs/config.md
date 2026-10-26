# Client configuration

`loadConfig(overrides)` merges these defaults with the caller's overrides.

| Setting | Default | Meaning |
|---|---|---|
| `baseUrl` | `https://api.example.test` | Service root. |
| `timeoutMs` | 5000 | Per-attempt timeout in milliseconds. |
| `retries` | 3 | Retries after a failed attempt. |
| `backoffMs` | 200 | First retry delay; doubles on every retry. |
