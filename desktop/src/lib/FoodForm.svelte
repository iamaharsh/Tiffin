<script lang="ts">
  import { tick, untrack } from 'svelte';
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';
  import OffEmailDialog from './OffEmailDialog.svelte';
  import * as api from './api';
  import type { FoodPreview, MyFood, Nutrients } from './api';

  // The My Food form. A box left empty is unknown, not 0: its key is simply left out of what is sent.
  // What blocks a save is decided by the crate (foods::check_food, carried by api.foodPreview); the form only shows its answer.
  let { food, onsaved, onclose, onerror, ondelete }: {
    food: MyFood;
    onsaved: (f: MyFood) => void;
    onclose: () => void;
    onerror: (m: string) => void;
    /** called after the food was deleted (the form's Delete Food, once confirmed) */
    ondelete?: (f: MyFood) => void;
  } = $props();

  type Num = number | null | undefined;
  type Draft = Omit<MyFood, 'serving_g' | 'serving_ml' | 'piece_g' | 'per_serving'> & { serving_g: Num; serving_ml: Num; piece_g: Num; per_serving: Nutrients };
  const boxes: [keyof Nutrients, string, boolean][] = [
    ['calories', 'Calories', true], ['protein_g', 'Protein (g)', false], ['carbs_g', 'Carbs (g)', false], ['fat_g', 'Fat (g)', false],
    ['fiber_g', 'Fiber (g)', false], ['sugar_g', 'Sugar (g)', false], ['sodium_mg', 'Sodium (mg)', false],
  ];

  const pos = (n: number) => (n > 0 ? n : undefined);
  const toDraft = (f: MyFood): Draft => ({ ...f, serving_g: pos(f.serving_g), serving_ml: pos(f.serving_ml), piece_g: pos(f.piece_g), per_serving: { ...f.per_serving } });
  /** What gets sent: an empty box is a missing key, never 0. */
  function payload(d: Draft): MyFood {
    const per: Nutrients = {};
    for (const [k] of boxes) if (typeof d.per_serving[k] === 'number') per[k] = d.per_serving[k];
    const g = (v: Num) => (typeof v === 'number' ? v : 0); // the three plain numbers have no "unknown"; 0 means none
    return { ...d, serving_g: g(d.serving_g), serving_ml: g(d.serving_ml), piece_g: g(d.piece_g), per_serving: per };
  }

  let draft: Draft = $state(untrack(() => toDraft(food)));
  let lookup = $state('');
  let busy = $state(false);
  let saving = false;
  /** what the last link/barcode/photo filled in; warn = check something by hand */
  let labelNote: { text: string; warn: boolean } | null = $state(null);
  let labelFile: HTMLInputElement | undefined = $state();
  let formEl: HTMLFormElement | undefined = $state();
  let preview = $state<FoodPreview | null>(null);
  /** the crate's problem shows only after Save Food was pressed once */
  let attempted = $state(false);
  const problem = $derived(attempted ? (preview?.problem ?? null) : null);
  let seq = 0;
  let askDiscard = $state(false);
  let askDelete = $state(false);
  /** a barcode lookup needs a contact email first; the lookup is kept to run again once it is saved */
  let askEmail: (() => Promise<api.LabelGuess>) | null = $state(null);
  const start = untrack(() => JSON.stringify(payload(draft)));
  const dirty = $derived(JSON.stringify(payload(draft)) !== start);

  // the crate works out per 100 g, the two warnings and the problem; the form shows what it returns
  async function ask(body: MyFood) {
    const mine = ++seq; // a slower older answer never replaces a newer one
    const p = await api.foodPreview(body).catch(() => null); // the preview is optional; without it the form shows the fallback line
    if (mine === seq) preview = p;
  }
  $effect(() => {
    const body = payload(draft);
    untrack(() => ask(body));
  });

  /** Esc and the backdrop ask first when something was typed; the Discard Changes button is its own confirmation. */
  function requestClose() {
    if (dirty) askDiscard = true;
    else onclose();
  }
  async function remove() {
    askDelete = false;
    try {
      await api.deleteMyFood(food.id);
      ondelete?.(food);
    } catch (e) {
      onerror(String(e));
    }
  }

  /** Pressing Save Food: the crate's answer first, then the save (the command checks again). */
  async function save() {
    if (saving) return;
    saving = true;
    try {
      const body = payload(draft);
      const p = await api.foodPreview(body);
      // the crate decides what blocks a save; the form only shows its answer
      ++seq;
      preview = p;
      if (p.problem) {
        attempted = true;
        await tick();
        formEl?.querySelector<HTMLElement>(`[name="${p.problem.field}"]`)?.focus();
        return;
      }
      await api.saveMyFood(body);
      onsaved(body);
    } catch (e) {
      onerror(String(e));
    } finally {
      saving = false;
    }
  }

  /** Fill the form from a product link, a barcode or a label photo. */
  /** `byLookup`: the numbers came from a barcode or link, so the food's barcode becomes that one (or none); a label photo keeps it */
  async function fill(job: () => Promise<api.LabelGuess>, byLookup = false) {
    labelNote = { text: 'Looking it up…', warn: false };
    busy = true;
    try {
      const g = await job();
      if (!draft.name.trim()) draft.name = g.food.name;
      draft.serving = g.food.serving;
      if (g.food.serving_g > 0) draft.serving_g = g.food.serving_g;
      draft.serving_ml = pos(g.food.serving_ml);
      draft.per_serving = { ...g.food.per_serving };
      draft.source = g.source;
      // numbers from a lookup belong to its barcode (none for a link), never to the one the food had
      if (byLookup) draft.barcode = g.food.barcode;
      let text = `Filled in from ${g.source}. Check the numbers against the label, then Save Food.`;
      if (g.missing.length) text += ` Not found: ${g.missing.join(', ')} (left blank).`;
      if (g.food.serving_g <= 0) text += ' Type in the serving’s weight in grams.';
      for (const c of g.check) text += ` ${c}`;
      labelNote = { text, warn: g.missing.length > 0 || g.food.serving_g <= 0 || g.check.length > 0 };
    } catch (e) {
      if (String(e) === api.NEEDS_EMAIL) {
        labelNote = null; // nothing was sent
        askEmail = job;
      } else labelNote = { text: String(e), warn: true };
    } finally {
      busy = false;
    }
  }
  async function labelPhoto(e: Event) {
    const file = (e.currentTarget as HTMLInputElement).files?.[0];
    (e.currentTarget as HTMLInputElement).value = '';
    if (file) await fill(() => api.labelPhoto(file));
  }

  /** aria attributes for the box the crate named */
  const bad = (field: string) => (problem?.field === field ? ({ 'aria-invalid': 'true', 'aria-describedby': 'ff-err' } as const) : {});
