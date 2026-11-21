<script lang="ts">
  // The Tiffin lockup (concept C): the owner's Canva export, background keyed out by desktop/tools/key-canva.mjs
  // (Canva Free exports no SVG, so this is a 320 px raster, 2x the largest size it is shown at). The art is
  // terracotta and olive, so in the dark theme it sits on a small cream tile instead of recolouring. Never below 96px wide.
  import lockup from '../assets/tiffin-lockup.png';
  let { width = 120 }: { width?: number } = $props();
</script>

<img class="logo" src={lockup} width={Math.max(width, 96)} alt="Tiffin" />

<style>
  .logo { display: block; height: auto; max-width: 100%; }
  :global(:root[data-theme='dark']) .logo { box-sizing: border-box; background: #fff6e9; border-radius: 14px; padding: 8px; }
</style>
