// The Targets page's unsaved draft. It lives here, not in the page, so leaving the page and coming back does not
// drop what was typed (R-02) and no leave-guard dialog is needed. Nothing here is written to disk: Save Targets does that.
import type { TargetsDraft, Template } from './api';

export const targets: { draft: TargetsDraft | null; saved: string } = $state({ draft: null, saved: '' });

/** The draft differs from what is saved: the save bar shows, and an update install waits. */
export function targetsDirty(): boolean {
  return targets.draft !== null && JSON.stringify(targets.draft) !== targets.saved;
}

/** Make a fresh view's draft both the saved copy and the working copy. */
export function adoptSaved(draft: TargetsDraft) {
  targets.saved = JSON.stringify(draft);
  targets.draft = JSON.parse(targets.saved);
}

/** Drop unsaved edits (no confirm: it only drops typed text). */
export function discard() {
  if (targets.saved) targets.draft = JSON.parse(targets.saved);
}

// shortcut: a macro box left empty is sent as -1 and an unusable calorie change as 1,000,000, so the crate answers with
// its own sentence ("Enter a number of 0 or more for protein.", "Enter a change from -2,000 to 2,000 kcal.") instead of
// the page inventing one; the wire type has no way to say "empty". Replace with an optional field when the crate grows one.
export const EMPTY = -1;
export const BAD_OFFSET = 1_000_000;

/** The next unused template id. */
export function nextTemplateId(templates: Template[]): number {
  return templates.reduce((top, t) => Math.max(top, t.id), 0) + 1;
}

/** Add a template, or replace the one with the same id. */
export function putTemplate(d: TargetsDraft, t: Template) {
  const at = d.templates.findIndex((x) => x.id === t.id);
  if (at < 0) d.templates.push(t);
  else d.templates[at] = t;
}

/** Remove a template; every weekday that used it goes back to the default plan. */
export function dropTemplate(d: TargetsDraft, id: number) {
  d.templates = d.templates.filter((t) => t.id !== id);
  d.week = d.week.map((w) => (w === id ? null : w));
}
