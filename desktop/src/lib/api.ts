import { invoke, convertFileSrc } from '@tauri-apps/api/core';

export type Category = { path: string; name: string; depth: number; label: number | null; count: number };
export type Overview = {
  sort: number; categories: Category[]; counts: number[];
  pinned: { id: number; name: string }[]; lists: string[]; today: number;
  /** why the library can't be changed (damaged file); null when it can */
  locked: string | null;
};
/**
 * Something to tell about the library file. `kept` is the safety copy's path; it is empty when
 * the library file was missing (recovered) or no safety copy could be made yet (damaged).
 */
export type Notice = { kind: 'recovered' | 'replaced' | 'damaged'; file: string; kept: string };
export type Card = {
  id: number;
  name: string;
  rating: number;
  favorite: boolean;
  label: string;
  label_index: number | null;
  photo: string;
  seed: number;
  total_time: string;
  flags: number;
  pinned: boolean;
  in_trash: boolean;
};
export type Row = { amount: string | null; text: string; heading: boolean };
export type Detail = {
  card: Card;
  description: string; notes: string; source: string; source_url: string;
  prep_time: string; cook_time: string; servings: string; difficulty: string;
  rows: Row[]; steps: Piece[][]; video: string | null;
  per_serving: Nutrients; nutrition_edited: boolean; nutrition_counted: number; nutrition_skipped: number;
  flags: string[];
};
/** Step text; a piece with `seconds` is a time phrase that can start a timer. */
export type Piece = { text: string; seconds: number | null };
export type Draft = {
  name: string; rating: number; servings: string; prep_time: string; cook_time: string; total_time: string;
  difficulty: string; source: string; source_url: string; video_url: string; categories: string[];
  description: string; ingredients: string; directions: string; notes: string; photo: string; photo_seed: number;
};
export type Section = 'recipes' | 'browser' | 'groceries' | 'pantry' | 'foods' | 'meals' | 'menus' | 'settings';
export type Units = 'original' | 'metric' | 'us';

export const overview = () => invoke<Overview>('overview');
export const list = (view: string, search: string, sort: number) => invoke<Card[]>('list', { view, search, sort });
export const recipe = (id: number, scale: number, units: Units) => invoke<Detail>('recipe', { id, scale, units });
export const setFavorite = (id: number, on: boolean) => invoke<void>('set_favorite', { id, on });
export const setRating = (id: number, rating: number) => invoke<void>('set_rating', { id, rating });

export const draft = (id: number) => invoke<Draft>('draft', { id });
export const tidy = (draft: Draft) => invoke<[Draft, string[]]>('tidy', { draft });
export const saveRecipe = (id: number, draft: Draft) => invoke<number>('save_recipe', { id, draft });
export const openLink = (url: string) => invoke<void>('open_link', { url });
/** Sends the picture as raw bytes; returns its file name in the photos folder. */
export const addPhoto = async (file: File, stem: string) =>
  invoke<string>('add_photo', new Uint8Array(await file.arrayBuffer()), {
    headers: { stem: stem.toLowerCase().replace(/[^a-z0-9]+/g, '-').slice(0, 40) || 'photo' },
  });

export const setPinned = (id: number, on: boolean) => invoke<void>('set_pinned', { id, on });
export const setTrashed = (ids: number[], on: boolean) => invoke<void>('set_trashed', { ids, on });
export const deleteForever = (ids: number[]) => invoke<number>('delete_forever', { ids });
export const duplicate = (id: number) => invoke<number>('duplicate', { id });
/** Per-serving numbers typed in by hand (a missing key is unknown, not 0), or null to go back to the calculated estimate. */
export const setNutrition = (id: number, perServing: Nutrients | null) => invoke<void>('set_nutrition', { id, perServing });
export type MealType = 'Breakfast' | 'Lunch' | 'Dinner' | 'Snack';
/** No kind: the first meal of that day that's still free. */
export const addMeal = (recipeId: number, day: number, kind: MealType | null = null) => invoke<void>('add_meal', { recipeId, day, kind });
export type Meal = { id: number; day: number; kind: MealType; recipe_id: number | null; name: string; modified: number; label: number | null; photo: string; seed: number };
export const meals = (from: number, to: number) => invoke<Meal[]>('meals', { from, to });
export const updateMeal = (id: number, day: number | null, kind: MealType | null) => invoke<void>('update_meal', { id, day, kind });
/** Returns what was removed, for Undo. */
export const removeMeals = (ids: number[], from: number | null = null, to: number | null = null) => invoke<Meal[]>('remove_meals', { ids, from, to });
export const restoreMeals = (meals: Meal[]) => invoke<void>('restore_meals', { meals });

