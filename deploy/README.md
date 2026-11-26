# Deploying Tiffin on the web

Tiffin on the web consists of two small pieces with zero databases and zero user accounts:

1. **Static site (`desktop/dist-web`)** — the Svelte UI, WebAssembly engine (`tiffin.wasm`), offline PWA service worker (`sw.js`), standalone privacy page (`/privacy/`), and Cloudflare `_headers`.
2. **`tiffin-helper` (`helper/`)** — one stateless Rust binary that proxies recipe pages, recipe photos, and Open Food Facts name searches while enforcing public-web-only SSRF rules.

Everything runs offline on the visitor's device once loaded. The static site **must** be served over HTTPS with `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp` so OPFS saves atomically via `SharedArrayBuffer` (plain GitHub Pages cannot set custom response headers).

---

## 1. Build from a release snapshot

Always build production artifacts from a **code-only release snapshot** (the same exclusions used for desktop releases: no `.planning/`, `.claude/`, internal design docs, and the redistributable MET stand-in):

```sh
# 1. Verify only the redistributable MET stand-in is present (fails on the dev table)
python3 -I app/tools/build_mets.py --check-release app/data/mets.tsv

# 2. Build the size-optimized WebAssembly reactor
rustup target add wasm32-wasip1
cargo build --release --locked --manifest-path desktop/web-wasm/Cargo.toml --target wasm32-wasip1

# 3. Build the static site pointing at your helper origin and run all checks
cd desktop
npm ci
VITE_HELPER_URL=https://helper.tiffin.fyi npm run build:web
npm run check
npm run check:ui
npm run check:web
```

`desktop/dist-web/` now contains the complete static site plus `desktop/dist-web/_headers` tailored to `VITE_HELPER_URL`.

---

## 2. Option A — First host: Cloudflare Workers Static Assets + container host for `tiffin-helper`

### Static site on Cloudflare Workers Static Assets

Because `tiffin.wasm` is compiled with `rustup` + `wasm32-wasip1`, build `desktop/dist-web` in CI or locally as in Step 1 and deploy with `deploy/wrangler.json`:

- **Automatic deploy on every release tag (recommended):** the `web-bundle` job in `.github/workflows/release.yml` automatically deploys `desktop/dist-web` to the `tiffin` Worker via `deploy/wrangler.json` once `CLOUDFLARE_API_TOKEN` (scoped to Account → **Workers Scripts: Edit**) and `CLOUDFLARE_ACCOUNT_ID` are set as repository secrets, and only after the MET stand-in check, release WASM build, `npm run check`, `npm run build:web`, `npm run check:web`, and Caddy validation all pass.
- **Manual one-command deploy from a clean release snapshot:**

```sh
npx wrangler login   # once
npx wrangler deploy --config deploy/wrangler.json
```

- **Headers:** Cloudflare Workers Static Assets automatically reads `desktop/dist-web/_headers`, applying COOP, COEP, CORP, strict CSP (with an even stricter zero-JS CSP on `/privacy` and `/privacy/*`), `Permissions-Policy`, HSTS, and immutable caching on `/assets/*`, while blocking `/_headers` with `404`.
- **Custom domain:** attach `tiffin.fyi` to the `tiffin` Worker in the Cloudflare dashboard (`deploy/wrangler.json` intentionally omits `routes` so dashboard-managed custom domains stay intact, and disables `workers_dev` and `preview_urls`).

### `tiffin-helper` container (Fly.io or any container host)

A ready-to-use Fly.io config lives at `deploy/fly.toml`. Always deploy from a **clean release snapshot** (e.g. a clone of `iamaharsh/Tiffin` `main`, which carries the redistributable MET stand-in):

```sh
fly apps create tiffin-helper   # pick a unique name if taken, and match it in deploy/fly.toml
fly ips allocate-v4 --shared --config deploy/fly.toml   # free shared IPv4 (omit --shared only if you want a paid dedicated IPv4)
fly ips allocate-v6 --config deploy/fly.toml            # free IPv6
fly deploy . --config deploy/fly.toml --ha=false
fly certs add helper.tiffin.fyi --config deploy/fly.toml
```

Use `--ha=false` so one Machine holds the in-memory rate-limit counters and Open Food Facts backoff window (and keeps idle cost near zero). Apps created with `fly apps create` (instead of `fly launch`) need the two `fly ips allocate-*` lines so `<app>.fly.dev` resolves publicly. Then add the `CNAME` / `A` / `AAAA`, ACME DNS Challenge `CNAME`, and Ownership `TXT` records Fly prints for `helper.tiffin.fyi` (view anytime with `fly certs check helper.tiffin.fyi --config deploy/fly.toml`) in Cloudflare DNS with **Proxy status = DNS only (grey cloud)** so Fly Proxy terminates TLS and populates `Fly-Client-IP` with the real visitor IP (`TIFFIN_CLIENT_IP_HEADER=fly-client-ip`). Note that Fly Proxy does not log normal HTTP requests, though upstream/proxy error logs (e.g., 502/cold-start failures) can briefly record request URLs in Fly's short-term log buffer; moving to the self-hosted Caddy stack below eliminates edge error logging completely.

