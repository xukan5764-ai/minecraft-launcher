import { FormEvent, useEffect, useState } from "react";
import { api, Config, errorMessage, SettingsView } from "../api";
import { useApp } from "../state";

export default function SettingsPage() {
  const { refreshInstalled, notify } = useApp();
  const [view, setView] = useState<SettingsView | null>(null);
  const [config, setConfig] = useState<Config | null>(null);

  useEffect(() => {
    api
      .getSettings()
      .then((v) => {
        setView(v);
        setConfig(v.config);
      })
      .catch((e) => notify("error", errorMessage(e)));
  }, [notify]);

  if (!view || !config) return <section className="page muted">加载中...</section>;

  const set = <K extends keyof Config>(key: K, value: Config[K]) =>
    setConfig({ ...config, [key]: value });

  const save = async (e: FormEvent) => {
    e.preventDefault();
    try {
      await api.saveSettings(config);
      await refreshInstalled();
      notify("info", "设置已保存");
    } catch (err) {
      notify("error", errorMessage(err));
    }
  };

  return (
    <form className="page" onSubmit={save}>
      <h1>设置</h1>

      <div className="card">
        <h2>游戏</h2>
        <label className="field">
          <span>游戏目录</span>
          <input
            value={config.gameDir}
            placeholder={view.defaultGameDir}
            onChange={(e) => set("gameDir", e.target.value)}
          />
        </label>
        <label className="field">
          <span>Java 路径</span>
          <input
            value={config.javaPath}
            placeholder="留空则自动下载 Mojang 官方 Java"
            onChange={(e) => set("javaPath", e.target.value)}
          />
        </label>
        <label className="field">
          <span>最大内存 (MB)</span>
          <input
            type="number"
            min={512}
            step={256}
            value={config.maxMemoryMb}
            onChange={(e) => set("maxMemoryMb", Number(e.target.value))}
          />
        </label>
        <label className="field">
          <span>额外 JVM 参数</span>
          <input
            value={config.jvmArgs}
            placeholder="-XX:+UseG1GC"
            onChange={(e) => set("jvmArgs", e.target.value)}
          />
        </label>
        <label className="field">
          <span>下载线程数</span>
          <input
            type="number"
            min={1}
            max={64}
            value={config.downloadConcurrency}
            onChange={(e) => set("downloadConcurrency", Number(e.target.value))}
          />
        </label>
      </div>

      <div className="card">
        <h2>API 配置</h2>
        <p className="muted small">
          推荐在项目根目录的 <code>.env</code> 中配置，编译时会写进程序；这里填写的值会覆盖内置值。
        </p>
        <label className="field">
          <span>微软 Client ID</span>
          <input
            value={config.msClientId}
            placeholder={view.hasBuiltinMsClientId ? "已内置，留空即可" : "未配置"}
            onChange={(e) => set("msClientId", e.target.value)}
          />
        </label>
        <label className="field">
          <span>CurseForge API Key</span>
          <input
            type="password"
            value={config.curseforgeApiKey}
            placeholder={view.hasBuiltinCurseforgeKey ? "已内置，留空即可" : "未配置"}
            onChange={(e) => set("curseforgeApiKey", e.target.value)}
          />
        </label>
      </div>

      <button className="primary" type="submit">
        保存
      </button>
    </form>
  );
}
