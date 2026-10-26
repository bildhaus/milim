# Visits dashboard

`npm run dev` starts the development server on a free local port and prints
its URL once it is ready (about a second after launch).

| Route | Response |
|---|---|
| `GET /health` | `{ ok, boot_id, uptime_ms }`; `boot_id` is new on every start. |
| `GET /counter` | Records a visit and returns `{ boot_id, value }`, the visit count so far. |

The counter lives in memory: restarting the server loses it.
