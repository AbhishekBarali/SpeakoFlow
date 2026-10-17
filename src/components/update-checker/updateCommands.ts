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

/** Leaves a note so the new version knows it was just updated (`updates.rs`). */
export const prepareUpdateInstall = (version: string) =>
  invoke<void>("prepare_update_install", { version });

/** Progress of `downloadUpdate`, as an `InstallerProgress`. */
export const UPDATE_PROGRESS_EVENT = "update-download-progress";

/**
 * Download the update on several connections and verify its signature.
 * Resolves to the version downloaded. Replaces the plugin's
 * `update.download()`, which used one connection and ran at a tenth of the
 * speed against GitHub's per-connection throttle.
 */
export const downloadUpdate = () => invoke<string>("download_update");

/** Install what `downloadUpdate` fetched. On Windows the app exits here. */
export const installDownloadedUpdate = () =>
  invoke<void>("install_downloaded_update");

export interface FinishedUpdate {
  from: string;
  to: string;
}

/** The update this launch finished, if any. Returned once. */
export const takeUpdateNotice = () =>
  invoke<FinishedUpdate | null>("take_update_notice");
