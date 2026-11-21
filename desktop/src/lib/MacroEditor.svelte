<script lang="ts">
  import type { MacroMode, MacroResult, MacroSpec, MassUnit, PresetOption } from './api';
  import { EMPTY } from './targets.svelte';

  // The three macro rows, shared by the default plan and the template dialog so both follow one set of rules (R-04).
  // Every readout, total and message is the crate's; nothing here works out grams, percent or kcal.
  let { spec = $bindable(), result, ready, presets, unit, id, onchange }: {
    spec: MacroSpec;
    /** the crate's answer for this plan; null before it has one */
    result: MacroResult | null;
    /** a calorie target exists, so the readouts mean something */
    ready: boolean;
    presets: PresetOption[];
    /** the unit Grams per kg values are written in */
    unit: MassUnit;
    /** prefix for element ids, so the page can move focus to a row */
    id: string;
    onchange: () => void;
  } = $props();

  type Key = 'protein' | 'carbs' | 'fat';
  const rows: { key: Key; name: string; dot: 'p' | 'c' | 'f'; i: number }[] = [
    { key: 'protein', name: 'Protein', dot: 'p', i: 0 },
    { key: 'carbs', name: 'Carbs (net)', dot: 'c', i: 1 },
    { key: 'fat', name: 'Fat', dot: 'f', i: 2 },
  ];

  const perKg = $derived(unit === 'lb' ? 'Grams per lb' : 'Grams per kg');
  const options = (k: Key): [string, string][] => [
    ['grams', 'Grams'],
    ['percent', 'Percent of calories'],
    k === 'protein' ? ['per_kg', perKg] : ['rest', 'Fills the rest'],
  ];
  const other = (k: Key): MacroMode => (k === 'carbs' ? spec.fat : spec.carbs);
  const suffix = (m: MacroMode) => (m.mode === 'grams' ? 'g' : m.mode === 'percent' ? '%' : unit === 'lb' ? 'g/lb' : 'g/kg');
  const shown = (m: MacroMode) => (m.mode === 'rest' || m.value === EMPTY ? '' : m.value);

  // a new mode starts with an empty box: the old number meant something else (grams are not percent)
  function setMode(k: Key, mode: string) {
    spec[k] = mode === 'rest' ? { mode: 'rest' } : { mode: mode as 'grams' | 'percent' | 'per_kg', value: EMPTY };
    onchange();
  }
  function setValue(k: Key, e: Event) {
    const m = spec[k];
    if (m.mode === 'rest') return;
    const n = (e.currentTarget as HTMLInputElement).valueAsNumber;
    spec[k] = { mode: m.mode, value: Number.isFinite(n) ? n : EMPTY };
    onchange();
  }
  /** a preset fills the three boxes and locks nothing (D-10); its numbers are the crate's */
  function usePreset(p: PresetOption) {
    spec = $state.snapshot(p.spec) as MacroSpec;
    onchange();
  }

  const texts = $derived([...new Set((result?.problems ?? []).map((p) => p.text))]);
  const msgId = (text: string | null | undefined) => (text ? `${id}-msg-${texts.indexOf(text)}` : undefined);
</script>

<div class="macros">
  <p class="fine">Carbs are compared with net carbs eaten (carbs minus fiber).</p>

  <div class="presets" role="group" aria-label="Start from a preset">
    <span class="lab">Start from a preset</span>
    {#each presets as p (p.name)}
      <button type="button" class="btn" onclick={() => usePreset(p)}>{p.name}</button>
    {/each}
  </div>
  <p class="fine">A preset fills in the boxes. You can change any of them.</p>

  <div class="rows">
    {#each rows as r (r.key)}
      {@const m = spec[r.key]}
      {@const row = ready ? (result?.rows[r.i] ?? null) : null}
      <div class="row" id="{id}-{r.key}">
        <span class="nm"><i class="dot {r.dot}" aria-hidden="true"></i>{r.name}</span>
        <select
          class="mode"
          aria-label="{r.name.replace(' (net)', '')} mode"
          aria-invalid={row?.problem ? 'true' : undefined}
          aria-describedby={msgId(row?.problem)}
          value={m.mode}
          onchange={(e) => setMode(r.key, e.currentTarget.value)}
        >
          {#each options(r.key) as [value, label] (value)}
            <option {value} disabled={value === 'rest' && other(r.key).mode === 'rest'} title={value === 'rest' && other(r.key).mode === 'rest' ? 'Only one of carbs and fat can fill the rest.' : undefined}>{label}</option>
          {/each}
        </select>
        <span class="val">
          {#if m.mode !== 'rest'}
            <input
              class="v"
              type="number"
              min="0"
              step="any"
              inputmode="decimal"
              autocomplete="off"
              aria-label="{r.name.replace(' (net)', '')} amount"
              aria-invalid={row?.problem ? 'true' : undefined}
              aria-describedby={msgId(row?.problem)}
              value={shown(m)}
              onchange={(e) => setValue(r.key, e)}
            />
            <span class="fine">{suffix(m)}</span>
          {/if}
        </span>
        <span class="readout fine num">{row?.readout ?? ''}</span>
      </div>
    {/each}
  </div>

  <div class="total" aria-live="polite">
    {#if ready && result}<p class="fine num">{result.total_text}</p>{/if}
    {#each texts as t, n (t)}<p id="{id}-msg-{n}" class="fine warn">{t}</p>{/each}
  </div>
</div>

<style>
  .macros { container-type: inline-size; display: flex; flex-direction: column; gap: 16px; }
  .fine { margin: 0; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .fine.warn { color: var(--warn); }
  .lab { font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .presets { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .presets .lab { margin-right: 8px; }
  .rows { display: flex; flex-direction: column; }
  .row { display: grid; grid-template-columns: 104px 176px auto minmax(0, 1fr); align-items: center; gap: 8px 16px; min-height: 56px; border-bottom: 1px solid var(--rule); }
  .nm { display: flex; align-items: center; gap: 8px; font-weight: 600; white-space: nowrap; }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex: none; }
  .dot.p { background: var(--protein); }
  .dot.c { background: var(--carbs); }
  .dot.f { background: var(--fat); }
  .val { display: flex; align-items: center; gap: 8px; min-width: 96px; }
  .readout { text-align: right; min-width: 0; }
  .total { display: flex; flex-direction: column; gap: 4px; }
  .total:empty { display: none; }

  /* the one field look (the dialog sheet styles its own inputs; these selectors out-rank it) */
  .row select.mode, .row .val input.v {
    height: 36px;
    padding: 0 8px;
    border: 0;
    border-radius: 9px;
    background: var(--well);
    box-shadow: inset 0 0 0 1px var(--rule);
    color: var(--ink);
    font: inherit;
    font-size: 14.5px;
    outline: none;
  }
  .row select.mode { width: 176px; } /* fits "Percent of calories" */
  .row .val input.v { width: 96px; }
  .row select.mode:focus, .row .val input.v:focus { box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  .row select.mode[aria-invalid='true'], .row .val input.v[aria-invalid='true'] { box-shadow: inset 0 0 0 1px var(--warn); }

  /* under 640 px of its own width each row stacks: name, then mode and value, then the readout */
  @container (max-width: 639px) {
    .row { grid-template-columns: auto auto minmax(0, 1fr); padding: 8px 0; }
    .nm { grid-column: 1 / -1; }
    .readout { grid-column: 1 / -1; text-align: left; }
  }
</style>
