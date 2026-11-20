# LLM Translate 详细设计 v1

> 状态：方案已与需求方确认（2026-11-20）；M0 技术验证已完成，结果见第 10 节；M1、M2 已完成。

## 1. 需求范围

| 项 | 结论 |
|---|---|
| 平台 | Windows 10 1809 (build 17763)+ x64、Windows 11；不支持 ARM |
| 语言 | 仅中英互译（UI 中文） |
| 功能 | 输入框翻译、划词翻译、截图翻译；托盘常驻、开机自启、自定义快捷键 |
| 翻译 | 仅本地模型，无任何在线回退 |
| 模型分发 | 不进安装包；首次部署按硬件默认选一个模型下载；多档模型可选 |
| 部署 | 首次启动"一键部署"：检测硬件 → 选后端/模型 → 下载校验 → 启动自检 |
| 划词触发 | 选中后弹小图标，点击图标才取词翻译 |
| 划词兼容 | **任何情况下不发送 Ctrl+C**；UIA → Ctrl+Insert → 失败提示 |
| 截图展示 | 冻结画面，译文按原位置覆盖原文，可切换原图、复制全文 |
| 用途 | 个人自用 |

## 2. 技术栈

| 层 | 选型 |
|---|---|
| 应用框架 | Tauri 2（Rust stable） |
| 前端 | React 18 + TypeScript + Vite + Tailwind CSS + zustand；lucide 图标 |
| 视觉 | Fluent 风格；Win11 Mica / Win10 Acrylic（拖动卡顿则降级为纯色）；深浅色跟随系统 |
| Win32 / UIA / DXGI | `windows` crate (windows-rs) |
| 推理 | llama.cpp 官方 release 的 `llama-server`，作为子进程（sidecar） |
| OCR | PP-OCRv6 Small（det + rec）ONNX，经 `oar-ocr` 0.11（内部 `ort`，ONNX Runtime 静态链接）；模型约 30 MB 随安装包 |
| 网络 | `reqwest`（流式、Range 断点续传）+ `tokio` |
| 插件 | tauri-plugin-global-shortcut、tauri-plugin-autostart、tauri-plugin-single-instance |
| 安装包 | Tauri NSIS bundler，当前用户安装（免管理员），内嵌 WebView2 bootstrapper |

## 3. 模型

### 3.1 模型档位

选型原则：**翻译质量优先**（需求方确认许可证不作为选型因素）。候选模型库已由需求方确定为以下三个（2026-11-20），调研依据见 `spikes/research/candidates.md`。

| id | 模型 | 出品 | 量化 | 体积 | 定位 |
|---|---|---|---|---|---|
| `index-translate-2b-q4` | Index-Translate-2B | B 站 IndexTeam（基于 Qwen3.5） | Q4_K_M | 1.31 GB | 轻量；纯 CPU / 核显 / 小显存 |
| `hy-mt2-7b-q4` | Hy-MT2-7B | 腾讯混元 | Q4_K_M | 4.62 GB | 中档；显存约 6 GB |
| `index-translate-9b-q4` | Index-Translate-9B | B 站 IndexTeam（基于 Qwen3.5） | Q4_K_M | 5.78 GB | 高质量；显存约 8 GB |

- 三个模型都提供官方 GGUF。Index-Translate 的 GGUF 同时带图片翻译用的 `mmproj` 文件，本应用**不下载**（截图翻译需要文字位置做原位覆盖，仍走 OCR）。
- Hy-MT2-7B：M0 已在官方 llama.cpp b12541 上实测可用（GPU 约 42 tok/s）。
- Index-Translate-2B / 9B：M1 已在 llama.cpp b12541 上实测可用，思考模式不输出（见 10.6）。
- 不再纳入：HY-MT1.5 全系、Hy-MT2-1.8B、Hy-MT2-30B-A3B，以及 1.25bit / 2bit 极限量化版本。

### 3.2 模型清单 `models.json`（随应用发布，可扩展）

实际清单见 `src-tauri/resources/models.json`（M1 已填入精确大小与 SHA256）。每个条目字段：

