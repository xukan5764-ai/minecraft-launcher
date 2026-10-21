import { invoke } from "@tauri-apps/api/core";

export interface Config {
  gameDir: string;
  javaPath: string;
  maxMemoryMb: number;
  jvmArgs: string;
  downloadConcurrency: number;
  curseforgeApiKey: string;
  msClientId: string;
}

export interface SettingsView {
  config: Config;
  defaultGameDir: string;
  hasBuiltinCurseforgeKey: boolean;
  hasBuiltinMsClientId: boolean;
}

export interface ManifestEntry {
  id: string;
  type: "release" | "snapshot" | "old_beta" | "old_alpha";
  url: string;
  releaseTime: string;
}

export interface VersionManifest {
  latest: { release: string; snapshot: string };
  versions: ManifestEntry[];
}

export type Loader = "fabric" | "quilt" | "forge" | "neoforge";

export interface InstalledVersion {
  id: string;
  gameVersion: string;
  loader: Loader | null;
  loaderVersion: string | null;
}

export interface FabricLoader {
  version: string;
  stable: boolean;
}

export interface Account {
  id: string;
  kind: "microsoft" | "offline";
  name: string;
  uuid: string;
  selected: boolean;
}

export interface DeviceCode {
  deviceCode: string;
  userCode: string;
  verificationUri: string;
  expiresIn: number;
  interval: number;
}

export interface ModSummary {
  id: number;
  name: string;
  summary: string;
  downloadCount: number;
  iconUrl: string | null;
  authors: string[];
  websiteUrl: string | null;
}

export interface ProgressEvent {
  stage: string;
  done: number;
  total: number;
}

export const api = {
  getSettings: () => invoke<SettingsView>("get_settings"),
  saveSettings: (config: Config) => invoke<void>("save_settings", { config }),

  getVersionManifest: () => invoke<VersionManifest>("get_version_manifest"),
  listInstalledVersions: () => invoke<InstalledVersion[]>("list_installed_versions"),
  installVersion: (id: string) => invoke<void>("install_version", { id }),
  deleteVersion: (id: string) => invoke<void>("delete_version", { id }),
  getFabricLoaders: (gameVersion: string) =>
    invoke<FabricLoader[]>("get_fabric_loaders", { gameVersion }),
  installFabric: (gameVersion: string, loaderVersion: string) =>
    invoke<string>("install_fabric", { gameVersion, loaderVersion }),

  listAccounts: () => invoke<Account[]>("list_accounts"),
  addOfflineAccount: (name: string) => invoke<Account[]>("add_offline_account", { name }),
  removeAccount: (id: string) => invoke<Account[]>("remove_account", { id }),
  selectAccount: (id: string) => invoke<Account[]>("select_account", { id }),
  msLoginStart: () => invoke<DeviceCode>("ms_login_start"),
  msLoginFinish: (code: DeviceCode) => invoke<Account[]>("ms_login_finish", { code }),
  msLoginCancel: () => invoke<void>("ms_login_cancel"),

  launchGame: (versionId: string) => invoke<void>("launch_game", { versionId }),
  killGame: () => invoke<void>("kill_game"),

  cfSearch: (query: string, gameVersion: string | null, loader: string | null, index = 0) =>
    invoke<ModSummary[]>("cf_search", { query, gameVersion, loader, index }),
  cfInstall: (modId: number, versionId: string) =>
    invoke<string[]>("cf_install", { modId, versionId }),
  listMods: (versionId: string) => invoke<string[]>("list_mods", { versionId }),
  removeMod: (versionId: string, fileName: string) =>
    invoke<void>("remove_mod", { versionId, fileName }),
  openGameDir: (versionId?: string) => invoke<void>("open_game_dir", { versionId }),
};

/** Tauri rejects with the serialized backend error, which is a plain string. */
export function errorMessage(err: unknown): string {
  return typeof err === "string" ? err : err instanceof Error ? err.message : String(err);
}

export function versionLabel(v: InstalledVersion): string {
  return v.loader ? `${v.id} (${v.loader})` : v.id;
}
