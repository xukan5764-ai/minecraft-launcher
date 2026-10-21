import { listen } from "@tauri-apps/api/event";
import { createContext, ReactNode, useCallback, useContext, useEffect, useState } from "react";
import { Account, api, errorMessage, InstalledVersion, ProgressEvent } from "./api";

const MAX_LOG_LINES = 3000;
const SELECTED_VERSION_KEY = "selectedVersion";

interface AppContextValue {
  installed: InstalledVersion[];
  refreshInstalled: () => Promise<void>;
  selectedVersion: string;
  setSelectedVersion: (id: string) => void;
  accounts: Account[];
  setAccounts: (accounts: Account[]) => void;
  /** Label of the long-running task in progress, if any. */
  busy: string | null;
  progress: ProgressEvent | null;
  /** Runs a long task with a busy label; errors become a notice. Returns false on failure. */
  runTask: (label: string, task: () => Promise<unknown>) => Promise<boolean>;
  logs: string[];
  clearLogs: () => void;
  gameRunning: boolean;
  setGameRunning: (running: boolean) => void;
  notice: { kind: "error" | "info"; text: string } | null;
  notify: (kind: "error" | "info", text: string) => void;
  dismissNotice: () => void;
}

const AppContext = createContext<AppContextValue | null>(null);

function readStoredVersion(): string {
  try {
    return localStorage.getItem(SELECTED_VERSION_KEY) ?? "";
  } catch {
    return "";
  }
}

export function AppProvider({ children }: { children: ReactNode }) {
  const [installed, setInstalled] = useState<InstalledVersion[]>([]);
  const [selectedVersion, setSelectedVersionState] = useState(readStoredVersion);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [progress, setProgress] = useState<ProgressEvent | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [gameRunning, setGameRunning] = useState(false);
  const [notice, setNotice] = useState<AppContextValue["notice"]>(null);

  const notify = useCallback((kind: "error" | "info", text: string) => setNotice({ kind, text }), []);

  const setSelectedVersion = useCallback((id: string) => {
    setSelectedVersionState(id);
    try {
      localStorage.setItem(SELECTED_VERSION_KEY, id);
    } catch {
      // Storage may be unavailable; the selection just won't be remembered.
    }
  }, []);

  const refreshInstalled = useCallback(async () => {
    const list = await api.listInstalledVersions();
    setInstalled(list);
    setSelectedVersionState((current) =>
      list.some((v) => v.id === current) ? current : (list[0]?.id ?? ""),
    );
  }, []);

  const runTask = useCallback(
    async (label: string, task: () => Promise<unknown>) => {
      setBusy(label);
      setProgress(null);
      try {
        await task();
        return true;
      } catch (err) {
        notify("error", errorMessage(err));
        return false;
      } finally {
        setBusy(null);
        setProgress(null);
      }
    },
    [notify],
  );

  useEffect(() => {
    refreshInstalled().catch((err) => notify("error", errorMessage(err)));
    api.listAccounts().then(setAccounts).catch((err) => notify("error", errorMessage(err)));

    const unlisten = [
      listen<ProgressEvent>("install-progress", (e) => setProgress(e.payload)),
      listen<string>("game-log", (e) =>
        setLogs((prev) => {
          const next = prev.length >= MAX_LOG_LINES ? prev.slice(-MAX_LOG_LINES + 1) : prev.slice();
          next.push(e.payload);
          return next;
        }),
      ),
      listen<number | null>("game-exit", () => setGameRunning(false)),
    ];
    return () => {
      unlisten.forEach((p) => p.then((off) => off()));
    };
  }, [refreshInstalled, notify]);

  const value: AppContextValue = {
    installed,
    refreshInstalled,
    selectedVersion,
    setSelectedVersion,
    accounts,
    setAccounts,
    busy,
    progress,
    runTask,
    logs,
    clearLogs: () => setLogs([]),
    gameRunning,
    setGameRunning,
    notice,
    notify,
    dismissNotice: () => setNotice(null),
  };
  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): AppContextValue {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp must be used inside AppProvider");
  return ctx;
}
