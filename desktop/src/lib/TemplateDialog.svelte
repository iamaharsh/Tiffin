<script lang="ts">
  import { tick, untrack } from 'svelte';
  import Dialog from './Dialog.svelte';
  import MacroEditor from './MacroEditor.svelte';
  import * as api from './api';
  import type { MacroSpec, PresetOption, TargetsDraft, TargetsResult, Template } from './api';
  import { BAD_OFFSET, nextTemplateId, putTemplate } from './targets.svelte';

  // New Template / Edit Template: a name, a calorie change from the base target, and its own macros. Saving changes the
  // page's draft only; Save Targets writes it. Esc and a click outside are Discard Edits. The crate does every sum:
  // the day's number, the readouts and the messages come from a preview of the page's draft with this template in it.
  let { template, draft, presets, today, onsave, onremove, onclose, onerror }: {
    /** the template to edit, or null for a new one */
    template: Template | null;
    draft: TargetsDraft;
    presets: PresetOption[];
    today: string;
    onsave: (t: Template) => void;
    onremove: (id: number) => void;
    onclose: () => void;
    onerror: (m: string) => void;
  } = $props();

  const first = untrack(() => template);
  const adding = first === null;
  const was = first?.name ?? '';
  // a new template starts from the Balanced preset, like the crate's own default
  let t = $state(
    untrack(() =>
      first
        ? ($state.snapshot(first) as Template)
        : ({ id: nextTemplateId(draft.templates), name: '', kcal_offset: 0, macros: $state.snapshot(presets[0].spec) as MacroSpec } as Template),
    ),
  );
  let offset = $state(first ? first.kcal_offset : (0 as number | null));
  let res = $state(null as TargetsResult | null);
  let attempted = $state(false);
  let touchedName = $state(false);
  let touchedOffset = $state(false);
  let confirming = $state(false);
  let seq = 0;

  const row = $derived(res?.templates.find((x) => x.id === t.id) ?? null);
  const own = (suffix: string) => res?.problems.find((p) => p.field === `template ${t.id} ${suffix}`)?.text ?? '';
  const nameMsg = $derived(attempted || touchedName ? own('name') : '');
  const offsetMsg = $derived(attempted || touchedOffset ? own('offset') : '');

  /** ask the crate what the page's draft works out to with this template in it */
  async function refresh() {
    const mine = ++seq; // a slower older answer never replaces a newer one
    const d = $state.snapshot(draft) as TargetsDraft;
    putTemplate(d, $state.snapshot(t) as Template);
    try {
      const r = await api.targetsPreview(d, today);
      if (mine === seq) res = r;
    } catch (e) {
      if (mine === seq) onerror(String(e));
    }
  }
  $effect(() => {
    untrack(refresh);
  });

  function setOffset(e: Event) {
    const n = (e.currentTarget as HTMLInputElement).valueAsNumber;
    offset = Number.isFinite(n) ? n : null;
    touchedOffset = true;
    // an empty or fractional box can't be a whole number of kcal: the crate answers with its own sentence
    t.kcal_offset = offset !== null && Number.isInteger(offset) ? offset : BAD_OFFSET;
    refresh();
  }

  async function save() {
    attempted = true;
    await refresh(); // judge the newest answer, not one still on its way
    const mine = res?.problems.filter((p) => p.field.startsWith(`template ${t.id} `)) ?? [];
    if (mine.length) {
      await tick();
      const at = mine.some((p) => p.field.endsWith(' name')) ? 'tpl-name' : mine.some((p) => p.field.endsWith(' offset')) ? 'tpl-offset' : `tpl-m-${mine[0].field.split(' ')[2]}`;
      const el = document.getElementById(at);
      (el?.matches('input, select') ? el : el?.querySelector<HTMLElement>('input, select'))?.focus();
      return;
    }
    onsave($state.snapshot(t) as Template);
  }
</script>

<Dialog title={adding ? 'New Template' : 'Edit Template'} wide {onclose}>
  <form id="tpl-form" onsubmit={(e) => { e.preventDefault(); save(); }}>
    <label class="f">
      <span>Name</span>
      <input
        id="tpl-name"
        maxlength="40"
        autocomplete="off"
        bind:value={t.name}
        onchange={() => { touchedName = true; refresh(); }}
        aria-invalid={nameMsg ? 'true' : undefined}
        aria-describedby={nameMsg ? 'tpl-name-msg' : undefined}
      />
    </label>
    {#if nameMsg}<p id="tpl-name-msg" class="note warn" role="status">{nameMsg}</p>{/if}

    <label class="f">
      <span>Calories compared with your base target</span>
      <span class="off">
        <input
          id="tpl-offset"
          class="num-in"
          type="number"
          step="25"
          inputmode="numeric"
          autocomplete="off"
          value={offset ?? ''}
          onchange={setOffset}
          aria-invalid={offsetMsg ? 'true' : undefined}
          aria-describedby={offsetMsg ? 'tpl-offset-msg' : undefined}
        />
        <span class="note num">{row?.day_text ?? ''}</span>
      </span>
    </label>
    {#if offsetMsg}<p id="tpl-offset-msg" class="note warn" role="status">{offsetMsg}</p>{/if}

    <MacroEditor id="tpl-m" bind:spec={t.macros} result={row?.macros ?? null} ready={row !== null} {presets} unit={draft.per_kg_unit} onchange={refresh} />
  </form>
  {#snippet actions()}
    {#if !adding}<button type="button" class="btn ghost del" onclick={() => (confirming = true)}>Delete Template</button>{/if}
    <button type="button" class="btn" onclick={onclose}>Discard Edits</button>
    <button type="submit" form="tpl-form" class="btn primary">Save Template</button>
  {/snippet}
</Dialog>

{#if confirming}
  <Dialog title="Delete {was}?" onclose={() => (confirming = false)}>
    <p class="note">Days that use it will go back to your default plan. Earlier days keep it.</p>
    {#snippet actions()}
      <button type="button" class="btn" onclick={() => (confirming = false)}>Keep Template</button>
      <button type="button" class="btn danger" onclick={() => onremove(t.id)}>Delete Template</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  form { display: flex; flex-direction: column; gap: 16px; }
  .f { display: flex; flex-direction: column; gap: 4px; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .off { display: flex; align-items: center; gap: 16px; }
  p.note, .note { margin: 0; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  p.note.warn { color: var(--warn); }
  /* more specific than the sheet's own field rules, which share their specificity */
  form .f input { font-size: 14.5px; }
  form .f .off input.num-in { width: 120px; }
  form .f input[aria-invalid='true'] { box-shadow: inset 0 0 0 1px var(--warn); }
  .btn.ghost.del { margin-right: auto; color: var(--heart); }
  .btn.ghost.del:hover { background: color-mix(in srgb, var(--heart) 10%, transparent); text-decoration: none; }
</style>
