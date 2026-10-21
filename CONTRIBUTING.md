# 参与贡献

感谢你愿意帮忙改进 MC Launcher！

## 开发环境

按 [README](README.md#开发环境) 安装 Node.js、Rust 和 Tauri 的系统依赖，然后：

```bash
npm install
cp .env.example .env   # 填写你自己的 MS_CLIENT_ID / CURSEFORGE_API_KEY，也可以留空
npm run tauri dev
```

不配置 Client ID 也能开发，用离线账号测试即可。**请不要使用其他启动器的 Client ID**，原因见 README。

## 提交前检查

CI 会运行下面这些检查，提交前请在本地跑一遍：

```bash
npm run build                                 # TypeScript 类型检查 + 前端构建
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## 约定

- 后端逻辑放在 `src-tauri/src/` 下对应的模块里，`lib.rs` 只负责 Tauri 命令和状态；纯逻辑（解析、规则、参数拼装等）请尽量配上单元测试。
- 前端通过 `src/api.ts` 调用后端，新增命令时在这里补上类型。
- 界面文字目前使用简体中文。
- 不要提交任何 API Key、令牌或个人数据；`.env` 已被忽略。
- 一个 PR 只做一件事，标题写清楚改了什么。

## 发布新版本（维护者）

1. 同步修改 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json` 里的版本号并合并到 `main`
2. 打标签并推送：`git tag v0.2.0 && git push origin v0.2.0`
3. `release.yml` 会构建各平台安装包并创建草稿 Release，检查后在网页上发布
