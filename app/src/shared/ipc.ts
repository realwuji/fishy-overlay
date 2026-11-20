// Thin typed wrapper over Tauri IPC with a browser fallback so both windows can
// be developed/previewed in a plain browser (vite dev) with demo data.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { Commands } from './types';

export const inTauri = (): boolean =>
  typeof (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !== 'undefined';

type Args<K extends keyof Commands> = Parameters<Commands[K]> extends [infer A] ? A : undefined;

/** Typed invoke. Throws (rejects) with the Rust error string on failure. */
export async function call<K extends keyof Commands>(
  cmd: K, ...args: Parameters<Commands[K]> extends [] ? [] : [Args<K>]
): Promise<ReturnType<Commands[K]>> {
  if (!inTauri()) throw new Error(`not running inside Tauri (${cmd})`);
  return invoke(cmd as string, (args[0] ?? {}) as Record<string, unknown>);
}

/** Subscribe to a Rust event; no-op unsubscribe in a plain browser. */
export async function on<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  if (!inTauri()) return () => {};
  return listen<T>(event, (e) => handler(e.payload));
}
