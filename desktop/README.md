# Tiffin (new design)

The Tauri version of Tiffin: Svelte screens in `src/`, a thin Rust layer in `src-tauri/`
that uses all the logic from the `recipe_box` crate in `../app`.

Ported so far:

- **Recipes**: shelves, categories, pinned recipes, search, grid and list, right-click menu; the
  recipe page with scaling, units, rating, favorites, nutrition (also typed in by hand); new and
  edit (photo by button or drag-and-drop, Tidy formatting); duplicate; Trash with Undo, Put Back,
  Delete Forever and Empty Trash; Add to Meal Plan; Add to Grocery List; cook mode and timers.
- **Browser**: import by web address or pasted text, the review step (Fix All Formatting,
  duplicate merge), site rules and rules update files, and the recipe inbox folder.
- **Groceries**: lists, quick add, tick off, amounts ±1, sections, group by section or recipe,
  filters, add from the meal plan, pantry clean-up, clearing, all removals with Undo.

- **Pantry**: quick add, in/out of stock, bought and expiry dates, a "Use soon" strip, sorting
  and bands, out-of-stock items to a grocery list, removals with Undo.

- **Meals**: day, week and month views; drag a recipe from the picker onto a day (or pick one
  and click a day); drag meals between days; change breakfast/lunch/dinner/snack; remove or
  clear with Undo; make a grocery list for the days shown.

- **Menus**: reusable sets of meals over 1–28 days; add recipes to a day, drag them between
  days, rename, and put the menu on the meal plan from any start date (also from Meals ⋯).
  Deleting a menu has Undo.

- **Settings**: sync folder (and automatic sync every 5 minutes), one-file backup and restore,
  Version History rollback, health tracker export, the Claude inbox, update channel.

- **Foods**: search the bundled food tables and (on request) Open Food Facts; My Foods (your own
  foods, typed in or filled from a link, barcode or label photo) are added, edited and deleted here.

On Windows PowerShell, use `npm.cmd` if `npm` is blocked by the script policy.

## Run

```
npm install
npm run tauri dev                              # your real library
npx tauri dev -- -- --data-dir <folder>        # another library folder (e.g. a copy of app/shots/demo-data)
npm run tauri build                            # installer in src-tauri/target/release/bundle
```

Command-line flags (after `--` when started through `tauri dev`; directly on the built
`tiffin.exe`):

```
--import <file or link>    drop a recipe file or web link in the inbox for the open window (or the next start) and quit
--self-test                run the built-in release checks, print each one, exit 1 if any fails
--data-dir <folder>        use another library folder
```

## Building and releasing

- **Local installer** (no signing key needed, no updater files): `npm --prefix desktop run tauri -- build`.
  Updater artifacts are switched on only by `src-tauri/tauri.release.conf.json`, which only the release workflow passes.
- **A release**: set the same version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `../app/Cargo.toml`,
  run `node desktop/tools/check-versions.mjs`, then `git tag -a vX.Y.Z -m "<release notes>"` and `git push origin vX.Y.Z`.
  `.github/workflows/release.yml` checks that the tag and versions agree, runs both crates' tests, builds the NSIS
  installer, signs it and publishes the release with `latest.json`. The tag message becomes the release notes.
- **Signing key**: it lives only in the repo's Actions secrets (`TAURI_SIGNING_PRIVATE_KEY`,
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) plus the owner's offline backup. Losing it strands installed copies:
  they stop accepting updates. `*.key` is git-ignored.

## Data sources

USDA FoodData Central (public domain), Health Canada Canadian Nutrient File (Open Government Licence - Canada),
Open Food Facts (ODbL, online lookups), Barlow font (OFL).
