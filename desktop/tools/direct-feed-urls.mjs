// Points every latest.json platform url at the release's direct -setup.exe download (tauri-action writes api.github.com links).
// CI runs it after tauri-action; by hand for a published release: node desktop/tools/direct-feed-urls.mjs <latest.json> <release.json> <owner/repo> <tag>
import { readFileSync, writeFileSync } from 'node:fs';

const [feedPath, releasePath, repo, tag] = process.argv.slice(2);
const refuse = (why) => {
  console.error(why);
  process.exit(1);
};
if (!feedPath || !releasePath || !repo || !tag) refuse('Usage: direct-feed-urls.mjs <latest.json> <release.json> <owner/repo> <tag>');

const feed = JSON.parse(readFileSync(feedPath, 'utf8'));
const release = JSON.parse(readFileSync(releasePath, 'utf8'));

if (release.tag_name !== tag) refuse(`The release data is for ${release.tag_name}, not ${tag}.`);
const entries = Object.entries(feed.platforms ?? {});
if (!entries.length) refuse('The feed has no platforms.');

const direct = `https://github.com/${repo}/releases/download/${tag}/`;
for (const [key, platform] of entries) {
  // An already-direct url maps to itself, so a second run changes nothing.
  const asset = (release.assets ?? []).find((a) => a.url === platform.url || a.browser_download_url === platform.url);
  if (!asset) refuse(`Platform ${key} points at ${platform.url}, which is no asset of release ${tag}.`);
  platform.url = asset.browser_download_url;
}
for (const [key, platform] of entries) {
  if (!platform.url.startsWith(direct) || !platform.url.endsWith('-setup.exe')) refuse(`Platform ${key} url ${platform.url} is not a direct -setup.exe download of ${tag} in ${repo}.`);
}

writeFileSync(feedPath, JSON.stringify(feed, null, 2) + '\n');
for (const [key, platform] of entries) console.log(`${key} ${platform.url}`);
