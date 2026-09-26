# Agent instructions

- `routes.json` is the source of truth for HTTP routes. `src/routes.generated.js`
  is produced from it by `node scripts/gen-routes.mjs`. Never edit the
  generated file by hand; change `routes.json`, then run the generator.
- Each route's handler lives in `src/handlers/<name>.js` and exports one
  function named `handle<Name>` (for example `handleListItems` in
  `src/handlers/list-items.js`). Handlers are pure: they take a request
  object and return `{ status, body }`.
- Read configuration such as the version through `src/meta.js`; do not import
  `package.json` from handlers.
- Keep `routes.json` sorted by `path`, then `method`.
- Run `npm test` before finishing.
