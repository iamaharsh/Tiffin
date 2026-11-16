<script lang="ts">
  import Icon from './Icon.svelte';
  import type { FoodTag } from './api';

  // Where a food comes from. Neutral on purpose: accent colour stays for things you can press.
  let { tag }: { tag: FoodTag } = $props();
</script>

<span class="tag caps">{#if tag === 'Open Food Facts'}<Icon name="globe" size={12} />{/if}{tag}</span>

<style>
  .tag {
    display: inline-flex;
    align-items: center;
    flex: none;
    gap: 4px;
    height: 20px;
    padding: 0 8px;
    border-radius: 5px;
    background: var(--well);
    color: var(--ink-2);
    font-size: 13px;
    white-space: nowrap;
  }
</style>
