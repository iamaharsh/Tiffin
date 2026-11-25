<script lang="ts">
  import { untrack } from 'svelte';
  import { fade } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import Dialog from './Dialog.svelte';
  import Logo from './Logo.svelte';
  import Segmented from './Segmented.svelte';
  import { chooseTheme, savedTheme, type Theme } from './theme';
  import * as api from './api';
  import { update, checkForUpdate, installUpdate, holdsUnsaved } from './updates.svelte';
  import type { SavedVersion, SettingsInfo } from './api';

  type Tab = 'sync' | 'backup' | 'history' | 'trackers' | 'online' | 'claude' | 'appearance' | 'about';
  let { tab = $bindable(api.isWeb ? 'backup' : 'sync'), onchanged, onbackup, onerror, onnote }: {
    tab?: Tab;
    /** the library itself changed (restore, rollback, sync) */
    onchanged: () => void;
    /** a backup or persistence change completed (lets the web backup banner refresh) */
    onbackup?: () => void;
    onerror: (m: string) => void;
    onnote: (m: string) => void;
  } = $props();

  const DESKTOP_ONLY_TABS = new Set<Tab>(['sync', 'trackers', 'claude']);
  const allTabs: [Tab, string, string][] = [
    ['sync', 'Sync', 'refresh'],
    ['backup', 'Backup & Restore', 'tray'],
    ['history', 'Version History', 'clock'],
    ['trackers', 'Health Trackers', 'heart'],
    ['online', 'Online Food Search', 'globe'],
    ['claude', 'Send from Claude', 'out'],
    ['appearance', 'Appearance', 'moon'],
    ['about', 'About', 'book'],
  ];
  const tabs = api.isWeb ? allTabs.filter(([id]) => !DESKTOP_ONLY_TABS.has(id)) : allTabs;
  let look: Theme = $state(savedTheme());
  const looks: Record<Theme, string> = { system: api.isWeb ? 'Follow System' : 'Follow Windows', light: 'Light', dark: 'Dark' };
  let storage: api.StorageStatus | null = $state(null);
  let canInstall = $state(false);
  let installingApp = $state(false);
  let installNote = $state('');
  let info: SettingsInfo | null = $state(null);
  let folder = $state('');
  // A typed or chip-picked sync folder is lost unless Save & Sync Now is pressed; the other fields here save on blur or change.
  holdsUnsaved(() => info !== null && (folder !== info.sync_folder || email.trim() !== info.off_email));
  let device = $state('');
  let email = $state('');
  /** the crate's message when the address was refused */
  let emailProblem = $state('');
  let busy = $state('');
  let list: SavedVersion[] = $state([]);
  let confirm: { kind: 'backup' | 'history'; path: string | null; text: string | null; label: string; diary: boolean } | null = $state(null);
  let scope: 'recipes' | 'everything' = $state('recipes');
  let backupFile: HTMLInputElement | undefined = $state();
  let checks: api.Check[] | null = $state(null);
  let askErase = $state(false);

  async function load() {
    try {
      info = await api.settingsInfo();
      folder = info.sync_folder;
      device = info.device_name;
      email = info.off_email;
    } catch (e) {
      onerror(String(e));
    }
  }
  async function loadStorage() {
    if (!api.isWeb) return;
    storage = await api.storageStatus();
  }
  async function askPersist() {
    if (busy === 'persist') return;
    busy = 'persist';
    try {
      const granted = await api.requestPersistentStorage();
      await loadStorage();
      onbackup?.();
      onnote(granted ? 'Browser storage is now marked persistent.' : 'The browser didn’t mark storage as persistent. Download a backup file regularly to keep a safe copy.');
    } finally {
      busy = '';
    }
  }
  function formatBackupDate(ts: number | null): string {
    if (!ts) return 'No backup downloaded from this browser yet.';
    return `Last downloaded backup: ${new Date(ts).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' })}`;
  }

  $effect(() => {
    if (api.isWeb && DESKTOP_ONLY_TABS.has(tab)) tab = 'backup';
  });
  $effect(() => { untrack(load); });
  $effect(() => {
    if (!api.isWeb) return;
    return api.subscribeInstall((ok) => {
      canInstall = ok;
    });
  });

  async function installWebApp() {
    if (installingApp) return;
    installingApp = true;
    installNote = '';
    try {
      const outcome = await api.triggerInstall();
      if (outcome === 'installed') {
        installNote = 'Tiffin was installed or opened as an app.';
      } else if (outcome === 'cancelled') {
        installNote = 'Installation was canceled.';
      } else if (outcome === 'error') {
        installNote = 'Installation could not start in this browser.';
      }
    } finally {
      installingApp = false;
    }
  }

  $effect(() => {
    if (!api.isWeb) return;
    const onChange = () => {
      loadStorage();
    };
    window.addEventListener('tiffin-storage-change', onChange);
    return () => window.removeEventListener('tiffin-storage-change', onChange);
  });
  $effect(() => {
    const t = tab;
    untrack(() => {
      if (t === 'backup') {
        api.backups().then((l) => (list = l), (e) => onerror(String(e)));
        loadStorage();
      }
      if (t === 'history') api.snapshots().then((l) => (list = l), (e) => onerror(String(e)));
    });
  });

  /** Run a job with its button showing progress; report its message. */
  async function run(key: string, job: () => Promise<string | void>, after?: () => void) {
    busy = key;
    try {
      const msg = await job();
      if (msg) onnote(msg);
      after?.();
    } catch (e) {
      onerror(String(e));
    }
    busy = '';
    await load();
  }
  const set = (change: api.SettingsChange) => run('', () => api.setSettings(change));

  /** Save, change or clear (empty) the Open Food Facts contact email; the crate decides whether it is acceptable. */
  async function saveEmail(value: string) {
    emailProblem = '';
    try {
      await api.setOffEmail(value);
      info = await api.settingsInfo();
      email = info.off_email;
      onnote(email ? 'Email saved.' : 'Email removed. Tiffin will ask again the next time you search online.');
    } catch (e) {
      emailProblem = String(e);
    }
  }

  async function chooseBackup(e: Event) {
    const f = (e.currentTarget as HTMLInputElement).files?.[0];
    (e.currentTarget as HTMLInputElement).value = '';
    if (!f) return;
    try {
      const text = await f.text();
      // the crate reads the file and says whether it has a diary; the restore command decides what is restored
      const [row] = await api.pickedBackup(text);
      scope = 'recipes';
      confirm = { kind: 'backup', path: null, text, label: `“${f.name}”`, diary: row.has_diary };
    } catch (err) {
      onerror(String(err));
    }
  }
  function doRestore() {
    const c = confirm!;
    confirm = null;
    run('restore', () => (c.kind === 'history' ? api.rollback(c.path!) : api.restoreBackup(c.path, c.text, c.diary ? scope : 'recipes')), () => {
      onchanged();
      tab = c.kind; // reload the list
      (c.kind === 'history' ? api.snapshots() : api.backups()).then((l) => (list = l));
    });
  }

  async function copyInstructions() {
    try {
      if (!info?.inbox_ready) await api.setupInbox();
      await navigator.clipboard.writeText(info!.instructions);
      onnote('Copied. Paste it into Claude.');
    } catch (e) {
      onerror(String(e));
    }
    await load();
  }
