import { openUrl } from "@tauri-apps/plugin-opener";
import { FormEvent, useState } from "react";
import { Account, api, DeviceCode, errorMessage } from "../api";
import { useApp } from "../state";

export default function AccountsPage() {
  const { accounts, setAccounts, notify } = useApp();
  const [offlineName, setOfflineName] = useState("");
  const [deviceCode, setDeviceCode] = useState<DeviceCode | null>(null);
  const [starting, setStarting] = useState(false);

  const update = async (action: () => Promise<Account[]>) => {
    try {
      setAccounts(await action());
    } catch (e) {
      notify("error", errorMessage(e));
    }
  };

  const addOffline = async (e: FormEvent) => {
    e.preventDefault();
    await update(() => api.addOfflineAccount(offlineName));
    setOfflineName("");
  };

  const loginMicrosoft = async () => {
    setStarting(true);
    try {
      const code = await api.msLoginStart();
      setDeviceCode(code);
      openUrl(code.verificationUri).catch(() => {});
      const list = await api.msLoginFinish(code);
      setAccounts(list);
      notify("info", "微软账号登录成功");
    } catch (e) {
      notify("error", errorMessage(e));
    } finally {
      setStarting(false);
      setDeviceCode(null);
    }
  };

  return (
    <section className="page">
      <h1>账号</h1>

      {accounts.length === 0 ? (
        <p className="muted">还没有账号。</p>
      ) : (
        <ul className="list">
          {accounts.map((a) => (
            <li key={a.id} className={`list-item${a.selected ? " selected" : ""}`}>
              <img
                className="avatar"
                src={`https://mc-heads.net/avatar/${a.uuid}/40`}
                alt=""
                onError={(e) => (e.currentTarget.style.visibility = "hidden")}
              />
              <div className="grow">
                <div>{a.name}</div>
                <div className="muted small">{a.kind === "microsoft" ? "微软账号" : "离线账号"}</div>
              </div>
              {a.selected ? (
                <span className="badge">使用中</span>
              ) : (
                <button onClick={() => update(() => api.selectAccount(a.id))}>使用</button>
              )}
              <button className="danger" onClick={() => update(() => api.removeAccount(a.id))}>
                删除
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="card">
        <h2>微软账号</h2>
        {deviceCode ? (
          <div className="device-code">
            <p>在打开的网页中输入下面的代码完成登录：</p>
            <div className="code">{deviceCode.userCode}</div>
            <div className="row">
              <button onClick={() => navigator.clipboard.writeText(deviceCode.userCode)}>复制代码</button>
              <button onClick={() => openUrl(deviceCode.verificationUri)}>重新打开登录页</button>
              <button className="link" onClick={() => api.msLoginCancel()}>
                取消
              </button>
            </div>
            <p className="muted small">
              浏览器会自动使用已登录的微软账号。如果那不是购买了 Minecraft 的账号，请把登录地址{" "}
              <code>{deviceCode.verificationUri}</code> 复制到无痕/InPrivate 窗口中打开。
            </p>
            <p className="muted small">正在等待你在浏览器中完成登录...</p>
          </div>
        ) : (
          <button className="primary" onClick={loginMicrosoft} disabled={starting}>
            使用微软账号登录
          </button>
        )}
      </div>

      <form className="card" onSubmit={addOffline}>
        <h2>离线账号</h2>
        <p className="muted small">只能进入单人游戏或关闭了正版验证的服务器。</p>
        <div className="row">
          <input
            className="grow"
            placeholder="用户名（3-16 位字母、数字、下划线）"
            value={offlineName}
            onChange={(e) => setOfflineName(e.target.value)}
          />
          <button type="submit" disabled={!offlineName.trim()}>
            添加
          </button>
        </div>
      </form>
    </section>
  );
}
