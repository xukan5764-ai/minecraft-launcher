import { useEffect, useRef } from "react";
import { api, errorMessage, versionLabel } from "../api";
import { useApp } from "../state";

export default function HomePage() {
  const {
    installed,
    selectedVersion,
    setSelectedVersion,
    accounts,
    busy,
    runTask,
    logs,
    clearLogs,
    gameRunning,
    setGameRunning,
    notify,
  } = useApp();
  const account = accounts.find((a) => a.selected) ?? accounts[0];
  const logRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [logs]);

  const launch = () =>
    runTask(`启动 ${selectedVersion}`, async () => {
      await api.launchGame(selectedVersion);
      setGameRunning(true);
    });

  const canLaunch = !!selectedVersion && !!account && !busy && !gameRunning;

  return (
    <section className="page">
      <h1>启动游戏</h1>

      <div className="card launch-card">
        <label className="field">
          <span>版本</span>
          <select
            value={selectedVersion}
            onChange={(e) => setSelectedVersion(e.target.value)}
            disabled={installed.length === 0}
          >
            {installed.length === 0 && <option value="">还没有安装任何版本</option>}
            {installed.map((v) => (
              <option key={v.id} value={v.id}>
                {versionLabel(v)}
              </option>
            ))}
          </select>
        </label>

        <div className="field">
          <span>账号</span>
          <div className="account-chip">
            {account ? (
              <>
                {account.name}
                <span className="muted">{account.kind === "microsoft" ? "微软" : "离线"}</span>
              </>
            ) : (
              <span className="muted">请先到“账号”页添加</span>
            )}
          </div>
        </div>

        <div className="row">
          {gameRunning ? (
            <button
              className="danger big"
              onClick={() => api.killGame().catch((e) => notify("error", errorMessage(e)))}
            >
              结束游戏
            </button>
          ) : (
            <button className="primary big" onClick={launch} disabled={!canLaunch}>
              启动
            </button>
          )}
          <button
            onClick={() =>
              api.openGameDir(selectedVersion || undefined).catch((e) => notify("error", errorMessage(e)))
            }
          >
            打开游戏目录
          </button>
        </div>
      </div>

      <div className="log-header">
        <h2>日志</h2>
        <button className="link" onClick={clearLogs}>
          清空
        </button>
      </div>
      <pre className="log" ref={logRef}>
        {logs.length ? logs.join("\n") : "启动游戏后，这里会显示游戏输出。"}
      </pre>
    </section>
  );
}
