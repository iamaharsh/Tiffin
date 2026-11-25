<script lang="ts">
  import { slide, fade } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import * as api from './api';
  import { holdsUnsaved } from './updates.svelte';
  import type { Review, RuleRow } from './api';

  let { queue, onreview, onopen, onsetupclaude, onerror, onnote }: {
    queue: Review[];
    /** open Settings › Send from Claude */
    onsetupclaude: () => void;
    onreview: (rv: Review) => void;
    onopen: (i: number) => void;
    onerror: (m: string) => void;
    onnote: (m: string) => void;
  } = $props();

  let url = $state('');
  let paste = $state('');
  holdsUnsaved(() => url.trim() !== '' || paste.trim() !== '');
  let reading: { id: number; url: string }[] = $state([]);
  let rules: RuleRow[] = $state([]);
  let updatedAny = $state(false);
  let inbox = $state('');
  let testing = $state(false);
  let rulesFile: HTMLInputElement;
  let seq = 0;

  async function loadRules(test = false) {
    try {
      const r = await api.rulesOverview(test);
      rules = r.rules;
      updatedAny = r.updated_any;
      inbox = r.inbox;
    } catch (e) {
      onerror(String(e));
    }
  }
  $effect(() => { loadRules(); });

  // Several pages can be reading at once; each goes to review when it arrives.
  async function importUrl() {
    const u = url.trim();
    if (!u) return;
    url = '';
    const id = ++seq;
    reading = [...reading, { id, url: u }];
    try {
      onreview(await api.importUrl(u));
      loadRules();
    } catch (e) {
      onerror(String(e));
    }
    reading = reading.filter((r) => r.id !== id);
  }

  async function readPaste() {
    if (!paste.trim()) return;
    try {
      onreview(await api.importText(paste));
      paste = '';
    } catch (e) {
      onerror(String(e));
    }
  }

  async function test() {
    testing = true;
    await loadRules(true);
    testing = false;
    const failed = rules.filter((r) => r.test && !r.test[0]).length;
    onnote(failed ? `${failed} rule${failed === 1 ? '' : 's'} failed its test` : `All ${rules.length} rules passed their tests`);
  }

  async function rulesUpdate(e: Event) {
    const f = (e.currentTarget as HTMLInputElement).files?.[0];
    (e.currentTarget as HTMLInputElement).value = '';
    if (!f) return;
    try {
      onnote(await api.loadRules(await f.text()));
      await loadRules(true);
    } catch (err) {
      onerror(String(err));
    }
  }

  async function fromSync() {
    try {
      onnote(await api.rulesFromSync());
      await loadRules(true);
    } catch (e) {
      onerror(String(e));
    }
  }

  async function builtin() {
    try {
      await api.useBuiltinRules();
      onnote('Back to the built-in rules');
      await loadRules();
    } catch (e) {
      onerror(String(e));
    }
  }
</script>

