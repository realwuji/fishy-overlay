// Reactive stores + settings persistence (Tauri set_settings / browser localStorage).
import { DEFAULT_SETTINGS } from '../shared/defaults';
import { call, inTauri, on } from '../shared/ipc';
import { EVT, type Settings } from '../shared/types';
import { clone, debounce } from './dom';
import { jsonEqual, sanitize } from './sanitize';

export interface Store<T> {
  get(): T;
  set(v: T): void;
  subscribe(fn: (v: T) => void): () => void;
}

export function createStore<T>(initial: T): Store<T> {
  let v = initial;
  const subs = new Set<(v: T) => void>();
  return {
    get: () => v,
    set(n) { v = n; subs.forEach((f) => f(v)); },
    subscribe(fn) { subs.add(fn); return () => subs.delete(fn); },
  };
}

const LS_KEY = 'fishy.control.settings.v1';

export type SaveState = 'idle' | 'saving' | 'saved' | 'error';
export const saveState = createStore<SaveState>('idle');

const settingsStore = createStore<Settings>(clone(DEFAULT_SETTINGS));
let dirty = false;      // local edits not yet confirmed by the backend
let inFlight = false;

export const getSettings = (): Settings => settingsStore.get();
export const subscribeSettings = (fn: (s: Settings) => void): (() => void) => settingsStore.subscribe(fn);

async function persist(): Promise<void> {
  if (inFlight) { dirty = true; return; }
  inFlight = true;
  dirty = false;
  saveState.set('saving');
  try {
    const snapshot = settingsStore.get();
    if (inTauri()) {
      const echoed = sanitize(await call('set_settings', { settings: snapshot }));
      // Adopt the backend's normalised copy only when nothing newer was typed meanwhile.
      if (!dirty && !jsonEqual(echoed, settingsStore.get())) settingsStore.set(echoed);
    } else {
      try { localStorage.setItem(LS_KEY, JSON.stringify(snapshot)); } catch { /* storage may be unavailable */ }
    }
    saveState.set('saved');
  } catch (err) {
    console.error('set_settings failed', err);
    saveState.set('error');
  } finally {
    inFlight = false;
    if (dirty) void persist();
  }
}

const schedule = debounce(() => { void persist(); }, 250);

/** Immutable edit: mutate a deep clone, validate/clamp, publish, auto-save (debounced 250ms). */
export function edit(mutate: (draft: Settings) => void): void {
  const draft = clone(settingsStore.get());
  mutate(draft);
  const next = sanitize(draft);
  if (jsonEqual(next, settingsStore.get())) return;
  dirty = true;
  settingsStore.set(next);
  saveState.set('saving');
  schedule();
}

/** Replace the whole settings object (preset, import, reset, undo) and save. */
export function replaceSettings(next: unknown): void {
  const s = sanitize(next);
  if (jsonEqual(s, settingsStore.get())) return;
  dirty = true;
  settingsStore.set(s);
  saveState.set('saving');
  schedule();
}

export function flushSettings(): void { schedule.flush(); }

/** Called on Rust pushes; ignored while local edits are still pending so sliders never jump back. */
function applyRemote(payload: unknown): void {
  if (dirty || inFlight) return;
  const s = sanitize(payload);
  if (!jsonEqual(s, settingsStore.get())) settingsStore.set(s);
}

export async function initSettings(): Promise<void> {
  if (inTauri()) {
    try {
      settingsStore.set(sanitize(await call('get_settings')));
    } catch (err) {
      console.error('get_settings failed', err);
    }
    await on<Settings>(EVT.settings, applyRemote);
  } else {
    try {
      const raw = localStorage.getItem(LS_KEY);
      if (raw) settingsStore.set(sanitize(JSON.parse(raw)));
    } catch { /* ignore corrupt storage */ }
  }
  window.addEventListener('beforeunload', flushSettings);
}

/** Reset everything to defaults. The fair-play acknowledgement is kept (user already gave it on this device). */
export async function resetAllSettings(): Promise<void> {
  const keep = settingsStore.get().acceptedFairPlay;
  let fresh: Settings = clone(DEFAULT_SETTINGS);
  if (inTauri()) {
    try { fresh = sanitize(await call('reset_settings')); } catch (err) { console.error(err); }
  }
  fresh.acceptedFairPlay = keep;
  dirty = true;
  settingsStore.set(sanitize(fresh));
  saveState.set('saving');
  schedule();
}
