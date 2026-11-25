// Verifies the built web bundle (desktop/dist-web) and self-host deploy config (deploy/Caddyfile,
// deploy/docker-compose.yml) carry matching strict CSP, COOP/COEP/CORP, Permissions-Policy,
// privacy page hygiene, and SW precache rules.
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const desktopDir = path.resolve(here, '..');
const repoRoot = path.resolve(desktopDir, '..');
const distWeb = path.join(desktopDir, 'dist-web');

let failed = 0;
function check(ok, msg) {
  if (ok) {
    console.log(`PASS  ${msg}`);
  } else {
    console.error(`FAIL  ${msg}`);
    failed++;
  }
}

function read(p) {
  if (!fs.existsSync(p)) {
    check(false, `file exists: ${path.relative(repoRoot, p)}`);
    return '';
  }
  return fs.readFileSync(p, 'utf8');
}

function parseCsp(raw) {
  const map = new Map();
  for (const part of raw.split(';')) {
    const trimmed = part.trim();
    if (!trimmed) continue;
    const sp = trimmed.indexOf(' ');
    if (sp === -1) map.set(trimmed.toLowerCase(), '');
    else map.set(trimmed.slice(0, sp).toLowerCase(), trimmed.slice(sp + 1).trim());
  }
  return map;
}

function checkAppCsp(label, raw, requireUpgrade) {
  const csp = parseCsp(raw);
  check(csp.get('default-src') === "'self'", `${label}: default-src is exactly 'self'`);
  check(csp.get('script-src') === "'self' 'wasm-unsafe-eval'", `${label}: script-src is exactly 'self' 'wasm-unsafe-eval'`);
  check(csp.get('worker-src') === "'self'", `${label}: worker-src is exactly 'self'`);
  check(!raw.includes("'unsafe-eval'"), `${label}: forbids JS 'unsafe-eval'`);
  check(!raw.includes('*'), `${label}: contains no wildcards`);
  check(
    csp.get('object-src') === "'none'" &&
      csp.get('base-uri') === "'none'" &&
      csp.get('form-action') === "'none'" &&
      csp.get('frame-src') === "'none'" &&
      csp.get('frame-ancestors') === "'none'",
    `${label}: locks down object/base/form/frame/frame-ancestors to 'none'`,
  );
  const conn = (csp.get('connect-src') ?? '').split(/\s+/).filter(Boolean);
  check(conn.includes("'self'") && conn.includes('https://world.openfoodfacts.org') && conn.length === 3, `${label}: connect-src allows 'self', Open Food Facts, and one helper origin`);
  const helperTok = conn.find((t) => t !== "'self'" && t !== 'https://world.openfoodfacts.org') ?? '';
  check(/^https?:\/\/\S+$/.test(helperTok), `${label}: connect-src includes a concrete helper origin (${helperTok || 'missing'})`);
  if (requireUpgrade) {
    check(csp.has('upgrade-insecure-requests'), `${label}: enforces upgrade-insecure-requests`);
  }
}

function checkPrivacyCsp(label, raw) {
  const csp = parseCsp(raw);
  check(csp.get('default-src') === "'none'", `${label}: default-src is 'none'`);
  check(csp.get('style-src') === "'self'", `${label}: style-src is 'self' only`);
  check(csp.get('img-src') === "'self'", `${label}: img-src is 'self' only`);
  check(!csp.has('script-src') && !raw.includes("'unsafe-inline'"), `${label}: no script-src and no 'unsafe-inline'`);
  check(
    csp.get('base-uri') === "'none'" && csp.get('form-action') === "'none'" && csp.get('frame-ancestors') === "'none'",
    `${label}: locks down base-uri/form-action/frame-ancestors`,
  );
  check(csp.has('upgrade-insecure-requests'), `${label}: enforces upgrade-insecure-requests`);
}

const headersTxt = read(path.join(distWeb, '_headers'));
const swTxt = read(path.join(distWeb, 'sw.js'));
const privacyHtml = read(path.join(distWeb, 'privacy', 'index.html'));
const manifestTxt = read(path.join(distWeb, 'manifest.webmanifest'));
const caddyTxt = read(path.join(repoRoot, 'deploy', 'Caddyfile'));
const composeTxt = read(path.join(repoRoot, 'deploy', 'docker-compose.yml'));

