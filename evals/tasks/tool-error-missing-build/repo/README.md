# fxkit

Converts amounts between currencies using the reference rates in
`data/rates.csv`.

## Setup

The rate table is generated, not checked in. Build it once after cloning and
again whenever `data/rates.csv` or the build script changes:

```
npm run build
```

Then run the tests with `npm test`.
