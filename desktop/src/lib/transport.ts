// Desktop transport (Tauri). Swapped for src/web/transport.ts by vite.web.config.ts in web builds.
import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';

export const isWeb = false;
export { invoke };

export type StorageStatus = { persisted: boolean | null; lastBackupAt: number | null };
export type InstallOutcome = 'installed' | 'cancelled' | 'unavailable' | 'error';
export const storageStatus = async (): Promise<StorageStatus> => ({ persisted: true, lastBackupAt: null });
export const requestPersistentStorage = async (): Promise<boolean> => true;
export const subscribeInstall = (fn: (available: boolean) => void): (() => void) => {
  fn(false);
  return () => {};
};
export const triggerInstall = async (): Promise<InstallOutcome> => 'unavailable';

export const photoUrl = (name: string) => convertFileSrc(name, 'photo');
export const loadPhotoUrl = async (name: string) => convertFileSrc(name, 'photo');

export const setWindowTheme = (theme: 'light' | 'dark' | null) =>
  Promise.resolve().then(() => getCurrentWindow().setTheme(theme)).catch(() => {});
