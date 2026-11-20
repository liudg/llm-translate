# LLM Translate

本地大模型中英互译（Windows）。设计文档见 [docs/DESIGN.md](docs/DESIGN.md)。

## 开发环境

1. 安装 Rust stable、Node 22+、pnpm。
2. 安装前端依赖：

   ```powershell
   pnpm install
   ```

3. 拉取固定版本的 llama.cpp（Vulkan 版，约 89 MB，放到 `src-tauri/resources/llama/`，不入 git）：

   ```powershell
   ./scripts/fetch-llama.ps1
   ```

   已有 release zip 时可用 `-Zip <路径>` 跳过下载。

4. 运行：

   ```powershell
   pnpm tauri dev
   ```

首次启动会打开部署向导；设置在 `%APPDATA%\LLMTranslate\settings.json`，日志与模型在 `%LOCALAPPDATA%\LLMTranslate\`。删除 `settings.json` 可重新走向导。

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
