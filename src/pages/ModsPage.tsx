import { openUrl } from "@tauri-apps/plugin-opener";
import { FormEvent, useCallback, useEffect, useState } from "react";
import { api, errorMessage, ModSummary, versionLabel } from "../api";
import { useApp } from "../state";

export default function ModsPage() {
  const { installed, selectedVersion, setSelectedVersion, busy, runTask, notify } = useApp();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<ModSummary[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [mods, setMods] = useState<string[]>([]);

  const target = installed.find((v) => v.id === selectedVersion);

  const refreshMods = useCallback(() => {
    if (!selectedVersion) return setMods([]);
    api.listMods(selectedVersion).then(setMods).catch((e) => notify("error", errorMessage(e)));
  }, [selectedVersion, notify]);

  useEffect(refreshMods, [refreshMods]);

  const search = async (e?: FormEvent) => {
    e?.preventDefault();
    setSearching(true);
    try {
      setResults(await api.cfSearch(query, target?.gameVersion ?? null, target?.loader ?? null));
    } catch (err) {
      notify("error", errorMessage(err));
    } finally {
      setSearching(false);
    }
  };

  const install = (mod: ModSummary) =>
    runTask(`安装模组 ${mod.name}`, async () => {
      const files = await api.cfInstall(mod.id, selectedVersion);
      refreshMods();
      notify("info", `已安装：${files.join("、")}`);
    });

  const remove = async (file: string) => {
    try {
      await api.removeMod(selectedVersion, file);
      refreshMods();
    } catch (e) {
      notify("error", errorMessage(e));
    }
  };

  return (
    <section className="page">
      <h1>模组</h1>

      <label className="field">
        <span>安装到</span>
        <select value={selectedVersion} onChange={(e) => setSelectedVersion(e.target.value)}>
          {installed.length === 0 && <option value="">还没有安装任何版本</option>}
          {installed.map((v) => (
            <option key={v.id} value={v.id}>
              {versionLabel(v)}
            </option>
          ))}
        </select>
      </label>
      {target && !target.loader && (
        <p className="warning-text">这是原版游戏，装模组前请先在“版本”页为它安装 Fabric。</p>
      )}

      <form className="row" onSubmit={search}>
        <input
          className="grow"
          placeholder="在 CurseForge 搜索模组，如 Sodium"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <button className="primary" type="submit" disabled={searching}>
          {searching ? "搜索中..." : "搜索"}
        </button>
      </form>

      {results && results.length === 0 && <p className="muted">没有找到匹配的模组。</p>}
      <ul className="list">
        {results?.map((m) => (
          <li key={m.id} className="list-item">
            {m.iconUrl ? <img className="mod-icon" src={m.iconUrl} alt="" /> : <div className="mod-icon" />}
            <div className="grow">
              <div>
                {m.name} <span className="muted small">by {m.authors.join(", ")}</span>
              </div>
              <div className="muted small">{m.summary}</div>
              <div className="muted small">{m.downloadCount.toLocaleString()} 次下载</div>
            </div>
            {m.websiteUrl && (
              <button
                className="link"
                onClick={() => openUrl(m.websiteUrl!).catch((e) => notify("error", errorMessage(e)))}
              >
                详情
              </button>
            )}
            <button className="primary" onClick={() => install(m)} disabled={!!busy || !target?.loader}>
              安装
            </button>
          </li>
        ))}
      </ul>

      <h2>已安装的模组{selectedVersion && <span className="muted small"> · {selectedVersion}</span>}</h2>
      {mods.length === 0 ? (
        <p className="muted">这个版本还没有模组。</p>
      ) : (
        <ul className="list">
          {mods.map((file) => (
            <li key={file} className="list-item">
              <div className="grow">{file}</div>
              <button className="danger" onClick={() => remove(file)}>
                删除
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