<div class="page">
  <section class="hero">
    <span class="tile"><Icon name="globe" size={26} /></span>
    <h1>Import a Recipe</h1>
    <p>Paste a recipe page’s address. Tiffin reads the page’s built-in recipe data first, then the tested rule for that site, then common recipe layouts. Anything it isn’t sure about is flagged for you to check before it’s saved.</p>
    <form class="url" onsubmit={(e) => { e.preventDefault(); importUrl(); }}>
      <Icon name="link" size={17} />
      <input bind:value={url} placeholder="https://www.allrecipes.com/recipe/…" aria-label="Recipe web address" spellcheck="false" autocomplete="off" inputmode="url" />
      <button class="go" disabled={!url.trim()}>Import</button>
    </form>
  </section>

  {#if reading.length || queue.length}
    <section class="waiting" transition:slide={{ duration: 220 }}>
      {#each reading as r (r.id)}
        <div class="row" transition:fade={{ duration: 150 }}>
          <span class="spin" aria-hidden="true"></span>
          <span class="what">Reading <span class="addr">{r.url}</span></span>
        </div>
      {/each}
      {#each queue as rv, i}
        <div class="row" transition:fade={{ duration: 150 }}>
          <Icon name={rv.field_issues.length ? 'warn' : 'check'} size={16} />
          <span class="what"><strong>{rv.recipe.name || 'Untitled recipe'}</strong> <span class="addr">{rv.origin}</span></span>
          <button class="review" onclick={() => onopen(i)}>Review</button>
        </div>
      {/each}
    </section>
  {/if}

  <section>
    <h2 class="caps">Or paste the recipe</h2>
    <textarea bind:value={paste} rows="6" placeholder={'Title on the first line, then Ingredients and Directions.\nA web page’s HTML works too.'}></textarea>
    <div class="right"><button class="btn" onclick={readPaste} disabled={!paste.trim()}>Read Pasted Recipe</button></div>
  </section>

  <section>
    <div class="head">
      <h2 class="caps">Site rules</h2>
      <button class="btn small" onclick={test} disabled={testing}>{testing ? 'Testing…' : 'Run Rule Tests'}</button>
    </div>
    <p class="fine">Each rule is tested against a sample page in that site’s layout. When a site changes its layout, its rule is marked here, and a rules update file can replace it (only rules that pass their test are accepted).</p>
    <div class="table" role="table" aria-label="Site rules">
      <div class="tr th" role="row"><span role="columnheader">Site</span><span role="columnheader">Rule</span><span role="columnheader">Checked</span><span role="columnheader">Last import</span><span role="columnheader">Test</span></div>
      {#each rules as r (r.domain)}
        <div class="tr" role="row">
          <span role="cell" class="dom">{r.domain}</span>
          <span role="cell" class="num">v{r.version}{#if r.updated}<em> updated</em>{/if}</span>
          <span role="cell">{r.verified}</span>
          <span role="cell" class:bad={r.needs_update} class:good={r.last_import && !r.needs_update}>{r.needs_update ? 'Needs update: layout changed' : (r.last_import ?? 'Not used yet')}</span>
          <span role="cell" class:bad={r.test && !r.test[0]} class:good={r.test?.[0]}>{r.test ? (r.test[0] ? 'Passed' : `Failed: ${r.test[1]}`) : '—'}</span>
        </div>
      {/each}
    </div>
    <div class="right">
      {#if updatedAny}<button class="btn small ghost" onclick={builtin}>Use Built-In Rules</button>{/if}
      {#if !api.isWeb}<button class="btn small" onclick={fromSync}>Check Sync Folder</button>{/if}
      <button class="btn small" onclick={() => rulesFile.click()}>Load Rules Update…</button>
      <input bind:this={rulesFile} type="file" accept=".json,application/json" hidden onchange={rulesUpdate} />
    </div>
  </section>

  {#if !api.isWeb}
    <section>
      <h2 class="caps">Recipe inbox</h2>
      <p class="fine">Claude or any other app can send recipes here: drop a web link, recipe text or recipe JSON into <code class="selectable">{inbox}</code>, or run <code>recipe_box --import &lt;file or link&gt;</code>. New recipes appear above, ready to review.</p>
      <button class="link" onclick={onsetupclaude}>Set up sending from Claude…</button>
    </section>
  {/if}
</div>

<style>
  .link { display: block; margin-top: 8px; padding: 0; font-size: 14px; font-weight: 500; color: var(--accent-text); }
  .link:hover { text-decoration: underline; }
  /* white text needs a deeper blue than the sidebar tile (4.5:1) */
  .page { --browser-deep: color-mix(in srgb, var(--tile-browser) 82%, black); max-width: 860px; margin: 0 auto; padding: 36px 0 80px; display: flex; flex-direction: column; gap: 34px; }
  .hero { display: flex; flex-direction: column; align-items: flex-start; gap: 10px; }
  .tile {
    display: grid;
    place-items: center;
    width: 48px;
    height: 48px;
    border-radius: 14px;
    color: #fff;
    background: var(--tile-browser);
    box-shadow: var(--shadow-card), inset 0 -2px 0 rgba(0, 0, 0, 0.15);
  }
  h1 { margin: 6px 0 0; font: 700 30px/1.1 var(--font-display); letter-spacing: -0.02em; }
  .hero p { margin: 0; max-width: 64ch; color: var(--ink-2); }
  .url {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    margin-top: 8px;
    padding: 0 6px 0 16px;
    height: 52px;
    border-radius: 14px;
    background: var(--card);
    box-shadow: var(--shadow-card), inset 0 0 0 1px var(--rule);
    color: var(--ink-3);
    transition: box-shadow 160ms ease-out;
  }
  .url:focus-within { box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  .url input { flex: 1; min-width: 0; border: 0; outline: none; background: none; color: var(--ink); font: inherit; font-size: 15.5px; -webkit-user-select: text; user-select: text; }
  .url input::placeholder { color: var(--ink-3); }
  .go {
    height: 40px;
    padding: 0 20px;
    border-radius: 10px;
    font-weight: 600;
    color: #fff;
    background: var(--browser-deep);
    transition: filter 120ms ease-out, transform 100ms ease-out, opacity 120ms ease-out;
  }
  .go:hover:not(:disabled) { filter: brightness(1.08); }
  .go:active:not(:disabled) { transform: scale(0.97); }
  .go:disabled { opacity: 0.45; }

  .waiting { display: flex; flex-direction: column; gap: 2px; padding: 6px; border-radius: 14px; background: color-mix(in srgb, var(--tile-browser) 8%, var(--page)); }
  .row { display: flex; align-items: center; gap: 10px; min-height: 40px; padding: 0 10px; color: var(--tile-browser); }
  .what { flex: 1; min-width: 0; color: var(--ink); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .addr { margin-left: 4px; font-size: 12.5px; color: var(--ink-3); }
  .review { height: 30px; padding: 0 14px; border-radius: 8px; font-size: 13.5px; font-weight: 600; color: #fff; background: var(--browser-deep); }
  .review:hover { filter: brightness(1.08); }
  .spin { width: 16px; height: 16px; border-radius: 50%; border: 2px solid color-mix(in srgb, var(--tile-browser) 25%, transparent); border-top-color: var(--tile-browser); animation: spin 800ms linear infinite; }
  @keyframes spin { to { transform: rotate(1turn); } }
  @media (prefers-reduced-motion: reduce) { .spin { animation-duration: 2.4s !important; } }

  section > h2, .head h2 { margin: 0 0 10px; font-size: 16px; color: var(--ink-2); }
  .head { display: flex; align-items: baseline; justify-content: space-between; }
  .head h2 { margin: 0; }
  textarea {
    width: 100%;
    padding: 10px 12px;
    border: 0;
    border-radius: 12px;
    background: var(--well);
    box-shadow: inset 0 0 0 1px var(--rule);
    color: var(--ink);
    font: inherit;
    line-height: 1.5;
    resize: vertical;
    outline: none;
    -webkit-user-select: text;
    user-select: text;
  }
  textarea:focus { background: var(--card); box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  textarea::placeholder { color: var(--ink-3); }
  .right { display: flex; justify-content: flex-end; gap: 8px; margin-top: 10px; }
  .btn { height: 34px; padding: 0 16px; border-radius: 9px; font-size: 14px; font-weight: 500; background: var(--well); box-shadow: inset 0 0 0 1px var(--rule); }
  .btn:hover:not(:disabled) { background: var(--rule); }
  .btn:disabled { opacity: 0.5; }
  .btn.small { height: 30px; font-size: 13px; padding: 0 12px; }
  .btn.ghost { background: none; box-shadow: none; color: var(--accent-text); }
  .fine { margin: 6px 0 12px; font-size: 13px; color: var(--ink-2); max-width: 75ch; }
  code { font-size: 12.5px; padding: 1px 5px; border-radius: 5px; background: var(--well); color: var(--ink); overflow-wrap: anywhere; }
  .table { border-radius: 12px; overflow: hidden; box-shadow: inset 0 0 0 1px var(--rule); font-size: 13.5px; }
  .tr { display: grid; grid-template-columns: 1.4fr 0.7fr 0.9fr 1.6fr 1.3fr; gap: 12px; padding: 8px 14px; border-top: 1px solid var(--rule); }
  .tr > span { min-width: 0; overflow-wrap: anywhere; }
  .th { border-top: 0; background: var(--well); font-weight: 600; color: var(--ink-2); font-size: 12.5px; }
  .dom { font-weight: 500; }
  em { font-style: normal; font-size: 12px; color: var(--accent-text); }
  .good { color: var(--ok); }
  .bad { color: var(--heart); }

  @media (max-width: 760px) {
    .tr { grid-template-columns: 1fr 1fr; }
    .th { display: none; }
  }
</style>
