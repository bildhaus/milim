# Projects API

Request handlers live in `src/handlers/`, one per file, and `src/router.js`
maps routes to them. `src/db.js` is an in-memory store with a callback API
and the same operations as promises under `db.promises`.

Handlers currently use Node-style callbacks: `handler(req, callback)` calls
`callback(error, { status, body })`. `npm test` runs the route tests.
