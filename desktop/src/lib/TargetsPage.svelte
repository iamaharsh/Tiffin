<script lang="ts">
  import { tick, untrack } from 'svelte';
  import Icon from './Icon.svelte';
  import Segmented from './Segmented.svelte';
  import MacroEditor from './MacroEditor.svelte';
  import TemplateDialog from './TemplateDialog.svelte';
  import * as api from './api';
  import type { GoalKind, HeightUnit, MassUnit, Sex, TargetSource, TargetsDraft, TargetsResult, TargetsView, Template } from './api';
  import { adoptSaved, discard, dropTemplate, putTemplate, targets, targetsDirty } from './targets.svelte';

  // The Targets page prints the crate's answers and does no maths: every figure, unit conversion and sentence below
  // comes from the crate (no rounding, no kcal arithmetic here). Edits live in a draft module until Save Targets.
  let { onerror, onnote }: {
    onerror: (m: string) => void;
    onnote: (m: string, action?: { label: string; run: () => void } | null) => void;
  } = $props();

  let today = $state(api.localDay());
  let view = $state(null as TargetsView | null);
  let result = $state(null as TargetsResult | null);
  let busy = $state(false);
  let loadError = $state('');
  let saving = false;
  let saveError = $state('');
  /** the crate's question about a weight that looks wrong; the primary reads Save Anyway until the weight is edited again */
  let confirm = $state('');
  /** the status after a save, read out politely */
  let savedNote = $state(false);
  /** Save Targets was pressed while it could not save: every problem shows, not only those of fields already touched */
  let attempted = $state(false);
  let announce = $state('');
  let touched: string[] = $state([]);
  let seq = 0;
  /** a unit picker changed: the next newest answer carries the converted figures for that field */
  const adopting = new Set<'height' | 'weight' | 'rate' | 'goal_weight'>();

  const d = $derived(targets.draft);
  const locked = $derived(view?.read_only ?? '');

  async function load() {
    const mine = ++seq; // a slower older answer never replaces a newer one
    busy = true;
    try {
      const v = await api.targetsView(today);
      if (mine !== seq) return;
      view = v;
      loadError = '';
      if (targetsDirty()) recalc();
      else {
        adoptSaved(v.draft);
        result = v.result;
      }
    } catch (e) {
      if (mine === seq) loadError = api.reason(e);
      onerror(String(e));
    } finally {
      if (mine === seq) busy = false;
    }
  }
  $effect(() => {
    today;
    untrack(load);
  });

  /** ask the crate what the draft works out to; the earlier figures stay on screen until the answer comes */
  async function recalc() {
    const draft = targets.draft;
    if (!draft) return;
    const mine = ++seq;
    busy = true;
    try {
      const r = await api.targetsPreview($state.snapshot(draft) as TargetsDraft, today);
      if (mine !== seq) return;
      result = r;
      if (adopting.size) {
        const i = r.inputs;
        if (adopting.has('height')) draft.height = i.height;
        if (adopting.has('weight')) {
          draft.weight = i.weight;
          draft.macros = i.macros;
          draft.templates = i.templates;
          draft.per_kg_unit = i.per_kg_unit;
        }
        if (adopting.has('rate')) draft.rate = i.rate;
        if (adopting.has('goal_weight')) draft.goal_weight = i.goal_weight;
        adopting.clear();
      }
    } catch (e) {
      if (mine === seq) onerror(String(e));
    } finally {
      if (mine === seq) busy = false;
    }
  }

  /** a field was changed (leaving a box, picking a segment or a select), never on each keystroke */
  function commit(field?: string) {
    savedNote = false;
    saveError = '';
    announce = '';
    if (field && !touched.includes(field)) touched.push(field);
    recalc();
  }
  const numOf = (e: Event) => {
    const n = (e.currentTarget as HTMLInputElement).valueAsNumber;
    return Number.isFinite(n) ? n : null;
  };

  // each unit picker sets the field's display unit and asks the crate for the converted figure (D-02)
  function flipMass(k: 'weight' | 'rate' | 'goal_weight', u: MassUnit) {
    if (!d || d.units[k] === u) return;
    d.units[k] = u;
    adopting.add(k);
    commit();
  }
  function flipHeight(u: HeightUnit) {
    if (!d || d.units.height === u) return;
    d.units.height = u;
    adopting.add('height');
    commit();
  }

  // the rate select offers the crate's presets in the rate's own unit, or Custom; a rate the presets don't hold reads as Custom
  const ratePresets = $derived((d?.rate.unit === 'lb' ? view?.rate_presets_lb : view?.rate_presets_kg) ?? []);
  let rateCustom = $state(false);
  const rateChoice = $derived(rateCustom || d?.rate.value == null || !ratePresets.includes(d.rate.value) ? 'custom' : String(d.rate.value));
  function pickRate(e: Event & { currentTarget: HTMLSelectElement }) {
    if (!d) return;
    if (e.currentTarget.value === 'custom') {
      rateCustom = true;
      return;
    }
    rateCustom = false;
    d.rate.value = Number(e.currentTarget.value);
    commit('rate');
  }

  // ---- problems ----
  const IDS: Record<string, string> = {
    sex: 'tg-sex', 'birth date': 'tg-birth', height: 'tg-height', weight: 'tg-weight', 'activity level': 'tg-activity', rate: 'tg-rate',
    'goal weight': 'tg-goal-weight', calories: 'tg-manual', 'add-back': 'tg-addback', protein: 'tg-m-protein', carbs: 'tg-m-carbs', fat: 'tg-m-fat',
    templates: 'tg-templates', week: 'tg-week',
  };
  const ORDER = Object.keys(IDS);
  const idFor = (field: string) => {
    const t = /^template (\d+)/.exec(field);
    return t ? `tg-tpl-${t[1]}` : IDS[field];
  };
  const rank = (field: string) => (ORDER.includes(field) ? ORDER.indexOf(field) : ORDER.length);
  /** the crate's sentence for a field, once the person has touched it or tried to save */
  const problemOf = (field: string) => (attempted || touched.includes(field) ? (result?.problems.find((p) => p.field === field)?.text ?? '') : '');
  const bad = (field: string) => (problemOf(field) ? 'true' : undefined);
  const why = (field: string, id: string) => (problemOf(field) ? id : undefined);
  function focusField(id: string | undefined) {
    const el = id ? document.getElementById(id) : null;
    const to = el?.matches('input, select, button') ? el : el?.querySelector<HTMLElement>('input:checked, [tabindex="0"], input, select, button');
    to?.focus();
  }

  async function save() {
    if (saving || !d || !result || locked) return;
    if (!result.can_save) {
      // stay where we are, say what is missing, and move to the first field in page order
      attempted = true;
      const first = [...result.problems].sort((a, b) => rank(a.field) - rank(b.field))[0];
      announce = first?.text ?? '';
      await tick();
      focusField(first ? idFor(first.field) : undefined);
      return;
    }
    saving = true;
    saveError = '';
    try {
      const r = await api.saveTargets($state.snapshot(d) as TargetsDraft, today, confirm !== '');
      if (r.confirm) {
        confirm = r.confirm;
        return;
      }
      confirm = '';
      view = r.view;
      result = r.view.result;
      adoptSaved(r.view.draft);
      attempted = false;
      touched = [];
      announce = '';
      savedNote = true;
      onnote('Targets saved. They apply from today.');
    } catch (e) {
      saveError = `Couldn't save your targets: ${api.reason(e)}. Nothing was changed, so try again.`;
      onerror(saveError);
    } finally {
      saving = false;
    }
  }

  /** Update Target: the only thing that moves the target when the trend weight drifts; it writes a new record from today */
  let accepting = false;
  async function accept() {
    if (accepting || locked) return;
    accepting = true;
    try {
      const v = await api.acceptTargetUpdate(today);
      view = v;
      if (targetsDirty()) {
        // the person's edits stay; only what is saved moves
        targets.saved = JSON.stringify(v.draft);
        recalc();
      } else {
        adoptSaved(v.draft);
        result = v.result;
      }
      onnote(`Target updated to ${v.result.kcal_text} kcal from today.`);
    } catch (e) {
      onerror(`Couldn't update your target: ${api.reason(e)}. Nothing was changed, so try again.`);
      load();
    } finally {
      accepting = false;
    }
  }

  // ---- the weekday plan: templates are edited in a dialog and change the draft only ----
  const DAYS = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];
  /** the template being edited (null: a new one) and the button to give focus back to */
  let editing = $state(null as { t: Template | null; opener: HTMLElement | null } | null);
  function closeTemplate() {
    const back = editing?.opener;
    editing = null;
    // a deleted template's row is gone: fall back to New Template
    tick().then(() => (back?.isConnected ? back : document.getElementById('tg-new-template'))?.focus());
  }
  function saveTemplate(t: Template) {
    if (!d) return;
    putTemplate(d, t);
    commit();
    closeTemplate();
  }
  function removeTemplate(id: number) {
    if (!d) return;
    dropTemplate(d, id);
    commit();
    closeTemplate();
  }

  const SEX: Record<string, string> = { male: 'Male', female: 'Female', other: 'Other' };
  const GOAL: Record<string, string> = { lose: 'Lose', maintain: 'Maintain', gain: 'Gain' };
  const MASS: MassUnit[] = ['kg', 'lb'];
  const LENGTH: HeightUnit[] = ['cm', 'ft_in'];
  const dirty = $derived(targetsDirty());

  /** back from a long absence (or past midnight): the page's today moves, and the figures follow */
  function visible() {
    if (document.visibilityState === 'visible') today = api.localDay();
  }
