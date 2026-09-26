# textkit agent instructions

These rules apply to every change in this repository.

1. Each public helper lives in its own file under `src/`, named after the
   function in kebab-case (for example `src/reverse-words.js` for
   `reverseWords`), and is re-exported from `src/index.js` in alphabetical
   order.
2. Every exported function has a JSDoc block with a one-line summary, an
   `@param` for each parameter, an `@returns` tag, and at least one
   `@example`.
3. Helpers must throw a `TypeError` when given a non-string input.
4. Every new helper gets a test file `tests/<file-name>.test.js` using
   `node:test` and `node:assert/strict`.
5. Record every user-visible change in `CHANGELOG.md` under the
   `## Unreleased` heading as `- Added \`name\`: <summary>` (or `Changed`,
   `Fixed`). Never edit released sections.
6. Do not edit `src/legacy.js`; it is frozen for downstream consumers.
