<script lang="ts">
  import { slide } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import * as api from './api';
  import type { Category, LineCheck, Review } from './api';

  let { initial, categories, waiting, onsaved, ondiscard, onerror, onnote }: {
    initial: Review;
    categories: Category[];
    waiting: number;
    onsaved: (id: number, msg: string) => void;
    ondiscard: () => void;
    onerror: (m: string) => void;
    onnote: (m: string) => void;
  } = $props();

  // svelte-ignore state_referenced_locally -- the parent remounts this page per review
  let rv: Review = $state($state.snapshot(initial));
  // svelte-ignore state_referenced_locally
  let onlyFlagged = $state(initial.existing_id != null);
  let ing: LineCheck[] = $state([]);
  let steps: LineCheck[] = $state([]);
  let saving = $state(false);
  let showTried = $state(false);
  // stable keys so rows animate when one is removed or joined
  // svelte-ignore state_referenced_locally
  let ingKeys = $state(rv.ing_lines.map((_, i) => i));
  // svelte-ignore state_referenced_locally
  let stepKeys = $state(rv.step_lines.map((_, i) => i));
  let nextKey = 100000;

  // Each line is re-checked as it's typed (a short pause first, so typing stays smooth).
  let timer = 0;
  $effect(() => {
    const a = $state.snapshot(rv.ing_lines), b = $state.snapshot(rv.step_lines);
    clearTimeout(timer);
    timer = window.setTimeout(() => api.checkLines(a, b).then((c) => { ing = c.ing; steps = c.steps; }), ing.length ? 180 : 0);
  });

  const flaggedIng = $derived(ing.filter((c) => !c.ok).length);
  const flaggedSteps = $derived(steps.filter((c) => !c.ok).length);
  const total = $derived(flaggedIng + flaggedSteps + rv.field_issues.length);
  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  const summary = $derived.by(() => {
    if (!total) return 'Everything was read confidently.';
    const parts = [
      flaggedIng && plural(flaggedIng, 'ingredient line', 'ingredient lines'),
      flaggedSteps && plural(flaggedSteps, 'step', 'steps'),
      rv.field_issues.length && plural(rv.field_issues.length, 'other item', 'other items'),
    ].filter(Boolean) as string[];
    const list = parts.length === 1 ? parts[0] : parts.length === 2 ? parts.join(' and ') : `${parts[0]}, ${parts[1]} and ${parts[2]}`;
    return `${list} ${total === 1 ? 'needs' : 'need'} a quick look.`;
  });
  const saveLabel = $derived(rv.existing_id != null ? 'Save' : rv.duplicate && rv.merge ? 'Merge into Library' : 'Save to Library');

  async function save() {
    if (saving) return;
    saving = true;
    try {
      const r = await api.saveReview($state.snapshot(rv) as Review);
      onsaved(r.id, r.message);
    } catch (e) {
      onerror(String(e));
      saving = false;
    }
  }

  async function fixAll() {
    try {
      const t = await api.reviewTidy($state.snapshot(rv) as Review);
      rv = t;
      ingKeys = t.ing_lines.map(() => nextKey++);
      stepKeys = t.step_lines.map(() => nextKey++);
      if (!t.changes.length) onnote('Already tidy: nothing to fix.');
    } catch (e) {
      onerror(String(e));
    }
  }

  function removeIng(i: number) { rv.ing_lines.splice(i, 1); ingKeys.splice(i, 1); }
  function addIng() { rv.ing_lines.push(''); ingKeys.push(nextKey++); }
  function removeStep(i: number) { rv.step_lines.splice(i, 1); stepKeys.splice(i, 1); }
  function joinStep(i: number) {
    rv.step_lines[i - 1] = `${rv.step_lines[i - 1].trimEnd()} ${rv.step_lines[i].trimStart()}`;
    removeStep(i);
  }
  function addStep() { rv.step_lines.push(''); stepKeys.push(nextKey++); }

  function toggleCategory(path: string) {
    const c = rv.recipe.categories;
    rv.recipe.categories = c.includes(path) ? c.filter((x) => x !== path) : [...c, path];
  }
  const groups = $derived(
    categories.filter((c) => c.depth === 0).map((top) => ({ top, subs: categories.filter((c) => c.path.startsWith(top.path + '/')) })),
  );

  const show = (c: LineCheck | undefined) => !onlyFlagged || (c && !c.ok);

  function keys(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
      e.preventDefault();
      save();
    }
  }
</script>

<svelte:window onkeydown={keys} />

