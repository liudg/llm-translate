# LLM Translate

本地大模型中英互译（Windows）。设计文档见 [docs/DESIGN.md](docs/DESIGN.md)。

## 开发环境

1. 安装 Rust stable、Node 22+、pnpm。
2. 安装前端依赖：

   ```powershell
   pnpm install
   ```

3. 拉取固定版本的 llama.cpp（Vulkan 版，约 89 MB，放到 `src-tauri/resources/llama/`，不入 git）和 PP-OCRv6 Small 模型（约 31 MB，放到 `src-tauri/resources/ocr/`）：

   ```powershell
   ./scripts/fetch-llama.ps1
   ./scripts/fetch-ocr.ps1
   ```

   已有 llama.cpp release zip 时可用 `-Zip <路径>` 跳过下载。

4. 开发运行：

   ```powershell
   pnpm tauri dev
   ```

5. 打包 NSIS 安装包（需先至少构建过一次以生成 `src-tauri/target/ort-cache`，再整理应用本地运行时 DLL）：

   ```powershell
   ./scripts/prepare-runtime.ps1
   pnpm tauri build
   ```

   产物位于 `src-tauri/target/release/bundle/nsis/LLM Translate_0.1.0_x64-setup.exe`。

首次启动会打开部署向导；设置在 `%APPDATA%\LLMTranslate\settings.json`，日志与模型在 `%LOCALAPPDATA%\LLMTranslate\`。删除 `settings.json` 可重新走向导。

## 发布与版本更新

项目已接入 GitHub Actions 自动构建发布（[.github/workflows/release.yml](.github/workflows/release.yml)）与 Tauri v2 应用内更新（`tauri-plugin-updater`）：

- 首次发版前需在 GitHub 仓库的 `Settings → Secrets and variables → Actions` 配置私钥 Secret：`TAURI_SIGNING_PRIVATE_KEY`（如私钥设置了密码，同时配置 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`）。
- 同步版本号并触发正式发布：

  ```powershell
  ./scripts/bump-version.ps1 -Version 0.2.0
  git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json src-tauri/Cargo.lock
  git commit -m "Release v0.2.0"
  git tag -a v0.2.0 -m "更新说明..."
  git push origin HEAD --follow-tags
  ```

- 流水线会自动打包 NSIS 安装包、生成 `.sig` 签名与 `latest.json` 并直接发布到 GitHub Release。客户端在打开主面板时静默检查更新（30 分钟冷却），也可在「设置 → 关于」手动检查并一键下载覆盖安装。

## 测试

```powershell
cd src-tauri; cargo test --lib
```

端到端检查（真实下载源与 llama-server）：

```powershell
cd src-tauri; cargo run --example m1_check -- detect
```

```powershell
cd src-tauri; cargo run --example m1_check -- download <临时目录>
```

```powershell
cd src-tauri; cargo run --example m1_check -- infer <含 Index-Translate-2B 的模型目录>
```