// ---- importing ----
export type Recipe = Record<string, unknown> & { name: string; servings: string; prep_time: string; cook_time: string; total_time: string; categories: string[]; notes: string };
export type Review = {
  recipe: Recipe; ing_lines: string[]; step_lines: string[]; method: string; tried: [string, string][];
  field_issues: string[]; duplicate: { id: number; name: string; score: number; reason: string } | null; merge: boolean;
  changes: string[]; rule_domain: string | null; rule_worked: boolean | null; existing_id: number | null; origin: string; image_url: string;
};
export type LineCheck = { ok: boolean; low: boolean; issues: string[]; parsed: string };
export type RuleRow = { domain: string; name: string; version: number; updated: boolean; verified: string; last_import: string | null; needs_update: boolean; test: [boolean, string] | null };
export const importUrl = (url: string) => invoke<Review>('import_url', { url });
export const importText = (text: string) => invoke<Review>('import_text', { text });
export const checkLines = (ing: string[], steps: string[]) => invoke<{ ing: LineCheck[]; steps: LineCheck[] }>('check_lines', { ing, steps });
export const reviewTidy = (review: Review) => invoke<Review>('review_tidy', { review });
export const reviewExisting = (id: number) => invoke<Review>('review_existing', { id });
export const saveReview = (review: Review) => invoke<{ id: number; message: string }>('save_review', { review });
export const takeInbox = () => invoke<{ reviews: Review[]; notes: string[] }>('take_inbox');
export const launchNotes = () => invoke<string[]>('launch_notes');
export const rulesOverview = (test: boolean) => invoke<{ rules: RuleRow[]; updated_any: boolean; inbox: string }>('rules_overview', { test });
export const loadRules = (text: string) => invoke<string>('load_rules', { text });
export const useBuiltinRules = () => invoke<void>('use_builtin_rules');
export const rulesFromSync = () => invoke<string>('rules_from_sync');

// ---- groceries ----
export type GroceryItem = { id: number; name: string; note: string; quantity: string; aisle: string; rank: number; recipe: string; purchased: boolean; label: number | null };
export type Groceries = {
  lists: { id: number; name: string; to_buy: number }[]; list: number; items: GroceryItem[]; recipes: string[];
  aisles: string[]; by_recipe: boolean; pantry: number; today: number;
};
export type Preview = { items: number; in_pantry: string[]; recipes: number };
export type ItemChange = { purchased?: boolean; aisle?: string; step?: number; name?: string; quantity?: string };
export const groceries = (list: number) => invoke<Groceries>('groceries', { list });
export const groceryPreview = (id: number, times: number, skip: boolean) => invoke<Preview>('grocery_preview', { id, times, skip });
export const addRecipeToList = (id: number, list: number, times: number, skip: boolean) => invoke<string>('add_recipe_to_list', { id, list, times, skip });
export const planPreview = (from: number, to: number, skip: boolean) => invoke<Preview>('plan_preview', { from, to, skip });
export const addPlanToList = (list: number, from: number, to: number, skip: boolean) => invoke<string>('add_plan_to_list', { list, from, to, skip });
export const addItem = (list: number, text: string) => invoke<void>('add_item', { list, text });
export const updateItem = (list: number, id: number, change: ItemChange) => invoke<void>('update_item', { list, id, change });
/** The removals below return the undo bar's message (null when nothing went). */
export const removeItem = (list: number, id: number) => invoke<string | null>('remove_item', { list, id });
export const newList = (name: string) => invoke<number>('new_list', { name });
export const renameList = (list: number, name: string) => invoke<void>('rename_list', { list, name });
export const deleteList = (list: number) => invoke<string | null>('delete_list', { list });
export const clearItems = (list: number, allLists: boolean, pantry: boolean, which: number) => invoke<string | null>('clear_items', { list, allLists, pantry, which });
export const removePantryItems = (list: number) => invoke<string | null>('remove_pantry_items', { list });
export const moveBoughtToPantry = (list: number) => invoke<string | null>('move_bought_to_pantry', { list });
export const undoGroceries = () => invoke<void>('undo_groceries');
export const setGroceryByRecipe = (on: boolean) => invoke<void>('set_grocery_by_recipe', { on });

