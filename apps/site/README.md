# milim site

Static Vite/React site for `milim.ai`, deployed with Cloudflare Pages.

The build imports `docs/wiki/*.md`, emits route-specific docs metadata under an internal static namespace, and generates the Pages Worker that serves those files on `docs.milim.ai`. A shared branded `404.html` keeps missing routes and assets on both hosts as real `404` responses. The landing page and docs share one Pages project and one reduced-motion-aware Lenis smooth-scroll setup; do not edit generated `dist` files.

## Cloudflare Pages

- Project name: `milim-site`
- Production branch: `main`
- Custom domains: `milim.ai`, `www.milim.ai`, `docs.milim.ai`

Deploys are owned by GitHub Actions with Wrangler direct upload. Create a GitHub environment named `production` and add these environment secrets:

- `CLOUDFLARE_ACCOUNT_ID`
- `CLOUDFLARE_API_TOKEN`

The Cloudflare token only needs `Account > Cloudflare Pages > Edit`.

Create the Pages project once:

```powershell
$env:CLOUDFLARE_ACCOUNT_ID="..."
$env:CLOUDFLARE_API_TOKEN="..."
pnpm exec wrangler pages project create milim-site --production-branch main
```

Then run the `Site` workflow from GitHub Actions or push to `main`. Attach the custom domains in Workers & Pages > `milim-site` > Custom domains. The domain is already in Cloudflare, so Cloudflare should create the DNS records during setup.

## Local check

```powershell
pnpm install --frozen-lockfile
pnpm build
pnpm exec wrangler pages dev dist
```
