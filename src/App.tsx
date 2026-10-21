import { useState } from "react";
import "./App.css";
import AccountsPage from "./pages/AccountsPage";
import HomePage from "./pages/HomePage";
import ModsPage from "./pages/ModsPage";
import SettingsPage from "./pages/SettingsPage";
import VersionsPage from "./pages/VersionsPage";
import { AppProvider, useApp } from "./state";

const PAGES = [
  { id: "home", label: "启动", component: HomePage },
  { id: "versions", label: "版本", component: VersionsPage },
  { id: "mods", label: "模组", component: ModsPage },
  { id: "accounts", label: "账号", component: AccountsPage },
  { id: "settings", label: "设置", component: SettingsPage },
] as const;

type PageId = (typeof PAGES)[number]["id"];

function Shell() {
  const [page, setPage] = useState<PageId>("home");
  const { busy, progress, notice, dismissNotice } = useApp();
  const navigate = (id: PageId) => {
    dismissNotice();
    setPage(id);
  };
  const Current = PAGES.find((p) => p.id === page)!.component;
  const percent = progress && progress.total > 0 ? (progress.done / progress.total) * 100 : 0;

  return (
    <div className="shell">
      <nav className="sidebar">
        <div className="brand">MC Launcher</div>
        {PAGES.map((p) => (
          <button
            key={p.id}
            className={`nav-item${p.id === page ? " active" : ""}`}
            onClick={() => navigate(p.id)}
          >
            {p.label}
          </button>
        ))}
      </nav>

      <main className="content">
        {notice && (
          <div className={`notice ${notice.kind}`} role="alert">
            <span>{notice.text}</span>
            <button className="link" onClick={dismissNotice} aria-label="关闭提示">
              ×
            </button>
          </div>
        )}
        <Current />
      </main>

      {busy && (
        <footer className="taskbar">
          <div className="task-label">
            {busy}
            {progress && (
              <span className="muted">
                {" "}
                · {progress.stage} {progress.done}/{progress.total}
              </span>
            )}
          </div>
          <div className="progress">
            <div className="progress-fill" style={{ width: `${percent}%` }} />
          </div>
        </footer>
      )}
    </div>
  );
}

export default function App() {
  return (
    <AppProvider>
      <Shell />
    </AppProvider>
  );
}