</script>

<svelte:document onvisibilitychange={visible} />

<div class="targets" class:dirty aria-busy={busy}>
  <header class="title"><h1>Targets</h1></header>

  {#if loadError}
    <div class="notice" role="alert">
      <Icon name="warn" size={16} />
      <p>Couldn't open your targets: {loadError}. Press Try Again, or restart Tiffin if it keeps happening.</p>
      <button type="button" class="btn ghost" onclick={load}>Try Again</button>
    </div>
  {/if}

  {#if view && d}
    {#if locked}
      <div class="notice" role="alert"><Icon name="warn" size={16} /><p>{locked}</p></div>
    {/if}
    {#if !view.has_target}
      <p class="setup">Tell Tiffin about you to work out a daily calorie target. Fill in About You below, or choose Use My Own Number.</p>
    {/if}

    <section aria-labelledby="tg-about">
      <h2 id="tg-about" class="caps">About You</h2>
      <div class="grid">
        <div class="field">
          <span class="lab" id="tg-sex-lab">Sex</span>
          <div id="tg-sex">
            <Segmented label="Sex" options={['male', 'female', 'other']} value={d.sex ?? ''} onchange={(v) => { d.sex = v as Sex; commit('sex'); }}>
              {#snippet item(o)}{SEX[o]}{/snippet}
            </Segmented>
          </div>
          {#if d.sex === 'other' && result?.other_note}<p class="fine">{result.other_note}</p>{/if}
          {#if problemOf('sex')}<p class="fine warn" role="status">{problemOf('sex')}</p>{/if}
        </div>

        <div class="field">
          <label class="lab" for="tg-birth">Birth date</label>
          <div class="inline">
            <input
              id="tg-birth"
              class="in"
              type="date"
              min="1900-01-01"
              max={today}
              value={d.birth_date ?? ''}
              aria-invalid={bad('birth date')}
              aria-describedby={why('birth date', 'tg-birth-msg')}
              onchange={(e) => { d.birth_date = e.currentTarget.value || null; commit('birth date'); }}
            />
            {#if result?.age_text}<span class="fine">{result.age_text}</span>{/if}
          </div>
          {#if problemOf('birth date')}<p id="tg-birth-msg" class="fine warn" role="status">{problemOf('birth date')}</p>{/if}
        </div>

        <div class="field">
          <span class="lab" id="tg-height-lab">Height</span>
          <div class="inline" role="group" aria-labelledby="tg-height-lab">
            {#if d.height.unit === 'cm'}
              <input id="tg-height" class="in w96" type="number" min="0" step="any" inputmode="decimal" autocomplete="off" aria-label="Height in centimetres"
                aria-invalid={bad('height')} aria-describedby={why('height', 'tg-height-msg')}
                value={d.height.cm ?? ''} onchange={(e) => { d.height.cm = numOf(e); commit('height'); }} />
              <span class="fine">cm</span>
            {:else}
              <input id="tg-height" class="in w96" type="number" min="0" step="any" inputmode="decimal" autocomplete="off" aria-label="Height, feet"
                aria-invalid={bad('height')} aria-describedby={why('height', 'tg-height-msg')}
                value={d.height.ft ?? ''} onchange={(e) => { d.height.ft = numOf(e); commit('height'); }} />
              <span class="fine">ft</span>
              <input class="in w96" type="number" min="0" step="any" inputmode="decimal" autocomplete="off" aria-label="Height, inches"
                aria-invalid={bad('height')} aria-describedby={why('height', 'tg-height-msg')}
                value={d.height.inches ?? ''} onchange={(e) => { d.height.inches = numOf(e); commit('height'); }} />
              <span class="fine">in</span>
            {/if}
            <Segmented label="Height unit" options={LENGTH} value={d.units.height} onchange={flipHeight}>
              {#snippet item(o)}{o === 'cm' ? 'cm' : 'ft/in'}{/snippet}
            </Segmented>
          </div>
          {#if problemOf('height')}<p id="tg-height-msg" class="fine warn" role="status">{problemOf('height')}</p>{/if}
        </div>

        <div class="field">
          <label class="lab" for="tg-weight">Weight</label>
          <div class="inline">
            <input
              id="tg-weight"
              class="in w96"
              type="number"
              min="0"
              step="any"
              inputmode="decimal"
              autocomplete="off"
              aria-invalid={bad('weight')}
              aria-describedby={why('weight', 'tg-weight-msg')}
              value={d.weight.value ?? ''}
              onchange={(e) => { d.weight.value = numOf(e); d.weight_edited = true; confirm = ''; commit('weight'); }}
            />
            <span class="fine">{d.weight.unit}</span>
            <Segmented label="Weight unit" options={MASS} value={d.units.weight} onchange={(u) => flipMass('weight', u)}>
              {#snippet item(o)}{o}{/snippet}
            </Segmented>
          </div>
          {#if problemOf('weight')}<p id="tg-weight-msg" class="fine warn" role="status">{problemOf('weight')}</p>{/if}
          <p class="fine">Saved as today's weigh-in. Your target follows your trend weight once there are enough weigh-ins.</p>
          {#if result?.weight_basis_text}<p class="fine">{result.weight_basis_text}</p>{/if}
        </div>

        <fieldset class="field full" id="tg-activity">
          <legend class="lab">Activity level</legend>
          <p class="fine">This is your ordinary day without workouts. Log workouts in the Diary; they're added on top.</p>
          <div class="radios">
            {#each view.activities as a (a.id)}
              <label class="opt" class:sel={d.activity === a.id}>
                <input type="radio" name="tg-activity" value={a.id} checked={d.activity === a.id} onchange={() => { d.activity = a.id; commit('activity level'); }} />
                <span class="txt"><span class="nm">{a.name}</span><span class="fine">{a.description}</span></span>
              </label>
            {/each}
          </div>
          {#if problemOf('activity level')}<p class="fine warn" role="status">{problemOf('activity level')}</p>{/if}
        </fieldset>

        <div class="field">
          <span class="lab" id="tg-goal-lab">Goal</span>
          <Segmented label="Goal" options={['lose', 'maintain', 'gain']} value={d.goal} onchange={(v) => { d.goal = v as GoalKind; commit(); }}>
            {#snippet item(o)}{GOAL[o]}{/snippet}
          </Segmented>
        </div>

        {#if d.goal !== 'maintain'}
          <div class="field">
            <label class="lab" for="tg-rate">Rate per week</label>
            <div class="inline">
              <select id="tg-rate" class="in" aria-invalid={bad('rate')} aria-describedby={why('rate', 'tg-rate-msg')} value={rateChoice} onchange={pickRate}>
                {#each ratePresets as p (p)}<option value={String(p)}>{p} {d.rate.unit}</option>{/each}
                <option value="custom">Custom</option>
              </select>
              {#if rateChoice === 'custom'}
                <input class="in w96" type="number" min="0" step="any" inputmode="decimal" autocomplete="off" aria-label="Custom rate per week" aria-invalid={bad('rate')}
                  value={d.rate.value ?? ''} onchange={(e) => { d.rate.value = numOf(e); commit('rate'); }} />
                <span class="fine">{d.rate.unit}</span>
              {/if}
              <Segmented label="Rate unit" options={MASS} value={d.units.rate} onchange={(u) => flipMass('rate', u)}>
                {#snippet item(o)}{o}{/snippet}
              </Segmented>
            </div>
            {#if problemOf('rate')}<p id="tg-rate-msg" class="fine warn" role="status">{problemOf('rate')}</p>{/if}
          </div>
        {/if}

        <div class="field">
          <label class="lab" for="tg-goal-weight">Goal weight (optional)</label>
          <div class="inline">
            <input id="tg-goal-weight" class="in w96" type="number" min="0" step="any" inputmode="decimal" autocomplete="off" aria-invalid={bad('goal weight')} aria-describedby={why('goal weight', 'tg-goal-weight-msg')}
              value={d.goal_weight.value ?? ''} onchange={(e) => { d.goal_weight.value = numOf(e); commit('goal weight'); }} />
            <span class="fine">{d.goal_weight.unit}</span>
            <Segmented label="Goal weight unit" options={MASS} value={d.units.goal_weight} onchange={(u) => flipMass('goal_weight', u)}>
              {#snippet item(o)}{o}{/snippet}
            </Segmented>
          </div>
          {#if problemOf('goal weight')}<p id="tg-goal-weight-msg" class="fine warn" role="status">{problemOf('goal weight')}</p>{/if}
          <p class="fine">Used for the forecast on the Body page. Leave it empty if you don't have one.</p>
        </div>
      </div>
    </section>

    <section aria-labelledby="tg-target">
      <h2 id="tg-target" class="caps">Calorie Target</h2>
      <div class="inline source">
        <Segmented label="Calorie target" options={['calculated', 'manual']} value={d.source} onchange={(v) => { d.source = v as TargetSource; commit(); }}>
          {#snippet item(o)}{o === 'manual' ? 'My Own Number' : 'Calculated'}{/snippet}
        </Segmented>
        {#if d.source === 'manual'}
          <input
            id="tg-manual"
            class="in w120"
            type="number"
            min="0"
            step="any"
            inputmode="decimal"
            autocomplete="off"
            aria-label="Your own number of calories a day"
            aria-invalid={bad('calories')}
            aria-describedby={why('calories', 'tg-manual-msg')}
            value={d.manual_kcal ?? ''}
            onchange={(e) => { d.manual_kcal = numOf(e); commit('calories'); }}
          />
          <span class="fine">kcal a day</span>
        {/if}
      </div>
      {#if problemOf('calories')}<p id="tg-manual-msg" class="fine warn" role="status">{problemOf('calories')}</p>{/if}
      <div class="card" aria-live="polite" aria-busy={busy}>
        {#if result?.ready}
          <p class="target"><span class="figure num">{result.kcal_text}</span><span class="fine">kcal a day</span></p>
          {#if view.in_use_text}<p class="fine">{view.in_use_text}</p>{/if}
          {#if result.calculated_text}<p class="fine ref">{result.calculated_text} <span class="caps">for reference</span></p>{/if}
          {#if result.lines.length}
            <dl>
              {#each result.lines as l}
                <div><dt class="fine">{l.label}</dt><dd class="num">{l.value}</dd></div>
              {/each}
            </dl>
          {/if}
          {#each result.adjustments as a}
            <div class="adjust" role="status">
              <Icon name="warn" size={16} />
              <div>
                <p>{a.reason}</p>
                <p class="fine">You asked for {a.original}. Tiffin uses {a.adjusted}.</p>
              </div>
            </div>
          {/each}
        {:else}
          <p class="fine">Fill in About You to see your target.</p>
        {/if}
      </div>

      <div class="field addback">
        <label class="lab" for="tg-addback">Add back exercise</label>
        <div class="inline">
          <input
            id="tg-addback"
            class="in w80"
            type="number"
            min="0"
            max="100"
            step="5"
            inputmode="numeric"
            autocomplete="off"
            aria-invalid={bad('add-back')}
            aria-describedby={why('add-back', 'tg-addback-msg')}
            value={d.addback_pct ?? ''}
            onchange={(e) => { d.addback_pct = numOf(e); commit('add-back'); }}
          />
          <span class="fine">%</span>
        </div>
        {#if problemOf('add-back')}<p id="tg-addback-msg" class="fine warn" role="status">{problemOf('add-back')}</p>{/if}
        <p class="fine">Exercise calories are estimates and often run high. Lower this if you tend to eat them back.</p>
      </div>

      {#if view.drift}
        <div class="drift">
          <p class="fine">{view.drift.text}</p>
          <button type="button" class="btn" aria-disabled={locked ? 'true' : undefined} title={locked || undefined} onclick={accept}>Update Target</button>
        </div>
      {/if}

      {#if view.earlier.length}
        <details class="earlier">
          <summary>Earlier Targets <span class="fine">({view.earlier.length})</span></summary>
          <ul>
            {#each view.earlier as e}
              <li><span class="when">{e.date_text}</span><span class="num">{e.kcal_text}</span><span class="fine">{e.macros_text}</span></li>
            {/each}
          </ul>
        </details>
      {/if}
    </section>

    <section aria-labelledby="tg-macros-h">
      <h2 id="tg-macros-h" class="caps">Macros</h2>
      <MacroEditor id="tg-m" bind:spec={d.macros} result={result?.macros ?? null} ready={!!result?.ready} presets={view.presets} unit={d.per_kg_unit} onchange={() => commit()} />
    </section>

    <section aria-labelledby="tg-week-h" id="tg-week">
      <h2 id="tg-week-h" class="caps">Weekday Plan</h2>
      <p class="fine">Make a template for days that need a different target, such as Training and Rest days, then choose one for each weekday. A template adds or subtracts calories from your base target.</p>
      <ul class="tpls" id="tg-templates">
        {#each d.templates as t (t.id)}
          {@const r = result?.templates.find((x) => x.id === t.id)}
          <li>
            <button type="button" class="tpl" id="tg-tpl-{t.id}" onclick={(e) => (editing = { t, opener: e.currentTarget })}>
              <span class="l1">
                <span class="name" title={t.name}>{t.name}</span>
                <span class="num">{r?.offset_text ?? ''}</span>
                <span class="num">{r?.day_text ?? ''}</span>
                <span class="pencil" aria-hidden="true"><Icon name="pencil" size={16} /></span>
              </span>
              <span class="fine">{r?.macros_text ?? ''}</span>
              <span class="sr">Edit {t.name}</span>
            </button>
          </li>
        {/each}
      </ul>
      {#if !d.templates.length}<p class="fine none">No weekday plans yet.</p>{/if}
      <button type="button" class="btn ghost new" id="tg-new-template" aria-disabled={locked ? 'true' : undefined} title={locked || undefined} onclick={(e) => { if (!locked) editing = { t: null, opener: e.currentTarget }; }}>
        <Icon name="plus" size={16} />New Template
      </button>

      <div class="week">
        {#each DAYS as day, i (day)}
          {@const w = result?.week[i]}
          <div class="day">
            <span class="caps lab" aria-hidden="true">{w?.label ?? day}</span>
            <select
              class="in"
              aria-label="Template for {day}"
              value={d.week[i] ?? ''}
              onchange={(e) => { d.week[i] = e.currentTarget.value === '' ? null : Number(e.currentTarget.value); commit(); }}
            >
              <option value="">Default plan</option>
              {#each d.templates as t (t.id)}<option value={String(t.id)}>{t.name}</option>{/each}
            </select>
            <span class="fine num">{w?.kcal_text ?? ''}</span>
            {#if w?.note}<span class="fine warn" title={w.note}>{w.note}</span>{/if}
          </div>
        {/each}
      </div>
      {#if result?.today_text}<p class="fine today">{result.today_text}</p>{/if}
    </section>

    <p class="saved fine" role="status">{#if savedNote}<span class="ok"><Icon name="check" size={16} /></span>Targets saved. They apply from today.{/if}</p>

    {#if dirty}
      <div class="bar">
        {#if confirm}
          <div class="notice" role="alert"><Icon name="warn" size={16} /><p>{confirm}</p></div>
        {/if}
        {#if saveError}
          <div class="notice" role="alert"><Icon name="warn" size={16} /><p>{saveError}</p></div>
        {/if}
        <div class="barrow">
          <div class="say">
            <p>You have unsaved changes.</p>
            <p class="fine warn" role="status">{announce}</p>
          </div>
          <button type="button" class="btn" onclick={() => { discard(); adopting.clear(); confirm = ''; attempted = false; touched = []; announce = ''; saveError = ''; recalc(); }}>Discard Changes</button>
          <button
            type="button"
            class="btn primary"
            aria-disabled={!result?.can_save || locked ? 'true' : undefined}
            title={locked || undefined}
            onclick={save}
          >{confirm ? 'Save Anyway' : 'Save Targets'}</button>
        </div>
      </div>
    {/if}
  {/if}
</div>

{#if editing && view && d}
  <TemplateDialog
    template={editing.t}
    draft={d}
    presets={view.presets}
    {today}
    onsave={saveTemplate}
    onremove={removeTemplate}
    onclose={closeTemplate}
    {onerror}
  />
{/if}

<style>
  /* the column is 840 px of content; the one layout threshold below is this column's own width, never the window's */
  .targets { container-type: inline-size; max-width: 904px; margin: 0 auto; padding: 0 32px 64px; font-size: 14.5px; line-height: 1.45; }
  .targets.dirty { padding-bottom: 0; }
  .fine { margin: 0; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .fine.warn { color: var(--warn); }
  .title { display: flex; align-items: center; min-height: 64px; }
  h1 { margin: 0 8px 0 4px; font: 700 24px var(--font-display); letter-spacing: -0.018em; color: var(--targets-ink); }
  section { margin-top: 32px; }
  h2.caps { margin: 0 0 16px; font-size: 14.5px; color: var(--ink); }
  p { margin: 0; }

  .setup { margin-top: 8px; padding: 16px 24px; border-radius: 14px; background: var(--well); color: var(--ink-2); }
  .notice { display: flex; align-items: flex-start; gap: 16px; margin: 8px 0 0; padding: 8px 16px; border-radius: 14px; background: var(--warn-wash); color: var(--warn); }
  .notice :global(svg) { flex: none; margin-top: 4px; }
  .notice p { flex: 1; }

  .grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 24px 16px; }
  .field { display: flex; flex-direction: column; align-items: flex-start; gap: 4px; min-width: 0; margin: 0; padding: 0; border: 0; }
  .field.full { grid-column: 1 / -1; }
  .lab { padding: 0; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .inline { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .in {
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
    -webkit-user-select: text;
    user-select: text;
  }
  .in:focus { box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  .in[aria-invalid='true'] { box-shadow: inset 0 0 0 1px var(--warn); }
  .w80 { width: 80px; }
  .w96 { width: 96px; }
  .w120 { width: 120px; }

  .radios { display: flex; flex-direction: column; width: 100%; margin-top: 8px; border-top: 1px solid var(--rule); }
  .opt { display: flex; align-items: center; gap: 16px; min-height: 56px; padding: 8px 16px; border-bottom: 1px solid var(--rule); }
  .opt.sel { background: var(--accent-wash); }
  .opt input { accent-color: var(--accent); width: 16px; height: 16px; margin: 0; flex: none; }
  .txt { display: flex; flex-direction: column; }
  .nm { font-weight: 600; }

  .card { padding: 16px 24px; border-radius: 14px; background: var(--card); box-shadow: inset 0 0 0 1px var(--rule); }
  .target { display: flex; align-items: baseline; gap: 8px; }
  .figure { font-size: 26px; line-height: 1.1; }
  dl { margin: 16px 0 0; }
  dl div { display: flex; align-items: baseline; justify-content: space-between; gap: 16px; min-height: 40px; padding: 8px 0; border-top: 1px solid var(--rule); }
  dl div:last-child .num { font-size: 21px; line-height: 1.2; }
  dt, dd { margin: 0; }
  dd { font-size: 14.5px; }
  .adjust { display: flex; align-items: flex-start; gap: 8px; margin-top: 16px; padding: 8px 16px; border-radius: 9px; background: var(--warn-wash); color: var(--warn); }
  .adjust :global(svg) { flex: none; margin-top: 4px; }
  .adjust .fine { color: var(--warn); }

  .tpls { list-style: none; margin: 16px 0 0; padding: 0; border-top: 1px solid var(--rule); }
  .tpl { position: relative; display: flex; flex-direction: column; justify-content: center; gap: 4px; width: 100%; min-height: 56px; padding: 8px 16px; border-bottom: 1px solid var(--rule); text-align: left; font: inherit; color: inherit; }
  .tpl:hover { background: var(--well); }
  .tpl .l1 { display: flex; align-items: center; gap: 16px; min-width: 0; }
  .tpl .name { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .pencil { display: grid; place-items: center; width: 32px; height: 32px; flex: none; color: var(--accent-text); }
  .none { margin-top: 8px; }
  .new { margin-top: 8px; padding: 0; }
  .week { display: grid; grid-template-columns: repeat(auto-fit, minmax(96px, 1fr)); gap: 8px; margin-top: 24px; }
  .day { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .day select.in { width: 100%; min-width: 0; }
  .today { margin-top: 16px; }

  .source { margin-bottom: 16px; }
  .ref { margin-top: 4px; }
  .addback { margin-top: 24px; }
  .drift { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 16px; margin-top: 24px; padding: 16px 24px; border-radius: 14px; background: var(--well); }
  .drift p { flex: 1 1 240px; }
  .earlier { margin-top: 24px; }
  .earlier summary { min-height: 40px; display: flex; align-items: center; gap: 8px; font-weight: 600; cursor: default; }
  .earlier ul { list-style: none; margin: 0; padding: 0; border-top: 1px solid var(--rule); }
  .earlier li { display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 16px; min-height: 48px; padding: 8px 0; border-bottom: 1px solid var(--rule); }
  .earlier .when { min-width: 96px; font-weight: 600; }

  .saved { display: flex; align-items: center; gap: 8px; min-height: 0; margin-top: 16px; }
  .saved:empty { display: none; }
  .ok { display: inline-grid; color: var(--ok); }

  .bar { position: sticky; bottom: 0; z-index: 4; margin: 24px -32px 0; padding: 0 32px 8px; background: var(--toolbar); backdrop-filter: blur(20px) saturate(170%); box-shadow: 0 -1px 0 var(--rule); }
  .barrow { display: flex; align-items: center; gap: 8px; min-height: 64px; }
  .say { flex: 1; min-width: 0; }
  .btn[aria-disabled='true'] { opacity: 0.5; }

  @container (max-width: 639px) {
    .grid { grid-template-columns: minmax(0, 1fr); }
  }
</style>