On another container host behind a reverse proxy, run `helper/Dockerfile` with:

- `TIFFIN_ORIGIN=https://tiffin.fyi`
- `TIFFIN_BEHIND_PROXY=1` (plus `TIFFIN_CLIENT_IP_HEADER=...` if your proxy puts the visitor IP in a dedicated header instead of the last `X-Forwarded-For` entry)
- **Access and HTTP error logging disabled** (or query strings stripped) at the reverse proxy so `?url=`, `?q=`, and `&email=` are never written to disk logs.

---

## 3. Option B — Self-host both on your own server (Caddy + Docker Compose)

Point DNS A/AAAA records for `tiffin.fyi` and `helper.tiffin.fyi` directly at your server, build `desktop/dist-web` as in Step 1, and start both services:

```sh
docker compose -f deploy/docker-compose.yml up -d --build
```

To use custom domains, set `TIFFIN_DOMAIN` and `TIFFIN_HELPER_DOMAIN` as **bare hostnames with no `https://` scheme** (matching the `VITE_HELPER_URL` used when building `desktop/dist-web`):

```sh
TIFFIN_DOMAIN=recipes.example.com TIFFIN_HELPER_DOMAIN=helper.example.com \
  docker compose -f deploy/docker-compose.yml up -d --build
```

What this setup guarantees:
- **Automatic HTTPS** via Caddy on ports 80/443.
- **No request URIs or visitor IPs logged to disk:** both site blocks discard access logs (`log { output discard }`), and the global logger excludes `http.log.error` so proxy errors never log `?url=`, `?q=`, or `&email=`.
- **Hardened containers:** both `caddy` and `tiffin-helper` run with `read_only: true`, `cap_drop: [ALL]` (`caddy` adds only `NET_BIND_SERVICE`), `no-new-privileges:true`, and memory/PID limits. `tiffin-helper` runs as non-root `tiffin` on the internal Docker network only (no public host port).
- **`/_headers` returns 404** (and is hidden from `file_server`) when `dist-web` is mounted directly into Caddy.
- **Note if using a CDN/proxy in front of Caddy:** Caddy replaces untrusted incoming `X-Forwarded-For` headers with the direct peer IP by default. If you place Cloudflare or another reverse proxy in front of Caddy, configure Caddy's `trusted_proxies` for that proxy's IP ranges; otherwise all visitors will share one rate-limit bucket on `tiffin-helper`.

---

## 4. Post-launch verification

1. **Headers check:** verify `curl -I https://tiffin.fyi/` returns `Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Embedder-Policy: require-corp`, `Content-Security-Policy`, `X-Frame-Options: DENY`, `X-Content-Type-Options: nosniff`, and `Referrer-Policy: no-referrer`.
2. **Privacy page check:** verify `curl -I https://tiffin.fyi/privacy/` returns `200` (no redirect) with `Content-Security-Policy: default-src 'none'; ...` and `Permissions-Policy: camera=(), ...`.
3. **Immutable assets check:** verify `curl -I` on a file under `https://tiffin.fyi/assets/` returns `Cache-Control: public, max-age=31536000, immutable`.
4. **Browser check:** open `https://tiffin.fyi/`, confirm `window.crossOriginIsolated === true`, zero CSP/console errors, offline reload works after service worker activation, and Settings → About opens `/privacy/`.
5. **Helper check:** confirm `https://helper.tiffin.fyi/page?url=https://example.com/` returns `403` without `Origin: https://tiffin.fyi`, and recipe import + Open Food Facts search work from `https://tiffin.fyi/`.
6. **HSTS:** both `_headers` and `deploy/Caddyfile` set `Strict-Transport-Security: max-age=31536000; includeSubDomains` (for a brand-new unverified domain, test TLS first with `max-age=300`).

---

## 5. Rollback

- **Cloudflare Workers:** roll back to the previous deployment in **Cloudflare Dashboard → Workers & Pages → `tiffin` → Deployments** (or run `npx wrangler rollback --config deploy/wrangler.json`, < 1 minute).
- **Self-hosted server:** replace `desktop/dist-web` with the previous build snapshot and `docker compose -f deploy/docker-compose.yml up -d`. Because each deploy's service worker uses an immutable content-hashed cache (`tiffin-shell-<hash>`), browsers pick up the previous build cleanly on next tab launch without corrupting user data in OPFS.