```json
{
  "id": "index-translate-2b-q4",
  "name": "轻量 · Index-Translate 2B",
  "file": "Index-Translate-2B.Q4_K_M.gguf",
  "size": 1312164352,
  "sha256": "044b313d29342bd3b2c77cbb64023ca0d209bd9b3247763f9d162767ef2d746a",
  "sources": [
    { "name": "modelscope",  "url": "https://www.modelscope.cn/models/IndexTeam/Index-Translate-2B-GGUF/resolve/master/<file>" },
    { "name": "huggingface", "url": "https://huggingface.co/IndexTeam/Index-Translate-2B-GGUF/resolve/main/<file>" },
    { "name": "hf-mirror",   "url": "https://hf-mirror.com/IndexTeam/Index-Translate-2B-GGUF/resolve/main/<file>" }
  ],
  "minRamGB": 4, "recommendVramGB": 2, "ctx": 4096, "promptProfile": "index-translate"
}
```

- M1 核实：三个模型在 ModelScope 都有官方 GGUF 仓库（`IndexTeam/*-GGUF`、`Tencent-Hunyuan/Hy-MT2-7B-GGUF`），`resolve/master/<file>` 302 跳转到 CDN，支持 Range；ModelScope 的 HEAD 请求不返回长度，测速/续传一律用 Range GET。

### 3.3 提示词与采样（按模型的 promptProfile）

不同模型的官方提示词与解码参数不同，由 `promptProfile` 决定，翻译调度层据此拼装请求。两者都不用 system prompt，目标语言取"中文"/"英语"。

| promptProfile | 模板 | 解码 | 额外请求参数 |
|---|---|---|---|
| `index-translate` | `请将以下文本翻译为{目标语言}，直接输出翻译结果，不要进行任何解释。\n\n{原文}` | greedy（temperature 0） | `chat_template_kwargs: {"enable_thinking": false}`（关闭 Qwen3.5 思考模式） |
| `hy-mt` | `将以下文本翻译为{目标语言}，注意只需要输出翻译后的结果，不要额外解释：\n\n{原文}` | top_k 20, top_p 0.6, repetition_penalty 1.05, temperature 0.7 | — |

- Index-Translate 官方还支持术语表、领域（genre）等约束模板，可用于以后的"术语表"功能（当前不在需求范围内）。

## 4. 架构

```
┌──────────────────────── Tauri 主进程 (Rust) ────────────────────────┐
│ app       设置/日志/托盘/自启/快捷键/单实例                              │
│ deploy    硬件检测 · 模型清单 · 镜像测速 · 断点续传 · SHA256 · 向导状态机  │
│ inference llama-server 守护（启动/健康检查/重启/降级）· 流式客户端 · 取消  │
│ translate 语言判定 · 分段 · 提示词 · LRU 缓存 · 请求调度（新请求顶替旧请求）│
│ selection 鼠标钩子线程 · UIA 线程 · 安全复制 · 黑名单                      │
│ capture   DXGI 截屏 · OCR · 段落合并 · 背景/文字色估计                     │
└───────────────┬──────────────────────────────┬──────────────────────┘
                │ HTTP 127.0.0.1:随机端口(SSE)  │ Tauri IPC / events
        ┌───────▼────────┐            ┌────────▼─────────────────────────┐
        │ llama-server   │            │ WebView 窗口（预创建、隐藏待命）     │
        │ (cpu / vulkan) │            │ main · mini · sel-icon · popup     │
        └────────────────┘            │ shot-overlay · shot-result · wizard│
                                      └──────────────────────────────────┘
```

### 4.1 目录结构

```
llm-translate/
├─ src-tauri/
│  ├─ src/
│  │  ├─ main.rs
│  │  ├─ app/        settings.rs tray.rs autostart.rs hotkey.rs logging.rs
│  │  ├─ deploy/     hardware.rs manifest.rs downloader.rs wizard.rs
│  │  ├─ inference/  supervisor.rs client.rs
│  │  ├─ translate/  detect.rs segment.rs prompt.rs scheduler.rs cache.rs
│  │  ├─ selection/  hook.rs uia.rs clipboard.rs input.rs blacklist.rs
│  │  └─ capture/    dxgi.rs overlay.rs ocr.rs layout.rs
│  ├─ resources/     llama/cpu/ llama/vulkan/ ocr/ models.json
│  └─ tauri.conf.json
├─ src/              React 前端
│  ├─ windows/       main mini sel-icon popup shot-overlay shot-result wizard
│  ├─ components/  stores/  styles/  lib/
└─ docs/DESIGN.md
```

