# inventory-service

Stock statistics for the ops dashboard, computed from `data/inventory.json`
and the confirmed shipments in `data/incoming.json`.

```
npm start            # prints "inventory service listening on http://127.0.0.1:<port>"
PORT=8123 npm start  # listen on a fixed port instead of a free one
```

The service needs a moment to build its index; it accepts requests once it
has printed its URL. Stop it with Ctrl-C (SIGINT) or SIGTERM.

## Endpoints

| Request | Response |
|---|---|
| `GET /health` | `{ "ok": true }` |
| `GET /stats/low-stock?warehouse=<name>` | `{ "warehouse", "count", "skus" }`: items whose stock plus confirmed incoming shipments is below their reorder point |

Every response also carries a unique `request_id`, which the service records
in `var/requests.jsonl`.