// ---- settings ----
export type SettingsInfo = {
  version: string; sync_folder: string; auto_sync: boolean; device_name: string; last_sync: string | null;
  onedrive: string | null; dropbox: string | null; backups_dir: string;
  tracker_service: string; tracker_folder: string; tracker_default: string; tracker_auto_send: boolean;
  /** the contact email Open Food Facts is told about; empty means the next online search asks */
  off_email: string;
  services: string[]; service_url: string;
  inbox_dir: string; inbox_ready: boolean; instructions: string; recipes: number; favorites: number;
};
export type SettingsChange = Partial<{ sync_folder: string; auto_sync: boolean; device_name: string; tracker_service: string; tracker_folder: string; tracker_auto_send: boolean }>;
export type SavedVersion = { path: string; when: string; recipes: number; reason: string; favorites: number; has_diary: boolean };
export type MyFood = { id: number; name: string; aliases: string; serving: string; serving_g: number; serving_ml: number; per_serving: Nutrients; piece: string; piece_g: number; source: string; modified: number };
export const settingsInfo = () => invoke<SettingsInfo>('settings_info');
export const setSettings = (change: SettingsChange) => invoke<void>('set_settings', { change });
export const syncNow = () => invoke<string>('sync_now');
/** Returns null when sync isn't set up or the library can't be changed. */
export const autoSync = () => invoke<string | null>('auto_sync');
export const backupNow = () => invoke<string>('backup_now');
export const backups = () => invoke<SavedVersion[]>('backups');
/** The same command, asked about one chosen file (its text): one row, with has_diary decided by the crate. */
export const pickedBackup = (text: string) => invoke<SavedVersion[]>('backups', { picked: text });
export const snapshots = () => invoke<SavedVersion[]>('snapshots');
export const takeSnapshot = () => invoke<void>('take_snapshot');
export const restoreBackup = (path: string | null, text: string | null = null, scope: 'recipes' | 'everything' = 'recipes') => invoke<string>('restore_backup', { path, text, scope });
export const rollback = (path: string) => invoke<string>('rollback', { path });
export const pendingNotice = () => invoke<Notice | null>('pending_notice');
export const startEmpty = () => invoke<string>('start_empty');
export const sendAllToTracker = () => invoke<string>('send_all_to_tracker');
export const setupInbox = () => invoke<string>('setup_inbox');
export const openFolder = (which: 'inbox' | 'backups' | 'tracker') => invoke<void>('open_folder', { which });
/** A new, empty food: numbers are 0 only because the type needs a number; the form shows them as blank boxes. */
export const blankFood = (name = ''): MyFood => ({ id: 0, name, aliases: '', serving: '', serving_g: 0, serving_ml: 0, per_serving: {}, piece: '', piece_g: 0, source: '', modified: 0 });
export const myFoods = () => invoke<MyFood[]>('my_foods');
export const saveMyFood = (food: MyFood) => invoke<void>('save_my_food', { food });
export const deleteMyFood = (id: number) => invoke<void>('delete_my_food', { id });
/** What a product link, barcode or label photo filled in. */
export type LabelGuess = { food: MyFood; source: string; missing: string[] };
export const labelLookup = (input: string) => invoke<LabelGuess>('label_lookup', { input });
/** The error a barcode lookup rejects with when no contact email is saved (mirrors `foods::off::NEEDS_EMAIL`). */
export const NEEDS_EMAIL = 'needs_email';
/** Saves, changes or clears (empty text) the contact email; rejects with the crate's message when it isn't an email address. */
export const setOffEmail = (email: string) => invoke<void>('set_off_email', { email });
/** Sends the photo as raw bytes; read offline with Windows' text recognition. */
export const labelPhoto = async (file: File) =>
  invoke<LabelGuess>('label_photo', new Uint8Array(await file.arrayBuffer()), { headers: { name: file.name.replace(/[^\w .-]/g, '').slice(0, 60) || 'label' } });
export type Check = { group: string; name: string; passed: boolean; detail: string };
export const selfTests = () => invoke<Check[]>('self_tests');
export const mergeDuplicates = () => invoke<string>('merge_duplicates');

