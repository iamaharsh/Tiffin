<script lang="ts">
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';
  import * as api from './api';

  // Asked once, the first time an online lookup is tried with no contact email saved. Rust decides whether the address is
  // acceptable (it goes into a request header); this dialog only shows its answer. Cancel sends nothing.
  let { onsaved, oncancel }: { onsaved: () => void; oncancel: () => void } = $props();

  let email = $state('');
  let problem = $state('');
  let saving = $state(false);

  async function save() {
    if (saving) return;
    saving = true;
    try {
      await api.setOffEmail(email);
      onsaved();
    } catch (e) {
      problem = String(e);
    } finally {
      saving = false;
    }
  }
</script>

<Dialog title="Add an email to search online" onclose={oncancel}>
  <form id="off-email-form" novalidate onsubmit={(e) => { e.preventDefault(); save(); }}>
    <p>Open Food Facts asks apps to include a contact email with searches. Tiffin sends it only to Open Food Facts, and only when you search online. You can change or remove it in Settings.</p>
    <label class="label" for="off-email-input">Your email address</label>
    <input
      id="off-email-input"
      name="email"
      type="email"
      autocomplete="email"
      spellcheck="false"
      placeholder="you@example.com"
      bind:value={email}
      aria-invalid={problem ? 'true' : undefined}
      aria-describedby={problem ? 'off-email-err' : undefined}
    />
    <p class="err" id="off-email-err" role="alert">{#if problem}<Icon name="warn" size={14} />{problem}{/if}</p>
  </form>
  {#snippet actions()}
    <button type="button" class="btn" onclick={oncancel}>Cancel</button>
    <button type="submit" form="off-email-form" class="btn primary" disabled={saving}>Save and search</button>
  {/snippet}
</Dialog>

<style>
  form { display: flex; flex-direction: column; gap: 8px; }
  form p { margin: 0 0 4px; }
  .label { font-size: 12.5px; color: var(--ink-2); }
  .err { display: flex; align-items: flex-start; gap: 6px; min-height: 18px; margin: 0; font-size: 12.5px; color: var(--warn); }
  .err :global(svg) { flex: none; margin-top: 1px; }
</style>
