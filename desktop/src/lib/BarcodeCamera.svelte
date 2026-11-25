<script lang="ts">
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';
  import * as api from './api';

  let { onfound, onclose }: {
    onfound: (code: string) => void;
    onclose: () => void;
  } = $props();

  let videoEl: HTMLVideoElement | undefined = $state();
  let phase: 'starting' | 'scanning' | 'error' = $state('starting');
  let message = $state('Starting camera…');

  function friendlyCameraError(e: unknown): string {
    const name = typeof e === 'object' && e !== null && 'name' in e ? String((e as { name?: unknown }).name) : '';
    if (name === 'NotAllowedError' || name === 'PermissionDeniedError') {
      return 'Camera access was declined. Allow camera permission in your browser, or use Scan Barcode… to pick a photo instead.';
    }
    if (name === 'NotFoundError' || name === 'DevicesNotFoundError') {
      return 'No camera was found on this device. Use Scan Barcode… to pick a photo instead.';
    }
    if (name === 'NotReadableError' || name === 'TrackStartError') {
      return 'Your camera is in use by another app. Close it or use Scan Barcode… to pick a photo instead.';
    }
    return 'Couldn’t open the camera. Use Scan Barcode… to pick a photo instead.';
  }

  $effect(() => {
    let closed = false;
    let stream: MediaStream | null = null;
    let videoTrack: MediaStreamTrack | null = null;
    let timer = 0;
    const canvas = document.createElement('canvas');
    const ctx = canvas.getContext('2d');

    const onTrackEnded = () => {
      if (closed) return;
      stopAll();
      phase = 'error';
      message = 'The camera disconnected. Use Scan Barcode… to pick a photo instead.';
    };

    const stopAll = () => {
      clearTimeout(timer);
      timer = 0;
      if (videoTrack) {
        videoTrack.removeEventListener('ended', onTrackEnded);
        videoTrack = null;
      }
      if (stream) {
        for (const t of stream.getTracks()) t.stop();
        stream = null;
      }
      if (videoEl) videoEl.srcObject = null;
    };

    const scheduleNext = () => {
      if (closed) return;
      timer = window.setTimeout(scanFrame, 600);
    };

    const scanFrame = async () => {
      if (closed || !videoEl || !ctx) return;
      if (videoEl.readyState < 2 || videoEl.videoWidth === 0 || videoEl.videoHeight === 0) {
        scheduleNext();
        return;
      }
      try {
        const maxSide = 1280;
        const longest = Math.max(videoEl.videoWidth, videoEl.videoHeight);
        const scale = longest > maxSide ? maxSide / longest : 1;
        canvas.width = Math.max(1, Math.round(videoEl.videoWidth * scale));
        canvas.height = Math.max(1, Math.round(videoEl.videoHeight * scale));
        ctx.drawImage(videoEl, 0, 0, canvas.width, canvas.height);

        const blob = await new Promise<Blob | null>((ok) => canvas.toBlob(ok, 'image/jpeg', 0.9));
        if (closed) return;
        if (blob) {
          const bytes = new Uint8Array(await blob.arrayBuffer());
          if (closed) return;
          const code = await api.barcodeBytes(bytes);
          if (closed) return;
          if (code) {
            stopAll();
            closed = true;
            onfound(code);
            return;
          }
        }
      } catch {
        // Transient frame error: try the next frame.
      }
      if (!closed) scheduleNext();
    };

    const onVisibility = () => {
      if (document.hidden && !closed) {
        stopAll();
        closed = true;
        onclose();
      }
    };
    document.addEventListener('visibilitychange', onVisibility);

    (async () => {
      if (!navigator.mediaDevices?.getUserMedia || !ctx) {
        phase = 'error';
        message = 'This browser doesn’t support live camera scanning. Use Scan Barcode… to pick a photo instead.';
        return;
      }
      try {
        const s = await navigator.mediaDevices.getUserMedia({
          video: { facingMode: { ideal: 'environment' }, width: { ideal: 1280 }, height: { ideal: 720 } },
          audio: false,
        });
        if (closed) {
          for (const t of s.getTracks()) t.stop();
          return;
        }
        stream = s;
        videoTrack = s.getVideoTracks()[0] ?? null;
        videoTrack?.addEventListener('ended', onTrackEnded);
        if (videoEl) {
          videoEl.srcObject = s;
          await videoEl.play().catch(() => {});
        }
        if (closed) {
          stopAll();
          return;
        }
        phase = 'scanning';
        message = 'Hold a grocery barcode steady in the frame…';
        scheduleNext();
      } catch (e) {
        if (closed) return;
        phase = 'error';
        message = friendlyCameraError(e);
      }
    })();

    return () => {
      closed = true;
      document.removeEventListener('visibilitychange', onVisibility);
      stopAll();
    };
  });
</script>

<Dialog title="Scan a Barcode" onclose={onclose}>
  <p class="fine privacy">Camera frames stay on this device and are read locally — nothing is sent anywhere.</p>

  {#if phase !== 'error'}
    <div class="stage">
      <!-- svelte-ignore a11y_media_has_caption -->
      <video bind:this={videoEl} autoplay playsinline muted></video>
      <div class="reticle" aria-hidden="true"></div>
    </div>
  {/if}

  <p class="fine status" class:warn={phase === 'error'} role="status" aria-live="polite">
    {#if phase === 'error'}<Icon name="warn" size={14} />{/if}{message}
  </p>

  {#snippet actions()}
    <button type="button" class="btn" onclick={onclose}>Cancel</button>
  {/snippet}
</Dialog>

<style>
  .privacy { margin: 0; }
  .stage {
    position: relative;
    width: 100%;
    aspect-ratio: 4 / 3;
    border-radius: 12px;
    overflow: hidden;
    background: #14110f;
    box-shadow: inset 0 0 0 1px var(--rule);
  }
  video {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .reticle {
    position: absolute;
    inset: 22% 14%;
    border: 2px solid rgba(255, 255, 255, 0.72);
    border-radius: 10px;
    box-shadow: 0 0 0 9999px rgba(20, 17, 15, 0.28);
    pointer-events: none;
  }
  .status {
    margin: 0;
    display: flex;
    align-items: flex-start;
    gap: 6px;
  }
  .status.warn { color: var(--warn); }
  .status :global(svg) { flex: none; margin-top: 2px; }
</style>
