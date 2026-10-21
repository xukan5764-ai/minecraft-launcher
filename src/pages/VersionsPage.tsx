import { useEffect, useMemo, useState } from "react";
import { api, errorMessage, FabricLoader, InstalledVersion, ManifestEntry } from "../api";
import { useApp } from "../state";

const TYPE_LABELS: Record<ManifestEntry["type"], string> = {
  release: "正式版",
  snapshot: "快照",
  old_beta: "远古 Beta",
  old_alpha: "远古 Alpha",
};

export default function VersionsPage() {
  const { installed, refreshInstalled, busy, runTask, notify } = useApp();
  const [manifest, setManifest] = useState<ManifestEntry[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [types, setTypes] = useState<Set<ManifestEntry["type"]>>(new Set(["release"]));
  const [fabricFor, setFabricFor] = useState<string | null>(null);

  useEffect(() => {
    api
      .getVersionManifest()
      .then((m) => setManifest(m.versions))
      .catch((e) => setLoadError(errorMessage(e)));
  }, []);

  const installedIds = useMemo(() => new Set(installed.map((v) => v.id)), [installed]);
  const visible = useMemo(
    () =>
      (manifest ?? []).filter((v) => types.has(v.type) && v.id.includes(filter.trim())).slice(0, 200),
    [manifest, types, filter],
  );

  const toggleType = (type: ManifestEntry["type"]) =>
    setTypes((prev) => {
      const next = new Set(prev);
      if (next.has(type)) next.delete(type);
      else next.add(type);
      return next;
    });

  const install = (id: string) =>
    runTask(`安装 ${id}`, async () => {
      await api.installVersion(id);
      await refreshInstalled();
      notify("info", `${id} 安装完成`);
    });

  const remove = async (v: InstalledVersion) => {
    if (!confirm(`确定删除 ${v.id}？该版本目录下的存档和模组也会一并删除。`)) return;
    try {
      await api.deleteVersion(v.id);
      await refreshInstalled();
    } catch (e) {
      notify("error", errorMessage(e));
    }
  };

  return (
    <section className="page">
      <h1>版本</h1>

      <h2>已安装</h2>
      {installed.length === 0 ? (
        <p className="muted">还没有安装任何版本，从下面的列表里选一个安装。</p>
      ) : (
        <ul className="list">
          {installed.map((v) => (
            <li key={v.id} className="list-item">
              <div className="grow">
                <div>{v.id}</div>
                <div className="muted small">
                  {v.loader ? `${v.loader} ${v.loaderVersion ?? ""} · ` : "原版 · "}
                  Minecraft {v.gameVersion}
                </div>
              </div>
              {!v.loader && (
                <button onClick={() => setFabricFor(fabricFor === v.id ? null : v.id)} disabled={!!busy}>
                  安装 Fabric
                </button>
              )}
              <button onClick={() => api.openGameDir(v.id).catch((e) => notify("error", errorMessage(e)))}>
                打开目录
              </button>
              <button className="danger" onClick={() => remove(v)} disabled={!!busy}>
                删除
              </button>
              {fabricFor === v.id && (
                <FabricInstaller gameVersion={v.gameVersion} onDone={() => setFabricFor(null)} />
              )}
            </li>
          ))}
        </ul>
      )}

      <h2>可安装</h2>
      <div className="row">
        <input placeholder="搜索版本号，如 1.20.1" value={filter} onChange={(e) => setFilter(e.target.value)} />
        {(Object.keys(TYPE_LABELS) as ManifestEntry["type"][]).map((t) => (
          <label key={t} className="checkbox">
            <input type="checkbox" checked={types.has(t)} onChange={() => toggleType(t)} />
            {TYPE_LABELS[t]}
          </label>
        ))}
      </div>
      {loadError && <p className="error-text">获取版本列表失败：{loadError}</p>}
      {!manifest && !loadError && <p className="muted">正在获取版本列表...</p>}
      <ul className="list">
        {visible.map((v) => (
          <li key={v.id} className="list-item">
            <div className="grow">
              <div>{v.id}</div>
              <div className="muted small">
                {TYPE_LABELS[v.type]} · {v.releaseTime.slice(0, 10)}
              </div>
            </div>
            <button
              className="primary"
              onClick={() => install(v.id)}
              disabled={!!busy || installedIds.has(v.id)}
            >
              {installedIds.has(v.id) ? "已安装" : "安装"}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

function FabricInstaller({ gameVersion, onDone }: { gameVersion: string; onDone: () => void }) {
  const { refreshInstalled, runTask, notify, busy, setSelectedVersion } = useApp();
  const [loaders, setLoaders] = useState<FabricLoader[] | null>(null);
  const [loaderVersion, setLoaderVersion] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .getFabricLoaders(gameVersion)
      .then((list) => {
        setLoaders(list);
        setLoaderVersion((list.find((l) => l.stable) ?? list[0])?.version ?? "");
      })
      .catch((e) => setError(errorMessage(e)));
  }, [gameVersion]);

  const install = () =>
    runTask(`安装 Fabric ${loaderVersion}`, async () => {
      const id = await api.installFabric(gameVersion, loaderVersion);
      await refreshInstalled();
      setSelectedVersion(id);
      notify("info", `${id} 安装完成`);
      onDone();
    });

  if (error) return <p className="error-text full">获取 Fabric 版本失败：{error}</p>;
  if (!loaders) return <p className="muted full">正在获取 Fabric 版本...</p>;
  if (loaders.length === 0) return <p className="muted full">Fabric 暂不支持 {gameVersion}</p>;
  return (
    <div className="row full">
      <select value={loaderVersion} onChange={(e) => setLoaderVersion(e.target.value)}>
        {loaders.map((l) => (
          <option key={l.version} value={l.version}>
            {l.version}
            {l.stable ? "" : " (测试版)"}
          </option>
        ))}
      </select>
      <button className="primary" onClick={install} disabled={!!busy || !loaderVersion}>
        安装 Fabric Loader
      </button>
    </div>
  );
}