</script>

{#snippet err(field: string)}
  {#if problem?.field === field}<p class="fine warn err" id="ff-err"><Icon name="warn" size={14} />{problem.message}</p>{/if}
{/snippet}

<Dialog wide title={food.id ? 'Edit Food' : 'Add a Food'} onclose={requestClose}>
  <!-- the fields scroll, the buttons stay in view on a short window -->
  <div class="scroll">
  <form class="fill" onsubmit={(e) => { e.preventDefault(); if (lookup.trim()) fill(() => api.labelLookup(lookup), true); }}>
    <label class="label" for="food-lookup">Fill in from a product link or barcode</label>
    <span class="fillrow">
      <input id="food-lookup" name="lookup" bind:value={lookup} placeholder="https://… or 0123456789012" spellcheck="false" autocomplete="off" inputmode="url" />
      <button class="btn" disabled={!lookup.trim() || busy}>Look Up</button>
      {#if !api.isWeb}<button type="button" class="btn" disabled={busy} onclick={() => labelFile?.click()}>Read a Label Photo…</button>{/if}
    </span>
    {#if !api.isWeb}<input bind:this={labelFile} type="file" accept="image/*" hidden onchange={labelPhoto} />{/if}
    <p class="fine" class:warn={labelNote?.warn} role="status">{labelNote?.text ?? ''}</p>
    {#if draft.barcode}
      <p class="fine barcode">Barcode {draft.barcode}. Once saved, scanning it finds this food.
        <button type="button" class="btn ghost" onclick={() => (draft.barcode = '')}>Remove Barcode</button>
      </p>
    {/if}
  </form>

  <form id="food-form" class="ff" novalidate bind:this={formEl} onsubmit={(e) => { e.preventDefault(); save(); }}>
    <div class="f">
      <label for="ff-name">Name <span class="req" aria-hidden="true">required</span></label>
      <input id="ff-name" name="name" bind:value={draft.name} placeholder="Vanilla protein powder" autocomplete="off" aria-required="true" {...bad('name')} />
      {@render err('name')}
    </div>
    <div class="f">
      <label for="ff-aliases">Other names recipes use (comma separated)</label>
      <input id="ff-aliases" name="aliases" bind:value={draft.aliases} placeholder="protein powder, whey" autocomplete="off" />
    </div>
    <div class="row">
      <div class="f">
        <label for="ff-serving">Serving name</label>
        <input id="ff-serving" name="serving" bind:value={draft.serving} placeholder="1 scoop" autocomplete="off" />
      </div>
      <div class="f">
        <label for="ff-serving_g">Serving weight (g) <span class="req" aria-hidden="true">required</span></label>
        <input id="ff-serving_g" name="serving_g" type="number" min="0.1" step="any" inputmode="decimal" autocomplete="off" bind:value={draft.serving_g} aria-required="true" {...bad('serving_g')} />
        {@render err('serving_g')}
      </div>
      <div class="f">
        <label for="ff-serving_ml">Volume (ml)</label>
        <input id="ff-serving_ml" name="serving_ml" type="number" min="0" step="any" inputmode="decimal" autocomplete="off" bind:value={draft.serving_ml} {...bad('serving_ml')} />
        {@render err('serving_ml')}
      </div>
    </div>
    <div class="row">
      <div class="f">
        <label for="ff-piece">Piece name</label>
        <input id="ff-piece" name="piece" bind:value={draft.piece} placeholder="slice" autocomplete="off" {...bad('piece')} />
        {@render err('piece')}
      </div>
      <div class="f">
        <label for="ff-piece_g">Piece weight (g)</label>
        <input id="ff-piece_g" name="piece_g" type="number" min="0" step="any" inputmode="decimal" autocomplete="off" bind:value={draft.piece_g} {...bad('piece_g')} />
        {@render err('piece_g')}
      </div>
    </div>
    <p class="fine">Optional. Lets you measure this food in pieces, such as 1 slice = 28 g.</p>

    <p class="fine">Per serving, as printed on the label. Leave a box empty if the label doesn't list it; empty means unknown, not zero.</p>
    <div class="nut">
      {#each boxes as [k, label, req] (k)}
        <div class="f">
          <label for="ff-{k}">{label}{#if req} <span class="req" aria-hidden="true">required</span>{/if}</label>
          <input id="ff-{k}" name={k} type="number" min="0" step="any" inputmode="decimal" autocomplete="off" placeholder={req ? undefined : '–'} bind:value={draft.per_serving[k]} aria-required={req ? 'true' : undefined} {...bad(k)} />
          {@render err(k)}
        </div>
      {/each}
    </div>
    <p class="fine">
      {#if preview?.per100}
        Works out to {preview.per100.calories ?? '–'} kcal per 100 g · P {preview.per100.protein_g ?? '–'} · C {preview.per100.carbs_g ?? '–'} · F {preview.per100.fat_g ?? '–'}
      {:else}
        Enter the serving weight to see the per-100 g values.
      {/if}
    </p>
    {#if preview?.heavy}<p class="fine warn">Protein, carbs and fat add up to more than the serving weighs. Check the numbers.</p>{/if}
    {#if preview?.dense}<p class="fine warn">That is a lot of calories for one serving. Check the label.</p>{/if}
  </form>
  </div>

  {#snippet actions()}
    {#if food.id && ondelete}<button type="button" class="btn danger" style="margin-right:auto" onclick={() => (askDelete = true)}>Delete Food</button>{/if}
    <button type="button" class="btn" onclick={onclose}>Discard Changes</button>
    <button type="submit" form="food-form" class="btn primary">Save Food</button>
  {/snippet}
</Dialog>

{#if askEmail}
  <OffEmailDialog onsaved={() => { const again = askEmail!; askEmail = null; fill(again, true); }} oncancel={() => (askEmail = null)} />
{/if}

{#if askDiscard}
  <Dialog title="Discard changes?" onclose={() => (askDiscard = false)}>
    <p class="msg">What you typed in this food hasn't been saved.</p>
    {#snippet actions()}
      <button type="button" class="btn" onclick={() => (askDiscard = false)}>Keep Editing</button>
      <button type="button" class="btn danger" onclick={onclose}>Discard Changes</button>
    {/snippet}
  </Dialog>
{/if}

{#if askDelete}
  <Dialog title="Delete Food" onclose={() => (askDelete = false)}>
    <p class="msg">Delete “{food.name}”? Recipes that use it are re-estimated from the food database. This can't be undone.</p>
    {#snippet actions()}
      <button type="button" class="btn" onclick={() => (askDelete = false)}>Keep Food</button>
      <button type="button" class="btn danger" onclick={remove}>Delete Food</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .msg { margin: 0; }
  .ff { display: flex; flex-direction: column; gap: 12px; }
  .fill { display: flex; flex-direction: column; gap: 6px; padding-bottom: 14px; border-bottom: 1px solid var(--rule); }
  .fillrow { display: flex; flex-wrap: wrap; gap: 8px; }
  .barcode { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 8px; margin: 0; }
  .fillrow input { flex: 1 1 220px; min-width: 0; }
  .f { display: flex; flex-direction: column; gap: 5px; min-width: 0; }
  .f > label { font-size: 12.5px; }
  .req { margin-left: 4px; color: var(--ink-3); }
  .row { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 10px; }
  .nut { display: grid; grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)); gap: 10px; }
  .warn { color: var(--warn); }
  .err { display: inline-flex; gap: 4px; align-items: center; }
  .scroll { display: flex; flex-direction: column; gap: 12px; max-height: calc(100vh - 210px); overflow-y: auto; padding: 4px; margin: -4px; }
  input[aria-invalid='true']:not(:focus) { box-shadow: inset 0 0 0 1.5px var(--warn); }
</style>