// 1. Cloudflare Pages _headers
if (headersTxt) {
  const blocks = new Map();
  let currentPath = null;
  for (const rawLine of headersTxt.split(/\r?\n/)) {
    const line = rawLine.trimEnd();
    if (!line.trim() || line.trim().startsWith('#')) continue;
    if (!/^\s/.test(line)) {
      currentPath = line.trim();
      blocks.set(currentPath, []);
    } else if (currentPath) {
      blocks.get(currentPath).push(line.trim());
    }
  }

  const rootLines = blocks.get('/*') ?? [];
  check(rootLines.includes('Cross-Origin-Opener-Policy: same-origin'), '_headers /* sets COOP same-origin');
  check(rootLines.includes('Cross-Origin-Embedder-Policy: require-corp'), '_headers /* sets COEP require-corp');
  check(rootLines.includes('Cross-Origin-Resource-Policy: same-origin'), '_headers /* sets CORP same-origin');
  check(rootLines.includes('X-Content-Type-Options: nosniff'), '_headers /* sets X-Content-Type-Options nosniff');
  check(rootLines.includes('X-Frame-Options: DENY'), '_headers /* sets X-Frame-Options DENY');
  check(rootLines.includes('Referrer-Policy: no-referrer'), '_headers /* sets Referrer-Policy no-referrer');
  check(rootLines.some((l) => l.startsWith('Strict-Transport-Security:')), '_headers /* sets HSTS');
  check(rootLines.includes('Cache-Control: no-cache'), '_headers /* sets Cache-Control no-cache by default');
  check(
    rootLines.includes('Permissions-Policy: camera=(self), screen-wake-lock=(self), microphone=(), geolocation=(), payment=(), usb=()'),
    '_headers /* sets scoped Permissions-Policy allowing camera and wake lock',
  );

  const rootCsp = rootLines.find((l) => l.startsWith('Content-Security-Policy:'))?.slice('Content-Security-Policy:'.length).trim() ?? '';
  checkAppCsp('_headers /*', rootCsp, true);

  for (const p of ['/privacy', '/privacy/*']) {
    const lines = blocks.get(p) ?? [];
    check(lines.includes('! Content-Security-Policy'), `_headers ${p} detaches inherited CSP`);
    const pCsp = lines.find((l) => l.startsWith('Content-Security-Policy:'))?.slice('Content-Security-Policy:'.length).trim() ?? '';
    checkPrivacyCsp(`_headers ${p}`, pCsp);
    check(
      lines.includes('! Permissions-Policy') && lines.some((l) => l.startsWith('Permissions-Policy:') && l.includes('camera=()')),
      `_headers ${p} detaches Permissions-Policy and blocks camera`,
    );
  }

  const assetLines = blocks.get('/assets/*') ?? [];
  check(
    assetLines.includes('! Cache-Control') && assetLines.includes('Cache-Control: public, max-age=31536000, immutable'),
    '_headers /assets/* detaches Cache-Control and sets immutable caching',
  );
}

// 2. Service worker syntax, precache, and lifecycle invariants
if (swTxt) {
  try {
    new vm.Script(swTxt);
    check(true, 'sw.js parses as valid JavaScript');
  } catch (e) {
    check(false, `sw.js parses as valid JavaScript (${e instanceof Error ? e.message : e})`);
  }
  const precacheMatch = swTxt.match(/const PRECACHE = (\[[^\n]+\]);/);
  let precache = [];
  if (precacheMatch) {
    try {
      precache = JSON.parse(precacheMatch[1]);
    } catch {
      precache = [];
    }
  }
  check(precache.includes('./'), 'sw.js precaches app shell ./');
  check(precache.includes('./tiffin.wasm'), 'sw.js precaches ./tiffin.wasm');
  check(precache.includes('./manifest.webmanifest'), 'sw.js precaches ./manifest.webmanifest');
  check(precache.includes('./privacy/'), 'sw.js precaches canonical ./privacy/ (no .html redirect)');
  check(precache.includes('./privacy.css'), 'sw.js precaches ./privacy.css');
  check(!precache.includes('./_headers'), 'sw.js does not precache host-only ./_headers');
  check(!swTxt.includes('skipWaiting'), 'sw.js avoids skipWaiting() mid-session');
  check(!swTxt.includes('clients.claim'), 'sw.js avoids clients.claim() mid-session');
  check(swTxt.includes('!cacheable(res)'), 'sw.js enforces cacheable() during install');
}

// 3. Standalone zero-JS privacy page
if (privacyHtml) {
  check(!/<script\b/i.test(privacyHtml), 'privacy page has zero <script> tags');
  check(!/\son[a-z]+\s*=/i.test(privacyHtml), 'privacy page has zero inline event handlers');
  check(!/<style\b/i.test(privacyHtml) && !/\sstyle\s*=/i.test(privacyHtml), 'privacy page has zero inline styles');
  check(privacyHtml.includes('name="color-scheme" content="light dark"'), 'privacy page declares color-scheme light dark');
  check(privacyHtml.includes('href="/privacy.css"'), 'privacy page links external /privacy.css');
}