</script>

<div class="settings">
  <nav class="tabs" aria-label="Settings">
    <h1>Settings</h1>
    {#each tabs as [id, label, icon]}
      <button class="tab" class:sel={tab === id} aria-current={tab === id ? 'page' : undefined} onclick={() => (tab = id)}>
        <span class="ti"><Icon name={icon} size={16} /></span>{label}
      </button>
    {/each}
  </nav>

  {#if info}
    {@const i = info}
    {#key tab}
      <section class="pane" in:fade={{ duration: 160 }}>
        {#if tab === 'sync' && !api.isWeb}
          <h2>Keep your library the same on every device</h2>
          <p class="lead">Choose a folder your devices share (OneDrive, Dropbox, iCloud Drive or a network folder). Tiffin keeps one sync file there and merges changes both ways: the newest edit of each recipe wins, and favorites, ratings and categories travel with their recipe. Deleted recipes stay deleted. A snapshot is taken before every sync.</p>
          <div class="note">
            <span class="caps">Stays on this PC</span>
            <p>Your food diary, exercise, body measurements and profile are never put in the sync folder, so they don’t reach your other devices. Use Back Up Now to keep a copy of them. Recipes, photos, pantry, meal plan and settings sync as before.</p>
          </div>
          <label class="field">Sync folder
            <input bind:value={folder} name="sync-folder" autocomplete="off" placeholder="C:\Users\you\OneDrive\Tiffin" spellcheck="false" />
          </label>
          {#if i.onedrive || i.dropbox}
            <div class="chips">
              {#if i.onedrive}<button class="chip" onclick={() => (folder = i.onedrive!)}>Use OneDrive</button>{/if}
              {#if i.dropbox}<button class="chip" onclick={() => (folder = i.dropbox!)}>Use Dropbox</button>{/if}
            </div>
          {/if}
          <div class="row">
            <button class="btn primary" disabled={!folder.trim() || busy === 'sync'} onclick={() => run('sync', async () => { await api.setSettings({ sync_folder: folder }); return api.syncNow(); }, onchanged)}>{busy === 'sync' ? 'Syncing…' : 'Save & Sync Now'}</button>
            {#if i.sync_folder}<button class="btn" onclick={() => { folder = ''; set({ sync_folder: '' }); }}>Stop Syncing</button>{/if}
          </div>
          <label class="check"><input type="checkbox" checked={i.auto_sync} onchange={(e) => set({ auto_sync: e.currentTarget.checked })} /> Sync automatically (when the app opens and every 5 minutes)</label>
          <label class="field narrow">This device’s name
            <input bind:value={device} onblur={() => { if (device.trim() && device !== i.device_name) set({ device_name: device }); }} />
          </label>
          <p class="fine">Last synced: {i.last_sync ?? 'never'}</p>
        {:else if tab === 'backup'}
          <h2>Back up everything in one file</h2>
          {#if api.isWeb}
            <p class="lead">Recipes, photos, favorites, categories, meal plan, menus, grocery lists, pantry, settings, food diary, exercise, body measurements and profile download as a single <code>.recipebox</code> file you can restore here or in the Tiffin desktop app.</p>
            {#if storage}
              <div class="note">
                <span class="caps">Browser storage</span>
                {#if storage.persisted === true}
                  <p class="st ok"><Icon name="check" size={16} /> Protected from automatic browser cleanup on this device.</p>
                {:else if storage.persisted === false}
                  <p class="st bad"><Icon name="warn" size={16} /> Browser storage isn’t marked persistent, so site data may be cleared when space runs low.</p>
                {:else}
                  <p class="st">Your library and diary are kept in private browser storage on this device.</p>
                {/if}
                <p class="fine">{formatBackupDate(storage.lastBackupAt)}</p>
                {#if storage.persisted === false}
                  <div class="row"><button type="button" class="btn small" aria-busy={busy === 'persist'} disabled={busy === 'persist'} onclick={askPersist}>{busy === 'persist' ? 'Asking…' : 'Ask Browser to Keep Data'}</button></div>
                {/if}
              </div>
            {/if}
          {:else}
            <p class="lead">Recipes, photos, favorites, categories, meal plan, menus, grocery lists, pantry and settings go into a single file in <code>{i.backups_dir}</code>. Your food diary, exercise, body measurements and profile are included too. Only the recipe library is copied to your sync folder. Your diary and body data are never put there.</p>
          {/if}
          <div class="row">
            <button class="btn primary" disabled={busy === 'backup'} onclick={() => run('backup', api.backupNow, () => { api.backups().then((l) => (list = l)); loadStorage(); onbackup?.(); })}>{busy === 'backup' ? 'Backing Up…' : 'Back Up Now'}</button>
            {#if !api.isWeb}<button class="btn" onclick={() => api.openFolder('backups').catch((e) => onerror(String(e)))}>Open Folder</button>{/if}
          </div>
          <h2 class="sub">Restore on this or a new device</h2>
          <p class="lead">{api.isWeb ? 'Pick a saved backup below or choose a .recipebox file from your device.' : 'Pick a backup below or choose a backup file (from a USB stick, say).'} Your current library is saved to Version History first, so a restore can be undone.</p>
          <div class="row">
            <button class="btn" onclick={() => backupFile?.click()}>Choose a Backup File…</button>
            <input bind:this={backupFile} type="file" accept=".recipebox" hidden onchange={chooseBackup} />
          </div>
          {@render versions('backup')}
          <!-- disabled while any job runs: a backup already under way read the diary before the delete and would write it back -->
          <h2 class="sub">Delete your health data</h2>
          <p class="lead">
            {#if api.isWeb}
              Removes your food diary, exercise, body measurements and profile from this browser, with their snapshots, and takes them out of backups kept in this browser (files you already downloaded aren’t affected). Recipes, photos, My Foods and settings stay.
            {:else}
              Removes your food diary, exercise, body measurements and profile from this PC, with their snapshots, and takes them out of your backups. Recipes, photos, My Foods and settings stay.
            {/if}
          </p>
          <div class="row"><button class="btn danger" disabled={busy !== ''} onclick={() => (askErase = true)}>{busy === 'erase' ? 'Deleting…' : 'Delete All Health Data…'}</button></div>
        {:else if tab === 'history'}
          <h2>Roll back to an earlier version</h2>
          <p class="lead">Snapshots are kept automatically: every 10 minutes of changes, and before {api.isWeb ? 'imports, merges, restores and deletions' : 'imports, merges, syncs, restores, updates and deletions'}. Rolling back saves the current version first.</p>
          <p class="fine">Rolling back changes recipes and settings only. It doesn’t change your food diary.</p>
          <div class="row"><button class="btn" onclick={() => run('', async () => { await api.takeSnapshot(); list = await api.snapshots(); return 'Snapshot saved'; })}>Take a Snapshot Now</button></div>
          <div class="table">
            <div class="tr now"><span><strong>Now</strong><em>your library as it is</em></span><span class="num">{i.recipes} recipes, {i.favorites} favorites</span><span class="cur">Current</span></div>
          </div>
          {@render versions('history')}
        {:else if tab === 'trackers' && !api.isWeb}
          <h2>Send recipes to a health tracker</h2>
          <p class="lead">Tiffin writes each recipe’s per-serving nutrition (and its ingredient list) as a file your tracker can import. In Cronometer, add it as a Custom Food or paste the ingredient list into its recipe importer.</p>
          <div class="two">
            <label class="field">Service
              <select value={i.tracker_service} onchange={(e) => set({ tracker_service: e.currentTarget.value })}>{#each i.services as s}<option>{s}</option>{/each}</select>
            </label>
            <label class="field">Save files to
              <input value={i.tracker_folder} placeholder={i.tracker_default} spellcheck="false" onchange={(e) => set({ tracker_folder: e.currentTarget.value })} />
            </label>
          </div>
          <label class="check"><input type="checkbox" checked={i.tracker_auto_send} onchange={(e) => set({ tracker_auto_send: e.currentTarget.checked })} /> Send automatically whenever I save a recipe</label>
          <div class="row">
            <button class="btn primary" disabled={busy === 'tracker'} onclick={() => run('tracker', api.sendAllToTracker, onchanged)}>{busy === 'tracker' ? 'Preparing…' : 'Send All Recipes Now'}</button>
            <button class="btn" onclick={() => api.openFolder('tracker').catch((e) => onerror(String(e)))}>Open Folder</button>
            {#if i.service_url}<button class="btn ghost" onclick={() => api.openLink(i.service_url).catch((e) => onerror(String(e)))}>Open {i.tracker_service} <Icon name="out" size={13} /></button>{/if}
          </div>
        {:else if tab === 'online'}
          <h2>Search more foods online</h2>
          <p class="lead">Tiffin searches the foods stored {api.isWeb ? 'in this browser' : 'on this PC'} first. Search Online and barcode lookups also ask Open Food Facts, which asks apps to include a contact email. Tiffin sends it only to Open Food Facts, and only when you search online. If you remove it, Tiffin asks again the next time you search online.</p>
          <form class="stack" novalidate onsubmit={(e) => { e.preventDefault(); saveEmail(email); }}>
            <label class="field narrow">Contact email
              <input bind:value={email} name="email" type="email" autocomplete="email" spellcheck="false" placeholder="you@example.com" aria-invalid={emailProblem ? 'true' : undefined} aria-describedby={emailProblem ? 'off-email-problem' : undefined} />
            </label>
            <p class="problem" id="off-email-problem" role="alert">{#if emailProblem}<Icon name="warn" size={14} />{emailProblem}{/if}</p>
            <div class="row">
              <button class="btn primary" disabled={email.trim() === i.off_email}>Save email</button>
              <button type="button" class="btn" disabled={!i.off_email} onclick={() => saveEmail('')}>Remove email</button>
            </div>
          </form>
        {:else if tab === 'claude' && !api.isWeb}
          <h2>Send recipes from Claude</h2>
          <p class="lead">Ask Claude for a recipe, then say “save it to Tiffin”. Claude drops it in a folder this app watches, and it shows up for a quick review within seconds.</p>
          <ol class="steps">
            <li class:done={i.inbox_ready}>
              <span class="n">{#if i.inbox_ready}<Icon name="check" size={14} />{:else}1{/if}</span>
              <div>
                <strong>Set up the inbox</strong>
                <div class="row">
                  <button class="btn" class:primary={!i.inbox_ready} onclick={() => run('', api.setupInbox)}>{i.inbox_ready ? 'Set Up Again' : 'Set Up Inbox'}</button>
                  {#if i.inbox_ready}<button class="btn" onclick={() => api.openFolder('inbox').catch((e) => onerror(String(e)))}>Open Folder</button>{/if}
                </div>
                <code class="path">{i.inbox_dir}</code>
              </div>
            </li>
            <li>
              <span class="n">2</span>
              <div>
                <strong>Tell Claude how to reach it</strong>
                <div class="row"><button class="btn" class:primary={i.inbox_ready} onclick={copyInstructions}>Copy Instructions for Claude</button></div>
                <ul class="how">
                  <li><b>Claude Code:</b> paste them into a chat, or into your CLAUDE.md so it always knows. Claude Code writes the file itself.</li>
                  <li><b>Claude desktop app:</b> turn on a file-access connector (for example Filesystem), allow the inbox folder, then paste the instructions into a project or chat.</li>
                  <li><b>Claude on the web or phone</b> can’t reach your computer: ask for the recipe as JSON, copy the reply, and paste it on the Browser page.</li>
                </ul>
              </div>
            </li>
            <li>
              <span class="n">3</span>
              <div><strong>Ask for a recipe</strong><p class="fine">For example: “Find me a good weeknight chili and save it to Tiffin.” Keep Tiffin open: the recipe opens for review, and duplicates are merged.</p></div>
            </li>
          </ol>
          <details><summary>What the instructions say</summary><pre class="selectable">{i.instructions}</pre></details>
        {:else if tab === 'appearance'}
          <h2>Choose how Tiffin looks</h2>
          <p class="lead">{api.isWeb ? 'Follow System switches between light and dark with your device. Your choice is kept in this browser.' : 'Follow Windows switches between light and dark when Windows does. Your choice is kept on this computer.'}</p>
          <div class="row">
            <Segmented label="Appearance" options={['system', 'light', 'dark'] as Theme[]} value={look} onchange={(t) => { look = t; chooseTheme(t); }}>
              {#snippet item(t)}{looks[t]}{/snippet}
            </Segmented>
          </div>
        {:else}
          <div class="about-logo"><Logo width={160} /></div>
          <h2 class="about-h">Tiffin {i.version}</h2>
          {#if api.isWeb}
            <p class="lead">Your library and food diary live in this browser’s private storage on your device — never on a Tiffin server. Snapshots are kept automatically, and you can download a full backup file anytime.</p>
            <div class="row">
              <a class="btn" href="/privacy/" target="_blank" rel="noopener">Read Privacy Policy <Icon name="out" size={13} /></a>
            </div>
            {#if canInstall || installingApp || installNote}
              <h2 class="sub">Install app</h2>
              <div class="stack">
                {#if canInstall || installingApp}
                  <p class="lead">Install Tiffin on this device for offline access and its own app window.</p>
                  <div class="row">
                    <button type="button" class="btn primary" aria-busy={installingApp} aria-disabled={installingApp} onclick={installWebApp}>
                      {installingApp ? 'Installing…' : 'Install Tiffin'}
                    </button>
                  </div>
                {/if}
                <p class="fine" role="status">{installNote}</p>
              </div>
            {/if}
            <h2 class="sub">Desktop app</h2>
            <p class="lead">Prefer a native Windows app with folder sync, label OCR, and a Claude inbox? Move your library between desktop and web anytime with a <code>.recipebox</code> backup file.</p>
            <div class="row">
              <a class="btn" href="https://github.com/iamaharsh/Tiffin/releases/latest" target="_blank" rel="noopener noreferrer">Download for Windows <Icon name="out" size={13} /></a>
            </div>
          {:else}
            <p class="lead">Your library lives outside the program folder, and a snapshot is taken whenever a new version first opens it, so updates and reinstalls can’t remove your recipes, favorites or collections.</p>
            <h2 class="sub">Tiffin on the web</h2>
            <p class="lead">Use Tiffin in any modern browser at <code>tiffin.fyi</code> — same offline rules, kept privately in browser storage, and compatible with your <code>.recipebox</code> backups.</p>
            <div class="row">
              <button type="button" class="btn" onclick={() => api.openLink('https://tiffin.fyi').catch((e) => onerror(String(e)))}>Open tiffin.fyi <Icon name="out" size={13} /></button>
              <button type="button" class="btn ghost" onclick={() => api.openLink('https://tiffin.fyi/privacy/').catch((e) => onerror(String(e)))}>Privacy policy <Icon name="out" size={13} /></button>
            </div>
            <h2 class="sub">Updates</h2>
            <div class="upd" role="status">
              {#if update.status === 'checking'}
                <p class="st"><span class="spin"><Icon name="refresh" size={16} /></span> Checking for updates…</p>
              {:else if update.status === 'current'}
                <p class="st ok"><Icon name="check" size={16} /> Tiffin {i.version} is up to date.</p>
                <p class="fine soft">Checked {update.checked}.</p>
              {:else if update.status === 'checkfailed'}
                <p class="st bad" role="alert"><Icon name="warn" size={16} /> Couldn’t check for updates: {update.reason}.</p>
              {:else if update.status === 'available'}
                <p class="st acc"><Icon name="refresh" size={16} /> Tiffin {update.info?.version} is available.</p>
                {#if update.unsaved}<p class="fine">Finish your edit to install.</p>{/if}
              {:else if update.status === 'installing'}
                <p class="st"><span class="spin"><Icon name="refresh" size={16} /></span> Backing up your library, then installing Tiffin {update.info?.version}. Tiffin restarts when it’s done.</p>
              {:else if update.status === 'failed'}
                <p class="st bad" role="alert"><Icon name="warn" size={16} /> Update couldn’t install: {update.reason}.</p>
              {:else}
                <p class="st">Tiffin {i.version} is installed.</p>
              {/if}
            </div>
            <div class="row">
              {#if update.status === 'checkfailed'}
                <button class="btn" onclick={() => checkForUpdate(true)}>Try again</button>
              {:else if update.status === 'available'}
                <button class="btn" onclick={() => (update.whatsNew = true)}>What’s new</button>
                <button class="btn primary" aria-disabled={update.unsaved} onclick={installUpdate}>Install and restart</button>
              {:else if update.status === 'failed'}
                <button class="btn" onclick={installUpdate}>Try again</button>
              {:else if update.status === 'installing'}
                <!-- no buttons while the installer runs -->
              {:else}
                <button class="btn" disabled={update.status === 'checking'} onclick={() => checkForUpdate(true)}>Check for updates</button>
              {/if}
            </div>
          {/if}
          <h2 class="sub">Data sources</h2>
          <p class="lead">Food, nutrient and activity data in Tiffin comes from these sources, credited as each one asks.</p>
          <ul class="sources">
            {#each i.sources as s (s.name)}
              <li>
                <strong>{s.name}</strong>
                <span class="fine">{s.used_for}</span>
                <span class="fine">{s.licence}</span>
                <span class="fine credit selectable">{s.credit}{#if !s.credit.includes(s.link)} <code>{s.link}</code>{/if}</span>
              </li>
            {/each}
          </ul>
          <h2 class="sub">Tidy up</h2>
          <p class="lead">Find recipes saved more than once (same web address, or the same name and ingredients) and merge each into its older copy. Nothing is lost: missing details are filled in from the copy, and a snapshot goes to Version History first.</p>
          <div class="row"><button class="btn" disabled={busy === 'merge'} onclick={() => run('merge', api.mergeDuplicates, onchanged)}>{busy === 'merge' ? 'Merging…' : 'Find & Merge Duplicates'}</button></div>
          <h2 class="sub">Self-tests</h2>
          <p class="lead">The checks run before each release: timers, step-by-step mode, video links, every site import rule, the ingredient reader and data safety. They use sample recipes, so your library isn’t touched.</p>
          <div class="row"><button class="btn" disabled={busy === 'tests'} onclick={() => run('tests', async () => { checks = await api.selfTests(); const bad = checks.filter((c) => !c.passed).length; return bad ? `${bad} of ${checks.length} checks failed` : `All ${checks.length} checks passed`; })}>{busy === 'tests' ? 'Running…' : 'Run Self-Tests'}</button></div>
          {#if checks}
            <div class="table" role="table" aria-label="Self-test results">
              {#each checks as c, n (n)}
                <div class="tr check-row" role="row">
                  <span role="cell" class="res" class:bad={!c.passed}><Icon name={c.passed ? 'check' : 'warn'} size={14} />{c.passed ? 'Pass' : 'Fail'}</span>
                  <span role="cell"><strong>{c.name}</strong><em>{c.group}{c.detail ? ` · ${c.detail}` : ''}</em></span>
                </div>
              {/each}
            </div>
          {/if}
        {/if}
      </section>
    {/key}
  {/if}
</div>

{#snippet versions(kind: 'backup' | 'history')}
  {#if list.length}
    <div class="table" role="table">
      {#each list as v (v.path)}
        <div class="tr" role="row">
          <span role="cell"><strong>{v.when}</strong>{#if v.reason}<em>{v.reason}</em>{/if}</span>
          <span role="cell" class="num">{v.recipes} recipes{#if kind === 'history'}, {v.favorites} favorites{/if}{#if v.has_diary}<em>diary included</em>{/if}</span>
          <button class="btn small" aria-label="{kind === 'history' ? 'Roll Back' : 'Restore'} {v.when}" onclick={() => (confirm = { kind, path: v.path, text: null, label: kind === 'history' ? `the version from ${v.when}` : `the backup from ${v.when}`, diary: v.has_diary }, scope = 'recipes')}>{kind === 'history' ? 'Roll Back' : 'Restore'}</button>
        </div>
      {/each}
    </div>
  {:else}
    <p class="fine">{kind === 'history' ? 'No snapshots yet.' : 'No backups found yet.'}</p>
  {/if}
{/snippet}

{#if confirm}
  {@const c = confirm}
  <Dialog title={c.kind === 'history' ? 'Roll Back Library?' : 'Restore Library?'} onclose={() => (confirm = null)}>
    {#if c.kind === 'backup' && c.diary}
      <p style="margin:0;color:var(--ink)">Restore {c.label}. What should come back?</p>
      <div class="opts" role="radiogroup" aria-label="What to restore">
        <label class="check opt"><input type="radio" name="scope" value="recipes" bind:group={scope} /> <span><b>Recipes and library only</b><span class="fine">Recipes, photos, meal plan, pantry and settings. Your food diary, exercise and body data {api.isWeb ? 'in this browser' : 'on this PC'} are not touched.</span></span></label>
        <label class="check opt"><input type="radio" name="scope" value="everything" bind:group={scope} /> <span><b>Everything, including the food diary</b><span class="fine">Also replaces your diary, exercise, body measurements and profile with the ones in this backup. Your current diary is kept first, so you can switch back.</span></span></label>
      </div>
      <p class="fine">Your current library is saved to Version History first, so you can switch back.</p>
    {:else if c.kind === 'backup'}
      <p style="margin:0;color:var(--ink)">Replace your library with {c.label}?</p>
      <p class="fine">Your current library is saved to Version History first, so you can switch back.</p>
      <p class="fine">This backup was made before the food diary existed, so your diary stays as it is.</p>
    {:else}
      <p style="margin:0;color:var(--ink)">Replace your library with {c.label}?</p>
      <p class="fine">Your current library is saved to Version History first, so you can switch back.</p>
    {/if}
    {#snippet actions()}
      {#if c.kind === 'history'}
        <button class="btn" onclick={() => (confirm = null)}>Cancel</button>
        <button class="btn primary" onclick={doRestore}>Roll Back</button>
      {:else}
        <button class="btn" onclick={() => (confirm = null)}>Keep Current Library</button>
        <button class="btn primary" onclick={doRestore}>{c.diary && scope === 'everything' ? 'Restore Everything' : 'Restore Recipes'}</button>
      {/if}
    {/snippet}
  </Dialog>
{/if}

{#if askErase}
  <Dialog title="Delete All Health Data?" onclose={() => (askErase = false)}>
    {#if api.isWeb}
      <p style="margin:0;color:var(--ink)">Your food diary, exercise, body measurements and profile are deleted from this browser, with their snapshots, and taken out of backups kept in this browser (files you already downloaded aren’t affected).</p>
    {:else}
      <p style="margin:0;color:var(--ink)">Your food diary, exercise, body measurements and profile are deleted from this PC, with their snapshots, and taken out of every backup in <code>{info?.backups_dir}</code>.</p>
    {/if}
    <p class="fine">This can’t be undone. Recipes, photos, My Foods and settings stay, in the library and in the backups.</p>
    {#snippet actions()}
      <button class="btn" onclick={() => (askErase = false)}>Cancel</button>
      <button class="btn danger" onclick={() => { askErase = false; run('erase', api.deleteHealthData, () => { onchanged(); api.backups().then((l) => (list = l), (e) => onerror(String(e))); }); }}>Delete Health Data</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .settings { display: flex; min-height: 100%; }
  .tabs { position: sticky; top: 0; align-self: flex-start; width: 230px; flex: none; height: 100vh; padding: 22px 10px; border-right: 1px solid var(--rule); display: flex; flex-direction: column; gap: 2px; }
  h1 { margin: 0 10px 14px; font: 700 26px var(--font-display); letter-spacing: -0.02em; }
  .tab { display: flex; align-items: center; gap: 10px; height: 34px; padding: 0 10px; border-radius: 9px; text-align: left; font-weight: 500; transition: background-color 120ms ease-out; }
  .tab:hover { background: var(--sidebar-hover); }
  .tab.sel { background: var(--accent); color: #fff; }
  .ti { display: grid; place-items: center; width: 20px; color: var(--ink-2); }
  .tab.sel .ti { color: #fff; }
  .pane { flex: 1; min-width: 0; max-width: 760px; padding: 28px 36px 64px; display: flex; flex-direction: column; gap: 14px; }
  h2 { margin: 0; font: 700 21px var(--font-display); letter-spacing: -0.012em; }
  h2.sub { margin-top: 18px; }
  .about-logo { margin-bottom: 32px; }
  .about-h { font-weight: 600; line-height: 1.2; }
  .upd { display: flex; flex-direction: column; gap: 4px; }
  .st { margin: 0; display: flex; align-items: center; gap: 8px; color: var(--ink); }
  .st.ok { color: var(--olive-text); }
  .st.bad { color: var(--warn); }
  .st.acc { color: var(--accent-text); }
  .soft { color: var(--ink-3); }
  [aria-disabled='true'] { opacity: 0.5; cursor: default; }
  .spin { display: inline-flex; color: var(--ink-2); }
  @media (prefers-reduced-motion: no-preference) { .spin { animation: turn 1s linear infinite; } }
  @keyframes turn { to { transform: rotate(360deg); } }
  .lead { margin: 0; color: var(--ink-2); max-width: 66ch; }
  .fine { margin: 0; font-size: 13px; color: var(--ink-2); }
  .sources { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 14px; max-width: 66ch; }
  .sources li { display: flex; flex-direction: column; gap: 2px; }
  .sources strong { font-weight: 600; }
  .credit { font-style: italic; }
  .note { display: flex; flex-direction: column; gap: 6px; padding: 16px; border-radius: 12px; background: var(--well); }
  .note p { margin: 0; color: var(--ink-2); }
  .note .st { color: var(--ink); }
  .note .st.ok { color: var(--olive-text); }
  .note .st.bad { color: var(--warn); }
  .note .row { margin-top: 4px; }
  .note .btn { background: var(--card); box-shadow: inset 0 0 0 1px var(--rule-strong); }
  .note .btn:hover { background: var(--page); }
  .opts { display: flex; flex-direction: column; gap: 12px; }
  .check.opt { align-items: flex-start; }
  .check.opt > span { display: flex; flex-direction: column; gap: 2px; }
  .check.opt input { margin-top: 2px; }
  code { font-size: 12.5px; padding: 1px 5px; border-radius: 5px; background: var(--well); overflow-wrap: anywhere; -webkit-user-select: text; user-select: text; }
  .stack { display: flex; flex-direction: column; gap: 10px; }
  .problem { display: flex; align-items: flex-start; gap: 6px; min-height: 18px; margin: 0; font-size: 12.5px; color: var(--warn); }
  .problem :global(svg) { flex: none; margin-top: 1px; }
  .row { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .field { display: flex; flex-direction: column; gap: 5px; font-size: 12.5px; color: var(--ink-2); }
  .field.narrow { max-width: 320px; }
  .field input, .field select {
    height: 36px; padding: 0 10px; border: 0; border-radius: 9px; background: var(--well); box-shadow: inset 0 0 0 1px var(--rule);
    color: var(--ink); font: inherit; font-size: 14px; outline: none; -webkit-user-select: text; user-select: text;
  }
  .field input:focus, .field select:focus { background: var(--card); box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  .field input::placeholder { color: var(--ink-3); }
  .field select option { color: #1f1b18; background: #fff; }
  .two { display: grid; grid-template-columns: 1fr 2fr; gap: 12px; }
  .chips { display: flex; gap: 6px; }
  .chip { height: 28px; padding: 0 12px; border-radius: 14px; font-size: 13px; color: var(--accent-text); background: var(--accent-wash); }
  .chip:hover { filter: brightness(0.97); }
  .check { display: flex; align-items: flex-start; gap: 9px; font-size: 14px; }
  .check input { accent-color: var(--accent); width: 16px; height: 16px; margin: 2px 0 0; flex: none; }
  .btn { display: inline-flex; align-items: center; gap: 6px; height: 34px; padding: 0 16px; border-radius: 9px; font-size: 14px; font-weight: 500; color: var(--ink); text-decoration: none; background: var(--well); box-shadow: inset 0 0 0 1px var(--rule); transition: background-color 120ms ease-out, transform 100ms ease-out; }
  .btn:hover:not(:disabled):not([aria-disabled='true']) { background: var(--rule); }
  .btn:active:not(:disabled):not([aria-disabled='true']) { transform: scale(0.97); }
  .btn:disabled { opacity: 0.55; }
  .btn.primary { color: #fff; background: var(--accent); box-shadow: inset 0 -1px 0 rgba(0, 0, 0, 0.15); }
  .btn.primary:hover:not(:disabled):not([aria-disabled='true']) { background: var(--accent-press); }
  .btn.danger { color: var(--on-heart); background: var(--heart); box-shadow: none; }
  .btn.danger:hover:not(:disabled) { background: var(--heart); filter: brightness(0.94); }
  .btn.ghost { background: none; box-shadow: none; color: var(--accent-text); }
  .btn.small { height: 28px; padding: 0 12px; font-size: 13px; }
  .table { border-radius: 12px; overflow: hidden; box-shadow: inset 0 0 0 1px var(--rule); }
  .tr { display: grid; grid-template-columns: 1.4fr 1fr auto; gap: 12px; align-items: center; padding: 9px 14px; border-top: 1px solid var(--rule); font-size: 13.5px; }
  .tr:first-child { border-top: 0; }
  .tr span { display: flex; flex-direction: column; min-width: 0; }
  .tr strong { font-weight: 600; }
  .tr em { font-style: normal; font-size: 12.5px; color: var(--ink-2); }
  .tr.now { background: var(--accent-wash); }
  .cur { font-size: 12.5px; color: var(--accent-text); text-align: right; }
  .steps { list-style: none; margin: 4px 0 0; padding: 0; display: flex; flex-direction: column; gap: 18px; }
  .steps > li { display: flex; gap: 14px; }
  .steps > li > div { display: flex; flex-direction: column; gap: 8px; min-width: 0; }
  .n { display: grid; place-items: center; width: 26px; height: 26px; border-radius: 50%; flex: none; font: 600 15px var(--font-label); box-shadow: inset 0 0 0 1.5px var(--ink-3); }
  .done .n { color: #fff; background: var(--ok); box-shadow: none; }
  .path { align-self: flex-start; }
  .how { margin: 0; padding-left: 18px; font-size: 13.5px; color: var(--ink-2); display: flex; flex-direction: column; gap: 4px; }
  .how b { color: var(--ink); font-weight: 600; }
  details { margin-top: 6px; }
  summary { font-size: 13.5px; color: var(--accent-text); }
  pre { max-height: 300px; overflow: auto; padding: 12px; border-radius: 10px; background: var(--well); font-size: 12px; white-space: pre-wrap; }
  .check-row em { overflow-wrap: anywhere; }
  .check-row { grid-template-columns: 70px 1fr; }
  .res { flex-direction: row !important; align-items: center; gap: 5px; color: var(--ok); font-weight: 600; }
  .res.bad { color: var(--heart); }
  @media (max-width: 900px) {
    .tabs { width: 190px; }
    .pane { padding: 22px 20px 48px; }
    .two { grid-template-columns: 1fr; }
  }
</style>
