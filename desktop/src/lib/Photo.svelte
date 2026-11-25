<script lang="ts">
  import FoodArt from './FoodArt.svelte';
  import { photoUrl, loadPhotoUrl } from './api';
  // The recipe's photo, or its painted plate when there is none (or the file is missing).
  let { photo, seed }: { photo: string; seed: number } = $props();
  let failedFor = $state('');
  let loaded = $state<{ name: string; url: string } | null>(null);
  const direct = $derived(photo ? photoUrl(photo) : '');
  const src = $derived(direct || (loaded?.name === photo ? loaded.url : ''));
  const failed = $derived(failedFor === photo && photo !== '');
  $effect(() => {
    const current = photo;
    if (!current || direct) return;
    let alive = true;
    loadPhotoUrl(current).then(
      (url) => { if (alive) loaded = { name: current, url }; },
      () => { if (alive) failedFor = current; },
    );
    return () => { alive = false; };
  });
</script>

{#if photo && !failed && src}
  <img {src} alt="" draggable="false" loading="lazy" decoding="async" onerror={() => (failedFor = photo)} />
{:else}
  <FoodArt {seed} />
{/if}

<style>
  img { display: block; width: 100%; height: 100%; object-fit: cover; }
</style>