### 4.2 数据目录

- 设置：`%APPDATA%\LLMTranslate\settings.json`
- 日志：`%LOCALAPPDATA%\LLMTranslate\logs\`（按天滚动，保留 7 天）
- 模型：默认 `%LOCALAPPDATA%\LLMTranslate\models\`，向导中可改到其他盘

## 5. 模块设计

### 5.1 一键部署向导

状态机：`Welcome → Detect → (Customize) → Download → Verify → Launch → SelfTest → Done`，任一步失败给出原因 + 重试/降级按钮。

1. **Detect**
   - CPU：型号、物理核数、AVX2 支持（决定 CPU 后端可用性/线程数）。
   - 内存：总量/可用；< 8 GB 提示仅建议 2B。
   - 显卡：DXGI 枚举适配器（厂商 ID、专用显存）；Vulkan 可用性由 llama-server `--list-devices` 实测确认。
   - 磁盘：目标目录剩余空间 ≥ 模型体积 × 1.2。
2. **默认选择规则**（按 M1 实测显存校准，见 10.6；需求方授权按使用体验决定，2026-11-20）
   - 独显可用显存 ≥ 5.5 GB → Index-Translate-9B，全部 offload（`-ngl 99`）。实测 9B 含 4K 上下文占 5.1 GB，6 GB 显卡即可。
   - 独显可用显存 ≥ 2 GB → Index-Translate-2B，全部 offload（实测占 1.5 GB）。
   - 否则 → Index-Translate-2B，CPU，线程数 = 物理核数。9B 在 CPU 上仅约 5 tok/s，不作为默认。
   - Hy-MT2-7B 不作为默认：显存占用与 9B 相同而 9B 质量更好；保留在"自定义"中可选。
   - 核显（DXGI 专用显存 < 1 GB）共享系统内存且 Vulkan 驱动质量参差，不计入"可用显存"，默认走 CPU；可在"自定义"中手动选择核显。多显卡时选可用显存最大的独显并用 `--device VulkanN` 指定，避免 llama.cpp 把层拆到核显上。
   - 内存 < 8 GB 时只推荐 2B。
   - 用户可在 Customize 中改选（不强制展示，"一键"路径只需点"开始部署"）。
3. **Download**
   - 各源用 Range 请求前 2 MB 测速，选最快源；失败自动切下一源。
   - 写入 `.part`，支持暂停/续传；完成后 SHA256 校验，失败删除重下。
   - 只用直连，不做任何代理处理（需求方确认，2026-11-20）。
   - 每源最多重试 3 次（有进展的断开不计次），全部源失败后再测速重来，共 3 轮；SHA256 不一致自动重下 1 次。
4. **Launch / SelfTest**
   - 启动 llama-server，健康检查 `/health`；翻译"你好，世界"并校验返回非空。
   - GPU 后端启动失败或自检失败 → 自动以 CPU 后端重试，并记入设置。
5. **Done**：展示快捷键与使用方式，进入主界面；托盘常驻。

### 5.2 推理守护（inference）

- 启动参数：`-m <gguf> --host 127.0.0.1 --port <随机空闲端口> -c 4096 --parallel 1 -ngl <N> -t <threads>`。
- 后端选择：只发布 llama.cpp **Vulkan 版**一套二进制（已含全部 CPU 变体）。有 Vulkan 设备时 `-ngl 99`；无 Vulkan 驱动时 `ggml-vulkan.dll` 加载失败会自动只剩 CPU 后端（M0 已验证）；强制 CPU 用 `--device none -ngl 0`。CUDA 不随安装包发布（体积大），后续可作为设置中的可选加速包。
- 硬件检测：`llama-server --list-devices` 直接给出 Vulkan 设备名与可用显存，可作为选档依据。
- 守护：子进程退出即重启（指数退避，最多 3 次后降级 CPU 并通知）；主进程退出时以 Job Object 保证子进程一并结束。
- 内存策略：默认常驻；设置项"闲置 N 分钟后释放模型"（关闭进程，下次请求时冷启动并在 UI 显示"加载模型中"）。
- 客户端：`/v1/chat/completions` stream=true，SSE 逐 token 推送前端；取消 = 断开连接。

### 5.3 翻译调度（translate）

- **方向判定**：自动模式下**逐段**判定（中英混排的文本各段各自翻译）：统计 CJK 统一表意字符与拉丁字母数量，CJK ≥ 拉丁 → 中译英，否则英译中；不含任何字母的段落（数字、符号、分隔线）原样保留、不送模型。手动锁定方向时所有段落按锁定方向翻译。开始事件带 `mixed` 标志（自动模式下各段方向不一致），界面据此显示"中英混合，逐段翻译"而不是多数方向。
- **分段**：按段落切分，单段超过约 1500 token 再按句切；逐段流式输出，保留原段落结构。
- **调度**：单队列；新的划词/截图请求取消正在进行的旧请求；输入框输入防抖 500 ms。
- **缓存**：(原文, 方向, 模型) → 译文，LRU 500 条，仅内存。
- **清洗**：去掉模型偶发的首尾引号/"译文："前缀。

### 5.4 输入框翻译

- 主窗口（M2 实现）：左侧紧凑导航栏（翻译 / 设置，两个页面都常驻挂载，切到设置再回来原文不丢）。翻译页：
  - 顶部方向分段控件"自动 / 中→英 / 英→中"，切换方向立即重译；右侧模型状态与"迷你窗"按钮。
  - 原文 / 译文双栏，窗口宽 ≥ 860 px 左右排列，更窄时上下排列。
  - 停止输入 500 ms 后自动翻译（原文与方向都未变时不重复请求），Ctrl+Enter 立即翻译；原文上限 2 万字。
  - 译文流式显示（末尾闪烁光标），底栏显示方向与耗时，支持停止、重新翻译、复制；首次请求遇模型加载时显示"正在加载模型…"。
  - 被其他窗口的新请求顶替时显示"已被其他翻译请求中断"，可点"重新翻译"。
- 迷你窗（Alt+Q）：无边框、置顶、不进任务栏，出现在**光标所在显示器**的水平居中、工作区高度 1/4 处，译文向下展开不挪动输入框。
  - 单输入框（最多自动增高到约 5 行）：Enter 翻译，Shift+Enter 换行，Esc 关闭，失焦自动隐藏；左侧图标可拖动窗口。
  - 方向按钮点击循环"自动 → 中→英 → 英→中"。窗口高度随内容变化（译文区最高 320 px，超出滚动）。
  - 再次呼出时保留上次原文并全选，可直接输入覆盖；结果区"复制译文"按钮一键复制。
  - Alt+Q 在迷你窗已聚焦时再按一次即隐藏。

### 5.5 划词翻译（selection）

**线程模型**
- 钩子线程：`WH_MOUSE_LL`，回调只记录事件并投递到通道，不做任何阻塞操作（避免系统钩子超时被摘除）。
- 工作线程：判定手势、驱动图标窗口、执行取词。
- UIA 线程：MTA 初始化 COM，所有 UIA 调用带 200 ms 超时。

**触发判定**（左键 down→up 期间）
- 拖动距离 ≥ 8 px（按 DPI 缩放），或双击（系统双击时间内），或 Shift+单击。
- 排除：前台为本应用窗口、黑名单进程、全屏独占应用。
- 满足条件 → 在光标右下偏移处显示图标窗（`WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TOOLWINDOW`，不抢焦点、不进任务栏）。
- 3 秒未点击、鼠标远离、键盘按下、滚轮 → 隐藏图标。

**取词（仅在点击图标时执行）**
1. **UIA**：`GetFocusedElement` → 向上查找支持 `TextPattern` 的元素 → `GetSelection` → 拼接文本；按行去掉行尾填充空格（终端 UIA 会返回整行宽度的空格）。非空即成功。
2. **Ctrl+Insert**（不产生 0x03 中断信号）：
   - 保存剪贴板：枚举格式，复制可复制的 HGLOBAL 数据（单格式 > 10 MB 跳过）。
   - 记录 `GetClipboardSequenceNumber`。
   - `SendInput`：先释放用户当前按住的修饰键，再发 Ctrl↓ Insert↓(扩展键标志) Insert↑ Ctrl↑。
   - 轮询序号最长 300 ms；变化则读 `CF_UNICODETEXT`。
   - 恢复原剪贴板，并给恢复操作附加 `ExcludeClipboardContentFromMonitorProcessing` 格式，避免原内容在 Win+V 历史里重复出现。已知限制：被复制的选中文本由目标应用写入，无法标记排除，会出现在 Win+V 历史中（仅 UIA 失败走此路径时）。
   - 剪贴板序号变化但内容为空（如 conhost 快速编辑模式单击后留下零宽选区）视为取词失败。
3. **失败**：浮窗提示"该应用不支持划词，可使用截图翻译（Alt+W）"。
4. **绝不发送 Ctrl+C**；因此无需识别终端类型。

**译文浮窗**：出现在选区附近，流式显示译文；失焦自动关闭，可"钉住"；支持复制、重新翻译。

### 5.6 截图翻译（capture）

1. **截屏**：进程设为 Per-Monitor DPI Aware V2；DXGI Desktop Duplication 抓取所有显示器，冻结为位图。
2. **选区**：每个显示器一个全屏遮罩窗显示冻结图，拖拽框选；Esc 取消，右键取消选区。
3. **OCR**：det → rec（中英），得到文本行 + 四边形框；按行距/对齐合并为段落。
4. **翻译**：逐段调用翻译调度，结果到达即渲染（渐进显示）。
5. **渲染**（前端 Canvas）：
   - 背景色：取段落框外扩 2 px 边缘像素的中位数，填充擦除原文。
   - 文字色：取框内与背景对比最大的主色。
   - 字号：从原行高估算，译文放不下时逐步缩小（下限 9 px）并允许换行。
6. **结果窗**：无边框置顶窗口贴在原选区位置；工具条：切换原图/译图、复制全部译文、复制图片、关闭；可拖动。

### 5.7 托盘、自启、快捷键

- 托盘菜单：输入翻译 / 截图翻译 / 划词开关 / 设置 / 退出；单击托盘打开主窗口。
- 开机自启：tauri-plugin-autostart（HKCU Run），启动参数 `--minimized` 仅驻托盘。
- 快捷键默认：Alt+Q 输入翻译、Alt+W 截图翻译、划词开关（默认不绑定）。设置页录制快捷键，注册失败即提示"已被其他程序占用"。
- M2 已用 tauri-plugin-global-shortcut 固定注册 Alt+Q（不可改）；注册失败时主窗口顶部横幅提示、设置页快捷键行标"未生效"，迷你窗仍可从翻译页按钮打开。录制与冲突检测在 M5。
- 单实例：重复启动时激活已有实例。

### 5.8 设置页

常规（自启、主题）· 快捷键 · 划词（开关、黑名单、触发方式细项）· 模型（已下载列表、下载/切换/删除、后端选择、闲置释放）· 关于（版本、日志目录）。

- 设置页是主窗口内的一个页面（左侧导航切换），Win11 设置风格的分组卡片。
- M2 可用：
  - 主题：跟随系统 / 浅色 / 深色，所有窗口同步；同时设置原生窗口主题（标题栏、Mica 色调）与页面 `data-theme`。
  - 模型：当前模型、状态、目录；推理后端（CPU / 检测到的各 Vulkan 设备）与 CPU 线程数，"应用并重新加载模型"后在后台重启推理进程；闲置释放（从不 / 5 / 10 / 30 / 60 分钟，立即生效）；"更换模型 / 重新部署"打开向导。显卡列表用 `llama-server --list-devices` 检测，不停止正在运行的模型，首次打开设置页时才检测。
  - 关于：版本、系统 build 与窗口效果、设置文件位置、打开日志目录。
- M2 占位（标注"将在后续版本提供"）：开机自启、截图快捷键、划词开关快捷键、自定义快捷键（M5）；划词翻译各项（M3）；模型下载 / 切换 / 删除管理（M5）。
- 每次设置变更都广播 `settings-changed` 事件，各窗口据此同步（含 GPU 失败自动退回 CPU 的变更）。

## 6. 安装包

- NSIS，当前用户安装到 `%LOCALAPPDATA%\Programs\LLMTranslate`，免管理员。
- 内容：主程序（含静态链接的 ONNX Runtime）、llama.cpp Vulkan 版所需文件（约 89 MB 未压缩）、OCR 模型（约 30 MB）、`models.json`；压缩后预计 < 100 MB。
- WebView2：内嵌 bootstrapper（Win10 可能未预装）。
- VC++ 运行库：llama.cpp 依赖 `MSVCP140.dll`、`VCRUNTIME140.dll`、`VCRUNTIME140_1.dll`（UCRT `api-ms-win-crt-*` 为 Win10 系统自带）。采用**应用本地部署**：把这 3 个 DLL 放在 llama-server 同目录，无需安装 VC++ Redist。M6 在干净 Win10 上验证。
- 卸载：询问是否同时删除已下载模型。

## 7. 性能目标

| 指标 | 目标 |
|---|---|
| 启动到托盘就绪（不含模型加载） | < 1 s |
| 2B 模型加载（SSD） | < 3 s |
| 划词点击 → 首字（2B，短句，CPU） | < 500 ms；M1 实测 2B CPU 首字约 150 ms，短句总耗时 600–700 ms |
| 截图框选完成 → OCR 完成（常规区域） | < 300 ms |
| 应用自身内存（不含模型） | < 120 MB |

## 8. 风险与待验证项

| 风险 | 应对 |
|---|---|
| 老旧 Intel 核显 Vulkan 驱动异常 | 自检失败自动降级 CPU |
| Index-Translate（Qwen3.5 架构）在 llama.cpp b12541 上无法加载或思考模式关不掉 | M1 已验证可用（10.6），风险关闭 |
| 小众术语误译（M0 测过的模型都把"划词翻译"译错） | 可选：Index-Translate 官方支持术语表约束，可做内置/用户自定义术语表（未纳入当前范围，需需求方确认） |
| HuggingFace 下载频繁断连（M0 实测 5 次断开） | 下载器自动重试 + Range 续传 + 换源（M1 已实现并测试） |
| Chromium 首次 UIA 查询需激活无障碍树，可能取不到 | Ctrl+Insert 兜底 |
| 部分应用不支持 Ctrl+Insert | 提示改用截图翻译；黑名单 |
| 鼠标钩子/模拟按键被杀软误报 | 个人自用，必要时加白；日志记录 |
| ModelScope 是否有官方 GGUF 仓库 | M1 已核实：有，且国内直连最快 |
| Win10 Acrylic 拖动卡顿 | 检测系统版本降级为纯色背景 |

## 9. 里程碑

| 里程碑 | 内容 | 验收标准 |
|---|---|---|
| **M0 技术验证** | Tauri 骨架与窗口特效（Win10/11）；llama-server 跑 HY-MT1.5-1.8B（CPU/Vulkan）测延迟；UIA + Ctrl+Insert 取词原型；OCR ONNX 原型；Hy-MT2 与 VC++ 依赖核实 | 各原型跑通，测得数据回写本文档 |
| **M1 推理与部署** | 先验证 Index-Translate-2B / 9B 在 llama.cpp 上可用并实测速度与显存、填入 SHA256；然后做推理守护、翻译调度（含 promptProfile）、硬件检测、下载器、部署向导 | 新机器上一键完成部署并通过自检；断网/断点/GPU 失败均能正确处理 |
| **M2 输入框翻译** | 主窗口、迷你窗、设置页框架、主题 | 中英互译流式输出，界面在 Win10/11 正常 |
| **M3 划词翻译** | 钩子、图标窗、取词链路、译文浮窗、黑名单 | 见下方划词测试矩阵；**所有终端中运行 `ping -t` 时划词，进程均不被终止** |
| **M4 截图翻译** | 多屏截屏、选区、OCR、覆盖渲染、结果窗 | 多显示器 + 不同缩放比下位置准确；中英截图可读 |
| **M5 系统集成** | 托盘、自启、快捷键录制与冲突检测、模型管理页 | 功能完整，重启后状态保持 |
| **M6 打包与打磨** | NSIS 安装包、Win10 1809 / Win11 实机测试、性能调优 | 干净系统安装 → 部署 → 三大功能全部可用；达成第 7 节性能目标 |

- M2 验收（2026-11-20，Win11 build 26200）：中英互译流式输出、自动 / 锁定方向、中英混合逐段翻译、设置页切换 GPU↔CPU 并重载、闲置释放设置、浅色 / 深色 / 跟随系统、迷你窗随内容增高 / Esc 关闭 / 失焦隐藏 / 再次呼出全选上次原文，均已在真实应用中验证。Win10 无测试环境，界面与 Acrylic 效果的验证并入 M6。

**划词测试矩阵**：Windows Terminal（cmd / PowerShell / WSL）、传统 conhost、Git Bash（mintty）、VS Code 集成终端、Xshell、MobaXterm、PuTTY、Chrome、Edge、Word、WPS、微信、记事本、资源管理器。

## 10. M0 技术验证结果（2026-11-20）

测试机：Ryzen 9 7945HX（16C/32T）、RTX 4060 Laptop 8 GB、16 GB 内存、Windows 11 build 26200。原型代码与原始数据在 `spikes/`。

### 10.1 推理（llama.cpp b12541 Vulkan 版，`spikes/bench/`）

| 模型 | 后端 | 加载 | 短句总耗时 | 生成速度 |
|---|---|---|---|---|
| Hy-MT2-1.8B Q4_K_M | Vulkan GPU | 1.8–2.4 s | 110–130 ms | ~145 tok/s |
| Hy-MT2-1.8B Q4_K_M | CPU 8 线程 | 1.6 s | 460–540 ms | ~24 tok/s |
| Hy-MT2-7B Q4_K_M | Vulkan GPU | 12.3 s（首次） | 320–370 ms | ~42 tok/s |
| HY-MT1.5-1.8B Q4_K_M | CPU 4/8/16 线程 | 1.6–3.1 s | 450–600 ms | 21–25 tok/s（线程数影响很小，受内存带宽限制） |

- Vulkan 首次加载约 6 s（着色器编译），之后约 1.8 s。
- 删除 `ggml-vulkan.dll` 模拟无驱动：`--list-devices` 为空，`-ngl 99` 仍正常以 CPU 运行。

### 10.2 翻译质量（`spikes/bench/quality.md`，15 条真实场景句，GPU，官方采样参数）

| 模型 | 评价 |
|---|---|
| HY-MT1.5-1.8B | 有严重错误（语义反转），口语/网络用语/文案生硬或错译 |
| Hy-MT2-1.8B | 无明显错误，通顺准确 |
| Hy-MT2-7B | 最自然地道（学术、新闻、公文表现最好） |
| HY-MT1.5-7B | 与 Hy-MT2-7B 接近，技术概念（借用、future）更准 |

后续调研（`spikes/research/candidates.md`）发现 Index-Translate 系列：官方报告中 2B 在各项指标上超过 Hy-MT2-1.8B，9B 基础质量与 Hy-MT2-7B 持平、网络用语和指令遵循明显更强。需求方据此确定模型库为 **Index-Translate-2B、Hy-MT2-7B、Index-Translate-9B**（见 3.1），不再做额外的质量对比实测。

### 10.3 划词取词（`spikes/selection/`，自动化测试 `--term-test`）

| 终端 | UIA 读选区 | Ctrl+Insert 读选区 | 无选中时 Ctrl+Insert | 对照：真实 Ctrl+C |
|---|---|---|---|---|
| conhost (cmd) | ✅ | ✅ | ping 存活 ✅ | ping 被终止 |
| Windows Terminal (cmd) | ✅ | ✅ | ping 存活 ✅ | ping 被终止 |
| Windows Terminal (PowerShell) | ✅ | ✅ | ping 存活 ✅ | ping 被终止 |
| Git Bash (mintty) | ❌ 无 TextPattern | ✅ | ping 存活 ✅ | ping 被终止 |

结论：**Ctrl+Insert 在所有终端均不会终止进程**，对照组证明测试有效；UIA + Ctrl+Insert 两级可覆盖以上终端。非终端应用（浏览器、Office、WPS、微信等）与 Xshell / MobaXterm / PuTTY / VS Code 终端在 M3 测试矩阵中验证。

### 10.4 OCR（`spikes/ocr/`，oar-ocr 0.11 + PP-OCRv6）

| 档位 | 模型体积 | 加载 | 识别 1200×520 / 1372×956 | 准确度 |
|---|---|---|---|---|
| Tiny | 6 MB | 99 ms | 66–73 ms / 58 ms | 中英混排丢空格、个别字母错（dog→doq） |
| Small | 30 MB | 139 ms | 162–173 ms / 125 ms | 测试图全部正确 → 采用 |

- 窗口按钮等图标会被识别为低置信度的单个符号（□、X），渲染前按置信度 < 0.8 且长度 ≤ 1 过滤。
- 构建注意：`ort-sys` 下载 ONNX Runtime 需设置 `ORT_CACHE_DIR`（避免跨盘重命名失败）与正确的代理环境变量。

### 10.5 应用骨架

- Tauri 2.12 + React 19 + Tailwind 4 骨架可运行；Win11 Mica 效果正常。
- 遗留至 M6：Win10 Acrylic 效果、VC++ DLL 应用本地部署在干净 Win10 上的验证。

### 10.6 M1 模型实测（2026-11-20，llama.cpp b12541 Vulkan 版，`spikes/bench/bench_profile.py`）

三个模型 SHA256 均与官方一致；国内直连 ModelScope 下载 22–41 MB/s，hf-mirror 直连可用，huggingface.co 直连不通。

| 模型 | 后端 | 加载 | 短句首字 / 总耗时 | 生成速度 | 显存增量（含 4K 上下文） |
|---|---|---|---|---|---|
| Index-Translate-2B Q4_K_M | Vulkan GPU | 4.6 s | 90–240 ms / 190–360 ms | 87–117 tok/s | 1.5 GB |
| Index-Translate-2B Q4_K_M | CPU 8 线程 | 1.9 s | ~150 ms / 610–720 ms | ~19 tok/s | — |
| Index-Translate-9B Q4_K_M | Vulkan GPU | 6.6 s | 165–230 ms / 430–490 ms | 31–39 tok/s | 5.1 GB |
| Index-Translate-9B Q4_K_M | CPU 16 线程 | 4.7 s | ~580 ms / 2.6–2.8 s | ~5 tok/s | — |
| Hy-MT2-7B Q4_K_M | Vulkan GPU | 5.4 s | — | ~42 tok/s（M0） | 5.1 GB |

- Index-Translate 用 `chat_template_kwargs.enable_thinking=false` 时无 `reasoning_content`、无 `<think>`；不传该参数也不思考（模板默认插入空思考块），仍按 3.3 传参。
- 9B 显存占用与 Hy-MT2-7B 几乎相同（约 5.1 GB），6 GB 显卡也能全量 offload 9B；据此 5.1 节默认规则改为 ≥ 5.5 GB → 9B，7B 不再自动选择。
- llama-server 运行只需 `llama-server.exe`、`llama-server-impl.dll`、`llama-common.dll`、`llama.dll`、`mtmd.dll`、`ggml*.dll`、`libomp.dll`，共 89 MB；由 `scripts/fetch-llama.ps1` 拉取到 `src-tauri/resources/llama/`（不入 git）。
- 端到端检查 `src-tauri/examples/m1_check.rs`：下载（无可用源、暂停/续传/换源、已存在只校验、SHA256 不一致重下）与推理（GPU 启动失败→CPU 降级、新请求取消旧请求、杀进程自动重启、闲置释放后重新加载）全部通过。


### 10.7 M2 实现备注（2026-11-20）

- 窗口在 `tauri.conf.json` 中设为 `"create": false`，由 `setup` 在注册完全部状态后创建并带上 Mica / Acrylic 效果。否则开发模式下页面加载快于 `setup`，首批 `get_settings` / `app_info` 调用会因状态未注册而失败（实测出现过设置页缺模型分组）。
- 测试机上 Alt+Q 已被其他常驻程序占用（应用未运行时 `RegisterHotKey(Alt+Q)` 同样失败，Alt+W 空闲），占用提示路径因此得到实测；Alt+Q 呼出本身未能在测试机上验证，迷你窗的显示 / 定位逻辑经按钮路径验证。
- 界面自动化检查方式：开发时用 `--config` 覆盖窗口的 `additionalBrowserArgs` 加上 `--remote-debugging-port`，通过 CDP 截图与执行脚本（WebView2 会忽略 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`，因为 Tauri 自己传了参数）。
