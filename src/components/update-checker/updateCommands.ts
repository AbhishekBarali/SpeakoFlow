import { invoke } from "@tauri-apps/api/core";

/**
 * Typed wrappers for the update commands in `src-tauri/src/updates.rs`.
 *
 * Called through `invoke` rather than the generated `commands.*` because
 * `bindings.ts` is only regenerated while the app runs in dev; the shapes here
 * mirror the Rust types by hand.
 */

/** How this install gets a new version (`UpdateInstallMode`). */
export type UpdateMode = "in_app" | "download" | "portable" | "package_manager";

export interface UpdateSupport {
  mode: UpdateMode;
  release_page: string;
}

export interface DownloadedInstaller {
  path: string;
  version: string;
}

export interface InstallerProgress {
  downloaded: number;
  total: number | null;
}

export const INSTALLER_PROGRESS_EVENT = "update-installer-progress";

export const getUpdateSupport = () =>
  invoke<UpdateSupport>("get_update_support");

export const downloadUpdateInstaller = () =>
  invoke<DownloadedInstaller>("download_update_installer");

export const openUpdateInstaller = (path: string) =>
  invoke<void>("open_update_installer", { path });

export const revealUpdateInstaller = (path: string) =>
  invoke<void>("reveal_update_installer", { path });