// 4. Web manifest
if (manifestTxt) {
  try {
    const manifest = JSON.parse(manifestTxt);
    check(manifest.display === 'standalone' && manifest.start_url === '/', 'manifest declares standalone PWA at /');
    check(
      Array.isArray(manifest.icons) && manifest.icons.some((i) => i.sizes === '192x192') && manifest.icons.some((i) => i.sizes === '512x512'),
      'manifest includes 192x192 and 512x512 icons',
    );
  } catch (e) {
    check(false, `manifest.webmanifest parses as valid JSON (${e instanceof Error ? e.message : e})`);
  }
}

// 5. Self-hosted Caddyfile & Docker Compose parity
if (caddyTxt) {
  check(/log default\s*\{[\s\S]*?exclude http\.log\.error[\s\S]*?\}/.test(caddyTxt), 'Caddyfile excludes http.log.error globally so proxy errors never log query strings or IPs');
  const discardCount = (caddyTxt.match(/output discard/g) ?? []).length;
  check(discardCount >= 2, `Caddyfile discards access logs in both site blocks (found ${discardCount})`);
  check(caddyTxt.includes('header -Server'), 'Caddyfile isolates header -Server so auto-deferral does not clobber Cache-Control or CSP');
  check(caddyTxt.includes('Cross-Origin-Opener-Policy "same-origin"'), 'Caddyfile sets COOP same-origin');
  check(caddyTxt.includes('Cross-Origin-Embedder-Policy "require-corp"'), 'Caddyfile sets COEP require-corp');
  check(caddyTxt.includes('Cross-Origin-Resource-Policy "same-origin"'), 'Caddyfile sets CORP same-origin');
  check(caddyTxt.includes('X-Content-Type-Options "nosniff"'), 'Caddyfile sets X-Content-Type-Options nosniff');
  check(caddyTxt.includes('X-Frame-Options "DENY"'), 'Caddyfile sets X-Frame-Options DENY');
  check(caddyTxt.includes('Referrer-Policy "no-referrer"'), 'Caddyfile sets Referrer-Policy no-referrer');
  check(caddyTxt.includes('Strict-Transport-Security "max-age='), 'Caddyfile sets HSTS');
  check(caddyTxt.includes('header @assets Cache-Control "public, max-age=31536000, immutable"'), 'Caddyfile sets immutable Cache-Control for /assets/*');
  check(caddyTxt.includes('?Cache-Control "no-cache"'), 'Caddyfile uses ?Cache-Control fallback so @assets is not overwritten');
  check(caddyTxt.includes('?Permissions-Policy "camera=(self), screen-wake-lock=(self), microphone=(), geolocation=(), payment=(), usb=()"'), 'Caddyfile uses ?Permissions-Policy fallback');
  check(caddyTxt.includes('hide .* _headers') && caddyTxt.includes('respond @internal 404'), 'Caddyfile hides and blocks /_headers');

  const caddyAppCsp = caddyTxt.match(/\?Content-Security-Policy\s+"([^"]+)"/)?.[1] ?? '';
  checkAppCsp('Caddyfile default', caddyAppCsp, true);

  const caddyPrivCsp = caddyTxt.match(/header @privacy\s*\{[\s\S]*?Content-Security-Policy\s+"([^"]+)"/)?.[1] ?? '';
  checkPrivacyCsp('Caddyfile @privacy', caddyPrivCsp);
}

if (composeTxt) {
  const helperSection = composeTxt.split(/^  tiffin-helper:/m)[1]?.split(/^  caddy:/m)[0] ?? '';
  const caddySection = composeTxt.split(/^  caddy:/m)[1]?.split(/^networks:/m)[0] ?? '';

  check(helperSection.includes('read_only: true'), 'docker-compose runs tiffin-helper read-only');
  check(helperSection.includes('cap_drop:') && helperSection.includes('- ALL'), 'docker-compose drops all capabilities on tiffin-helper');
  check(helperSection.includes('no-new-privileges:true'), 'docker-compose sets no-new-privileges on tiffin-helper');
  check(helperSection.includes('TIFFIN_BEHIND_PROXY=1'), 'docker-compose enables TIFFIN_BEHIND_PROXY behind Caddy');
  check(!/^\s*ports:/m.test(helperSection), 'docker-compose publishes no host ports on tiffin-helper');

  check(caddySection.includes('read_only: true'), 'docker-compose runs caddy read-only');
  check(caddySection.includes('cap_drop:') && caddySection.includes('- ALL'), 'docker-compose drops all capabilities on caddy');
  check(caddySection.includes('no-new-privileges:true'), 'docker-compose sets no-new-privileges on caddy');
}

if (failed > 0) {
  console.error(`web-security-check: ${failed} check(s) failed`);
  process.exit(1);
}
console.log('web-security-check: all passed');