// ---- foods ----
/** A missing key is unknown, never 0. */
export type Nutrients = Partial<Record<'calories' | 'protein_g' | 'fat_g' | 'carbs_g' | 'fiber_g' | 'sugar_g' | 'sodium_mg', number | null>>;
export type FoodTag = 'My Food' | 'USDA' | 'Canada' | 'Open Food Facts';
export type FoodRow = { key: string; name: string; brand: string; tag: FoodTag; kcal: number | null; protein: number | null; carbs: number | null; fat: number | null; basis: string };
export type OnlineStatus = 'ok' | 'empty' | 'offline' | 'slow' | 'limited' | 'wait' | 'needs_email';
export type OnlineResult = { status: OnlineStatus; retry_in: number; added: number; rows: FoodRow[] };
export type Unit = { id: string; label: string; grams: number };
export type FoodDetail = { key: string; name: string; brand: string; tag: FoodTag; online: boolean; suspect: boolean; units: Unit[]; unit: string; amount: number };
export type Amount = { amount: number | null; grams: number | null; nutrients: Nutrients };
export type FoodPreview = { per100: Nutrients | null; heavy: boolean; dense: boolean; problem: { field: string; message: string } | null };
/** `online_ready` is the crate's rule for when Search Online may be pressed. */
export type FoodResults = { rows: FoodRow[]; online_ready: boolean };
/** Local search on every keystroke; never reaches the network. */
export const searchFoods = (q: string) => invoke<FoodResults>('search_foods', { q });
export const foodDetail = (key: string, us: boolean) => invoke<FoodDetail>('food_detail', { key, us });
/** `from` is the unit the amount was typed in when the unit changes; the crate converts and rounds. */
export const foodAmount = (key: string, amount: number | null, unit: string, from: string | null = null) => invoke<Amount>('food_amount', { key, amount, unit, from });
export const foodPreview = (food: MyFood) => invoke<FoodPreview>('food_preview', { food });
/** Keeps an opened online product for offline search. */
export const offOpen = (code: string) => invoke<void>('off_open', { code });
export const onlineFoodDraft = (code: string) => invoke<{ food: MyFood; ready: boolean }>('online_food_draft', { code });
/** Online search: only when the person presses Search Online, never while typing. */
export const offSearch = (q: string) => invoke<OnlineResult>('off_search', { q });

// ---- menus ----
export type MenuEntry = { day: number; kind: MealType; recipe_id: number | null; name: string };
export type MenuEntryOut = MenuEntry & { label: number | null; photo: string; seed: number };
export type MenuOut = { id: number; name: string; days: number; entries: MenuEntryOut[] };
export const menus = () => invoke<MenuOut[]>('menus');
export const newMenu = () => invoke<number>('new_menu');
export const saveMenu = (id: number, name: string, days: number, entries: MenuEntry[]) => invoke<void>('save_menu', { id, name, days, entries });
/** Returns the deleted menu, for Undo. */
export const deleteMenu = (id: number) => invoke<unknown>('delete_menu', { id });
export const restoreMenu = (menu: unknown) => invoke<void>('restore_menu', { menu });
/** Day 1 lands on `start`; returns how many meals were added. */
export const applyMenu = (id: number, start: number) => invoke<number>('apply_menu', { id, start });

// ---- pantry ----
export type Expiry = 'expired' | 'today' | 'week' | 'month' | 'later';
export type PantryItem = {
  id: number; name: string; quantity: string; aisle: string; rank: number;
  bought: number | null; expires: number | null; in_stock: boolean; expiry: Expiry | null;
};
export type Pantry = { items: PantryItem[]; sort: number; aisles: string[]; lists: [number, string][]; today: number };
export type PantryEdit = Pick<PantryItem, 'id' | 'name' | 'quantity' | 'aisle' | 'bought' | 'expires' | 'in_stock'>;
export const pantry = () => invoke<Pantry>('pantry');
export const addPantry = (text: string) => invoke<void>('add_pantry', { text });
export const updatePantry = (item: PantryEdit) => invoke<void>('update_pantry', { item });
export const deletePantry = (id: number) => invoke<string | null>('delete_pantry', { id });
export const removeExpired = () => invoke<string | null>('remove_expired');
export const removeOutOfStock = () => invoke<string | null>('remove_out_of_stock');
export const pantryToList = (ids: number[], list: number) => invoke<string>('pantry_to_list', { ids, list });
export const setPantrySort = (sort: number) => invoke<void>('set_pantry_sort', { sort });
/** Day number ↔ the yyyy-mm-dd a date field uses. */
export const dayToIso = (day: number | null) => (day == null ? '' : new Date(day * 86400000).toISOString().slice(0, 10));
export const isoToDay = (iso: string) => (iso ? Math.round(Date.parse(iso + 'T00:00:00Z') / 86400000) : null);

/** Days since 1970-01-01 → "Mon, Nov 16" (the library stores meal days this way). */
export const dayLabel = (day: number, opts: Intl.DateTimeFormatOptions = { weekday: 'short', month: 'short', day: 'numeric' }) =>
  new Date(day * 86400000).toLocaleDateString(undefined, { ...opts, timeZone: 'UTC' });

export const photoUrl = (name: string) => convertFileSrc(name, 'photo');

/** CSS colour for a label index (null = unlabelled). */
export const labelVar = (i: number | null) => (i == null ? 'var(--l-none)' : `var(--l${i})`);

export type UpdateInfo = { version: string; short: string; notes: string };
export const checkUpdate = () => invoke<UpdateInfo | null>('check_update');
export const installUpdate = () => invoke<void>('install_update');
/** Tells Rust a page holds typed input that is not saved yet; the install refuses while it is true. */
export const setUnsaved = (unsaved: boolean) => invoke<void>('set_unsaved', { unsaved });
