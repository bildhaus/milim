# Ledger

TypeScript sources in `src/` run directly on Node (built-in type stripping),
so they use only erasable syntax and import each other with `.ts`
extensions.

- `npm run typecheck` runs `tsc --noEmit` over `src/`.
- `npm test` runs `test/*.test.ts` with `node --test`.