<header class="bar">
  <div class="title">
    <h1>{rv.existing_id != null ? 'Review & Tidy' : 'Review Import'}</h1>
    <span class="origin">{rv.origin}{#if waiting} · {waiting} more waiting{/if}</span>
  </div>
  <button class="btn ghost" onclick={fixAll} title="Standardise amounts and units, remove clutter and numbering, fix capitals and broken steps, all at once">
    <Icon name="wand" size={16} /> Fix All Formatting
  </button>
  <button class="btn" onclick={ondiscard}>{rv.existing_id != null ? 'Cancel' : 'Discard'}</button>
  <button class="btn primary" onclick={save} disabled={saving}>{saving ? 'Saving…' : saveLabel}</button>
</header>

<div class="page">
  <section class="card summary" class:clear={!total}>
    <div class="head"><Icon name={total ? 'warn' : 'check'} size={18} /><strong>{summary}</strong></div>
    {#if rv.method}
      <p class="method">
        Read using: {rv.method}
        {#if rv.tried.length}<button class="link" onclick={() => (showTried = !showTried)} aria-expanded={showTried}>{showTried ? 'Hide' : 'Show'} what was tried</button>{/if}
      </p>
    {/if}
    {#if showTried}
      <ul class="tried" transition:slide={{ duration: 180 }}>{#each rv.tried as [m, o]}<li>{m}: {o}</li>{/each}</ul>
    {/if}
    {#if rv.rule_worked === false}
      <p class="bad">The import rule for {rv.rule_domain} found nothing: the site’s layout has probably changed. It’s marked for an update on the Browser page.</p>
    {/if}
    {#each rv.field_issues as f}<p class="issue"><Icon name="warn" size={14} /> {f}</p>{/each}
  </section>

  {#if rv.duplicate}
    {@const d = rv.duplicate}
    <section class="card dup">
      <strong>Already in your library: “{d.name}”</strong>
      <p class="fine">{d.reason} ({Math.round(d.score * 100)}% match)</p>
      <label class="radio"><input type="radio" name="dup" checked={rv.merge} onchange={() => (rv.merge = true)} /> <span><b>Merge into it.</b> Keeps your favorite, rating, collections and notes, and fills in anything missing.</span></label>
      <label class="radio"><input type="radio" name="dup" checked={!rv.merge} onchange={() => (rv.merge = false)} /> <span><b>Keep both</b> as separate recipes.</span></label>
    </section>
  {/if}

  {#if rv.changes.length}
    <section class="card fixed" transition:slide={{ duration: 220 }}>
      <strong>Fixed in one pass</strong>
      <ul>{#each rv.changes as c}<li><Icon name="check" size={14} /> {c}</li>{/each}</ul>
    </section>
  {/if}

  <div class="fields">
    <label class="f wide"><span>Title</span><input bind:value={rv.recipe.name} autocomplete="off" /></label>
    <label class="f"><span>Servings</span><input bind:value={rv.recipe.servings} autocomplete="off" /></label>
    <label class="f"><span>Prep</span><input bind:value={rv.recipe.prep_time} autocomplete="off" /></label>
    <label class="f"><span>Cook</span><input bind:value={rv.recipe.cook_time} autocomplete="off" /></label>
    <label class="f"><span>Total</span><input bind:value={rv.recipe.total_time} autocomplete="off" /></label>
  </div>

  {#if groups.length}
    <details class="cats">
      <summary>
        <span class="caps">Categories</span>
        <span class="chosen">{rv.recipe.categories.length ? rv.recipe.categories.map((c) => c.split('/').join(' › ')).join(', ') : 'None yet'}</span>
        <span class="open-hint">Choose</span>
      </summary>
      {#each groups as g (g.top.path)}
        <div class="chips">
          {#each [g.top, ...g.subs] as c (c.path)}
            {@const on = rv.recipe.categories.includes(c.path)}
            <button type="button" class="chip" class:sub={c.depth > 0} class:on style:--l={api.labelVar(g.top.label)} aria-pressed={on} onclick={() => toggleCategory(c.path)}>
              <span class="tick"><Icon name="check" size={11} /></span>{c.path.slice(g.top.path.length + 1).split('/').join(' › ') || c.name}
            </button>
          {/each}
        </div>
      {/each}
    </details>
  {/if}

  <label class="only"><input type="checkbox" bind:checked={onlyFlagged} /> Show only lines that need a look</label>

  <section>
    <h2 class="caps">Ingredients <em class="num">{rv.ing_lines.length}</em></h2>
    <p class="fine">Each line is split into amount · unit · ingredient · preparation. Edit the text and the split updates as you type.</p>
    <ol class="lines">
      {#each rv.ing_lines as _, i (ingKeys[i])}
        {@const c = ing[i]}
        {#if show(c)}
          <li class:flag={c && !c.ok} class:low={c?.low} transition:slide={{ duration: 160 }}>
            <span class="dot" title={c && !c.ok ? 'Needs a look' : 'Read confidently'}><Icon name={c && !c.ok ? 'warn' : 'check'} size={14} /></span>
            <div class="line">
              <input bind:value={rv.ing_lines[i]} aria-label="Ingredient {i + 1}" spellcheck="false" />
              <span class="parsed">{c?.parsed ?? ''}</span>
              {#if c && !c.ok}<span class="why">{c.issues.join(' · ')}</span>{/if}
            </div>
            <button class="x" aria-label="Remove ingredient {i + 1}" onclick={() => removeIng(i)}><Icon name="close" size={14} /></button>
          </li>
        {/if}
      {/each}
    </ol>
    <button class="add" onclick={addIng}><Icon name="plus" size={15} /> Add Ingredient</button>
  </section>

  <section>
    <h2 class="caps">Directions <em class="num">{rv.step_lines.length} steps</em></h2>
    <ol class="lines">
      {#each rv.step_lines as _, i (stepKeys[i])}
        {@const c = steps[i]}
        {#if show(c)}
          <li class="step" class:flag={c && !c.ok} class:low={c?.low} transition:slide={{ duration: 160 }}>
            <span class="n num">{i + 1}</span>
            <div class="line">
              <textarea bind:value={rv.step_lines[i]} rows="2" aria-label="Step {i + 1}"></textarea>
              {#if c && !c.ok}<span class="why"><Icon name="warn" size={13} /> {c.issues.join(' · ')}</span>{/if}
            </div>
            <div class="tools">
              <button class="x" aria-label="Remove step {i + 1}" onclick={() => removeStep(i)}><Icon name="close" size={14} /></button>
              {#if i > 0}<button class="x" aria-label="Join step {i + 1} onto step {i}" title="Join with the previous step" onclick={() => joinStep(i)}><Icon name="join" size={14} /></button>{/if}
            </div>
          </li>
        {/if}
      {/each}
    </ol>
    <button class="add" onclick={addStep}><Icon name="plus" size={15} /> Add Step</button>
  </section>

  <section>
    <h2 class="caps">Notes</h2>
    <textarea class="notes" bind:value={rv.recipe.notes} rows="3"></textarea>
  </section>
</div>

<style>
  .bar {
    position: sticky;
    top: 0;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 64px;
    margin: 0 -32px;
    padding: 0 32px;
    background: var(--toolbar);
    backdrop-filter: blur(20px) saturate(170%);
    border-bottom: 1px solid var(--rule);
  }
  .title { flex: 1; min-width: 0; display: flex; align-items: baseline; gap: 12px; }
  h1 { margin: 0; font: 700 22px var(--font-display); letter-spacing: -0.015em; white-space: nowrap; }
  .origin { font-size: 13px; color: var(--ink-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 14px;
    border-radius: 9px;
    font-size: 13.5px;
    font-weight: 500;
    background: var(--well);
    box-shadow: inset 0 0 0 1px var(--rule);
    transition: background-color 120ms ease-out, transform 100ms ease-out;
  }
  .btn:hover { background: var(--rule); }
  .btn:active { transform: scale(0.97); }
  .btn.ghost { background: none; box-shadow: none; color: var(--accent-text); }
  .btn.ghost:hover { background: var(--accent-wash); }
  .btn.primary { color: #fff; background: var(--accent); box-shadow: inset 0 -1px 0 rgba(0, 0, 0, 0.15); }
  .btn.primary:hover { background: var(--accent-press); }
  .btn:disabled { opacity: 0.6; }

  .page { max-width: 940px; margin: 0 auto; padding: 24px 0 80px; display: flex; flex-direction: column; gap: 22px; }
  .card { padding: 14px 16px; border-radius: 14px; display: flex; flex-direction: column; gap: 6px; }
  .summary { background: var(--warn-wash); color: var(--warn); }
  .summary.clear { background: color-mix(in srgb, var(--ok) 12%, var(--page)); color: var(--ok); }
  .head { display: flex; align-items: center; gap: 8px; }
  .head strong { color: var(--ink); font-size: 15px; font-weight: 600; }
  .method { margin: 0; font-size: 13px; color: var(--ink-2); }
  .link { margin-left: 8px; color: var(--accent-text); font-size: 13px; }
  .link:hover { text-decoration: underline; }
  .tried { margin: 0; padding-left: 18px; font-size: 12.5px; color: var(--ink-2); }
  .bad { margin: 0; font-size: 13px; color: var(--heart); }
  .issue { display: flex; align-items: center; gap: 6px; margin: 0; font-size: 13px; }
  .dup { background: var(--accent-wash); }
  .dup strong { font-size: 15px; }
  .radio { display: flex; gap: 9px; align-items: flex-start; font-size: 14px; }
  .radio input { margin-top: 3px; accent-color: var(--accent); }
  .fixed { background: color-mix(in srgb, var(--ok) 10%, var(--page)); }
  .fixed ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 3px; font-size: 13.5px; }
  .fixed li { display: flex; gap: 6px; align-items: baseline; }
  .fixed li :global(svg) { color: var(--ok); flex: none; }
  .fine { margin: 0 0 8px; font-size: 12.5px; color: var(--ink-2); }

  .fields { display: grid; grid-template-columns: 2.4fr repeat(4, 1fr); gap: 10px; }
  .f { display: flex; flex-direction: column; gap: 4px; font-size: 12.5px; color: var(--ink-2); }
  input:not([type]), textarea {
    width: 100%;
    padding: 7px 10px;
    border: 0;
    border-radius: 9px;
    background: var(--well);
    box-shadow: inset 0 0 0 1px var(--rule);
    color: var(--ink);
    font: inherit;
    font-size: 14.5px;
    outline: none;
    -webkit-user-select: text;
    user-select: text;
    transition: background-color 150ms ease-out, box-shadow 150ms ease-out;
  }
  input:not([type]):focus, textarea:focus { background: var(--card); box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  textarea { resize: none; field-sizing: content; min-height: 2lh; line-height: 1.5; }
  .notes { min-height: 3lh; }

  .cats { display: flex; flex-direction: column; gap: 8px; padding: 10px 14px; border-radius: 12px; background: var(--well); }
  .cats[open] { padding-bottom: 14px; }
  summary { display: flex; align-items: baseline; gap: 12px; list-style: none; cursor: default; }
  summary::-webkit-details-marker { display: none; }
  summary .caps { font-size: 15px; color: var(--ink-2); }
  .chosen { flex: 1; min-width: 0; color: var(--ink); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .open-hint { font-size: 13px; color: var(--accent-text); }
  .cats[open] .open-hint { visibility: hidden; }
  .cats[open] summary { margin-bottom: 6px; }
  .chips { display: flex; flex-wrap: wrap; gap: 5px; }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 10px 0 6px;
    border-radius: 13px;
    font-size: 12.5px;
    background: var(--well);
    box-shadow: inset 0 0 0 1px var(--rule);
    transition: background-color 160ms ease-out;
  }
  .chip:not(.sub) { font-weight: 600; margin-right: 4px; }
  .chip:active { transform: scale(0.96); }
  .tick { display: grid; place-items: center; width: 14px; height: 14px; border-radius: 50%; color: transparent; box-shadow: inset 0 0 0 1.5px var(--l); }
  .chip.on { background: color-mix(in srgb, var(--l) 14%, var(--card)); }
  .chip.on .tick { background: var(--l); color: #fff; }
  .tick :global(svg) { stroke-width: 2.8; }

  .only { display: flex; align-items: center; gap: 8px; font-size: 14px; }
  .only input { accent-color: var(--accent); width: 16px; height: 16px; margin: 0; }
  h2 { margin: 0 0 6px; font-size: 17px; color: var(--ink-2); }
  em { font-style: normal; margin-left: 4px; color: var(--ink-3); }
  .lines { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .lines li { display: flex; align-items: flex-start; gap: 10px; padding: 6px 8px; border-radius: 10px; }
  .lines li.flag { background: var(--warn-wash); }
  .dot { display: grid; place-items: center; width: 20px; height: 34px; color: var(--ok); flex: none; }
  .flag .dot { color: var(--warn); }
  .low .dot, .low .why { color: var(--heart); }
  .line { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .parsed { font-size: 12.5px; color: var(--ink-2); padding-left: 2px; }
  .why { display: flex; align-items: center; gap: 5px; font-size: 12.5px; color: var(--warn); padding-left: 2px; }
  .n { display: grid; place-items: center; width: 26px; height: 34px; font-size: 17px; color: var(--ink-2); flex: none; }
  .tools { display: flex; flex-direction: column; gap: 2px; }
  .x { display: grid; place-items: center; width: 30px; height: 30px; border-radius: 8px; color: var(--ink-3); flex: none; margin-top: 2px; }
  .x:hover { background: var(--rule); color: var(--ink); }
  .add { display: inline-flex; align-items: center; gap: 6px; margin-top: 8px; height: 30px; padding: 0 10px; border-radius: 8px; color: var(--accent-text); font-size: 13.5px; font-weight: 500; }
  .add:hover { background: var(--accent-wash); }

  @media (max-width: 860px) {
    .fields { grid-template-columns: 1fr 1fr; }
    .fields .wide { grid-column: 1 / -1; }
  }
</style>
