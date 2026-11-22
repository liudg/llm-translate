# LLM Translate 详细设计 v1

> 状态：方案已与需求方确认（2026-11-20）；M0 技术验证已完成，结果见第 10 节；M1、M2、M3、M4、M5、M6 已完成。

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
| 视觉 | Fluent 风格；Win11 Mica / Win10 降级为 Fluent 纯色（浅 `#f3f3f3` / 深 `#202020`，避免 Win10 Acrylic 拖动卡顿）；深浅色跟随系统 |
| Win32 / UIA / DXGI | `windows` crate (windows-rs) |
| 推理 | llama.cpp 官方 release 的 `llama-server`，作为子进程（sidecar） |
| OCR | PP-OCRv6 Small（det + rec）ONNX，经 `oar-ocr` 0.11（内部 `ort`，ONNX Runtime 静态链接）；模型约 30 MB 随安装包 |
| 网络 | `reqwest`（流式、Range 断点续传）+ `tokio` |
| 插件 | tauri-plugin-global-shortcut、tauri-plugin-single-instance、tauri-plugin-notification；开机自启自行写 HKCU 注册表（见 5.7） |
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
│ capture   GDI 截屏 · 原生选区遮罩 · OCR · 段落合并 · 背景/文字色估计        │
└───────────────┬──────────────────────────────┬──────────────────────┘
                │ HTTP 127.0.0.1:随机端口(SSE)  │ Tauri IPC / events
        ┌───────▼────────┐            ┌────────▼─────────────────────────┐
        │ llama-server   │            │ WebView 窗口（预创建、隐藏待命）     │
        │ (cpu / vulkan) │            │ main · mini · popup（图标为原生窗口）│
        └────────────────┘            │ shot · wizard（截图遮罩为原生窗口）  │
                                      └──────────────────────────────────┘
```

### 4.1 目录结构

```
llm-translate/
├─ src-tauri/
│  ├─ src/
│  │  ├─ main.rs
│  │  ├─ app/        settings.rs tray.rs autostart.rs hotkey.rs windows.rs commands.rs logging.rs
│  │  ├─ deploy/     hardware.rs manifest.rs downloader.rs wizard.rs
│  │  ├─ inference/  supervisor.rs client.rs
│  │  ├─ translate/  detect.rs segment.rs prompt.rs scheduler.rs cache.rs
│  │  ├─ selection/  hook.rs icon.rs uia.rs clipboard.rs input.rs blacklist.rs
│  │  └─ capture/    screen.rs overlay.rs ocr.rs layout.rs
│  ├─ resources/     llama/cpu/ llama/vulkan/ ocr/ models.json
│  └─ tauri.conf.json
├─ src/              React 前端
│  ├─ windows/       main mini popup shot wizard
│  ├─ components/  stores/  styles/  lib/
└─ docs/DESIGN.md
```

### 4.2 数据目录

- 设置：`%APPDATA%\LLMTranslate\settings.json`；主窗口位置与大小：同目录 `window.json`（单独存放，移动窗口不触发 `settings-changed` 广播）
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

**线程模型**（M3 实现）
- 钩子线程：`WH_MOUSE_LL` 与图标窗同在一个带消息循环的线程。钩子回调只做原子变量运算并 `PostMessage` 给图标窗，手势判定在窗口过程里执行，回调不做任何阻塞操作（避免系统钩子超时被摘除）。键盘钩子 `WH_KEYBOARD_LL` 只在图标显示期间安装。划词总开关关闭时卸载鼠标钩子。
- 取词线程：MTA 初始化 COM，执行 UIA → Ctrl+Insert；UIA 用 `IUIAutomation2` 把连接 / 事务超时设为 200 ms。

**触发判定**（左键 down→up 期间）
- 拖动距离 ≥ 8 px（按 DPI 缩放），或双击（系统双击时间与范围内；三击同样触发），或 Shift+单击。三种方式在设置页各有开关。
- 排除：前台为本应用窗口、黑名单进程、桌面与任务栏（`Progman` / `WorkerW` / `Shell_TrayWnd`）、全屏独占应用（`SHQueryUserNotificationState` 为 D3D 全屏或演示模式）；拖动期间按下处的顶层窗口位置 / 大小发生变化（在拖窗口或改窗口大小）也不弹。
- 满足条件 → 在光标右下偏移处显示图标窗。
- 3 秒未点击（鼠标停在图标上时不计时）、鼠标离开图标超过 120 px（按 DPI 缩放）、按下非修饰键、滚轮、右键 / 中键 → 隐藏图标。修饰键不算，否则 Shift+单击后按住 Shift 的自动重复会立刻关掉图标。

**图标窗**（M3 实现）：原生 Win32 分层窗口而不是 WebView，`WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TOOLWINDOW`，并对 `WM_MOUSEACTIVATE` 返回 `MA_NOACTIVATE`。点击图标时焦点和选区都留在原应用，UIA 与 Ctrl+Insert 才能取到原应用的选区；同时因为本进程收到了这次点击，之后弹出浮窗时可以把它设为前台。图标为 24 DIP 强调色圆角方块 + 白色"译"字 + 白边与阴影，按显示器 DPI 软件绘制，悬停变亮。

**取词（仅在点击图标时执行）**
1. **UIA**：`GetFocusedElement` → 向上查找支持 `TextPattern` 的元素 → `GetSelection` → 拼接文本；按行去掉行尾填充空格（终端 UIA 会返回整行宽度的空格）。非空即成功。
2. **Ctrl+Insert**（不产生 0x03 中断信号）：
   - 保存剪贴板：枚举格式，复制可复制的 HGLOBAL 数据（单格式 > 10 MB 跳过）。
   - 记录 `GetClipboardSequenceNumber`。
   - `SendInput`：先等用户松开修饰键（最多 300 ms），仍按住的 Shift / Ctrl / Win 注入抬起；Alt 不注入抬起（单独的 Alt 抬起会激活很多应用的菜单栏），仍按住则放弃本次取词。然后发 Ctrl↓ Insert↓(扩展键标志) Insert↑ Ctrl↑。
   - 轮询序号最长 300 ms；变化则读 `CF_UNICODETEXT`。
   - 剪贴板以图标窗（钩子线程，持续处理消息）为所有者打开，避免其他程序清空剪贴板时阻塞在发给我们的 `WM_DESTROYCLIPBOARD` 上。
   - 恢复原剪贴板，并给恢复操作附加 `ExcludeClipboardContentFromMonitorProcessing` 格式，避免原内容在 Win+V 历史里重复出现。已知限制：被复制的选中文本由目标应用写入，无法标记排除，会出现在 Win+V 历史中（仅 UIA 失败走此路径时）。
   - 剪贴板序号变化但内容为空（如 conhost 快速编辑模式单击后留下零宽选区）视为取词失败。
3. **失败**：浮窗提示"该应用不支持划词，可使用截图翻译（Alt+W）"。
4. **绝不发送 Ctrl+C**；因此无需识别终端类型。

**译文浮窗**：出现在选区附近，流式显示译文；失焦自动关闭，可"钉住"；支持复制、重新翻译。M3 实现（需求方确认，2026-11-20）：
- 顶部一行显示原文（默认折叠成一行，点击展开全文）与方向按钮（点击循环"自动 → 中→英 → 英→中"并重译，每次新划词重置为自动）、钉住、关闭；中间流式译文；底栏方向 / 耗时、停止 / 重新翻译、复制译文。Esc 关闭。
- 取词完成后才显示并设为前台（取词前抢焦点会让取词落到自己身上）；`SetForegroundWindow` 被拒时临时 `AttachThreadInput` 到前台线程再设置。
- 位置：宽 440 逻辑像素，放在选区（手势按下 / 松开点的外接矩形，上下各留约一行）下方；下方放不下而上方空间更大时放在上方，此后高度变化保持底边不动。高度随内容变化，译文区最高 300 px 后滚动。
- 钉住后失焦不关闭，新的划词结果显示在这个浮窗里且不移动位置；关闭即取消钉住。浮窗隐藏时停止正在进行的翻译。
- 取词失败时只显示失败提示，不保留上一次的译文。

**黑名单**（需求方确认，2026-11-20）：按进程名匹配，不区分大小写，默认为空。设置页可从"正在运行的程序"（有可见窗口的进程，带窗口标题，打开时才读取）中点选，也可手动输入进程名（省略 `.exe` 时自动补上）。

### 5.6 截图翻译（capture）

M4 实现（需求方确认，2026-11-20：结果窗只保留一个；只支持拖拽框选；工具条在原设计基础上加"复制原文"；多显示器不测）。

1. **触发**：Alt+W（固定注册，见 5.7），或翻译页"截图"按钮（先最小化主窗口再截屏）。截屏前先隐藏上一次的结果窗；会话进行中再按 Alt+W 忽略。
2. **截屏**：每个显示器一次 GDI `BitBlt(SRCCOPY | CAPTUREBLT)`，线程设为 Per-Monitor DPI Aware V2，坐标全部为物理像素。原计划的 DXGI Desktop Duplication 不再使用：一次性截屏 BitBlt 已足够快（2560×1600 实测 67 ms，debug 构建），且不需要 D3D 设备，旋转屏、接在另一块显卡上的屏幕（混合显卡笔记本上 DXGI 返回 UNSUPPORTED）也都能截。
3. **选区**：每个显示器一个**原生 Win32** 全屏置顶窗口（不用 WebView，按下即显示、无需把位图传进 WebView、物理像素无换算误差）。选区外压暗，选区内原亮度，外侧描边，左上角显示"宽 × 高"，未拖动时顶部居中提示"拖动鼠标框选要翻译的区域　Esc 取消"。选区限于开始拖动的显示器。Esc 取消；右键在拖动中清除本次拖动，否则取消；切到其他应用（失去激活）也取消。松开鼠标后遮罩保持显示，直到结果窗画好并显示在它上面才关闭，避免闪出实时桌面（页面 3 秒内未回报则强制关闭）。
4. **OCR**：PP-OCRv6 Small（det + rec + 字典，约 31 MB，`scripts/fetch-ocr.ps1` 拉到 `src-tauri/resources/ocr/`，随安装包）。按下 Alt+W 时即在后台加载引擎（与用户拖选重叠，实测加载 354 ms），闲置 1 分钟释放：引擎常驻时应用内存约 364 MB，释放后 93 MB（工作集，debug 构建；ONNX Runtime 的 CPU 内存池 oar-ocr 未提供关闭选项），而重新加载的耗时被拖选掩盖。检测参数用 PP-OCRv6 官方默认值。过滤置信度 < 0.8 且只有 1 个字符的结果（窗口按钮等图标）。
5. **段落合并**：下一行紧接上一行（间距 < 0.7 行高）、行高相近（0.75–1.33 倍）、左边缘相近（< 1.5 行高）、水平方向有重叠，且上一行足够长（≥ 8 倍行高）并到达段落右边缘（像是自动换行）时才合并。菜单、列表、表格单元格等短行各自独立，保留各自位置。中文行直接相接，英文行加空格，行尾连字符断词自动接回。阅读顺序为从上到下、同一行从左到右。
6. **颜色与可用空间**（Rust 端计算）：背景色取段落框外扩 2 px 一圈像素的逐通道中位数；文字色取框内与背景差异最大的前 10% 像素的中位数，差异过小时按背景亮度取近黑 / 近白。另外算出段落右侧、下方连续为背景色且不碰其他段落的空白范围（最多到原框的 2 倍），供译文变长时使用。
7. **翻译**：翻译调度新增批量接口 `translate_batch`：一张截图的所有可翻译段落算一个请求（一次取消），逐段推送 `delta` / `itemDone`；新的划词、输入或截图请求照常顶替它。方向自动、逐段判定；不含字母的段落（数字、符号）不送模型，原图保持不变。
8. **渲染**（结果窗 Canvas，按物理像素 1:1 绘制）：
   - 用背景色填充段落框（外扩 2 px）擦除原文，用文字色绘制译文。
   - 字号起点 = OCR 框高 × 0.9（实测原字号 / 框高为 0.73–1.16，多数约 0.95），行距 1.28 倍；放不下时逐步缩小，下限 9 DIP，允许换行（中文逐字断行，英文按词，避头标点）。
   - 若在原框内需要缩到起点的 85% 以下（典型为中译英变长），改在原框加右侧 / 下方空白范围内重新排版，取字号更大者；扩展部分同样先擦除。
   - 译文流式到达即重绘（每帧最多一次）。
9. **结果窗**（WebView `shot`，无边框、置顶、透明、无阴影、不进任务栏，失焦不关闭）：图像位于原选区的物理位置，与原画面严丝合缝；图像外圈 1 px 半透明强调色描边。工具条（高 44 DIP，最小宽 360 DIP）默认在图像下方，下方放不下放上方，都放不下则盖在图像底部；选区比工具条窄且靠近屏幕右边缘时，窗口向左延伸，图像位置不变，多出的区域透明。工具条：状态（正在识别文字 / 正在加载模型 / 正在翻译 n/m / 已翻译 n 段 / 未识别到文字 / 没有需要翻译的文字 / 被中断 / 错误）、显示原图 / 译文切换、复制全部译文（Ctrl+C）、复制原文、复制图片（复制当前显示的一面，24 位 CF_DIB）、关闭（Esc）。拖动图像或工具条空白处可移动窗口。新截图替换旧结果。

### 5.7 托盘、自启、快捷键

M5 实现（需求方确认，2026-11-20：关闭主窗口隐藏到托盘并首次提示；划词开关快捷键只用托盘图标变灰反馈；自启在向导完成页勾选、首次部署默认勾上；录制的快捷键被占用时不保存、保留原快捷键；记住主窗口大小位置）。

- **托盘**：菜单为 输入翻译 / 截图翻译（右侧显示当前生效的快捷键，Win32 菜单文字里的 Tab 右对齐）/ 划词翻译（勾选项）/ 设置 / 退出。左键单击打开主窗口；"设置"打开主窗口并切到设置页（`navigate` 事件）。未部署时这些入口改为打开部署向导。划词翻译关闭时托盘图标换成灰色半透明版本（运行时由应用图标去色生成），提示文字加"（划词翻译已关闭）"；托盘、设置页、快捷键三处切换共用同一个设置项，互相同步。
- **关闭主窗口**：隐藏到托盘，不退出；第一次关闭时发一条 Windows 通知"LLM Translate 仍在运行……可在托盘图标的菜单中退出"，之后不再提示（`trayHintShown`）。只能从托盘"退出"。部署完成前关闭向导仍直接退出。M6 已确认：NSIS 安装版在开始菜单快捷方式写入 `System.AppUserModel.ID = com.llmtranslate.app`，`tauri-plugin-notification` 识别非 `target/` 目录后自动附带该 `app_id`。
- **主窗口位置**：`GetWindowPlacement` 的正常矩形（工作区坐标）与是否最大化存入 `window.json`；隐藏前、退出前以及移动 / 改变大小停止 0.8 秒后保存。启动时窗口仍隐藏，用 `SetWindowPlacement(SW_HIDE)` 套用矩形，首次显示时再按需最大化；矩形不在任何现有显示器上（`MonitorFromRect` 为空）则忽略，保持居中。
- **单实例**：tauri-plugin-single-instance（第一个注册的插件）；再次启动时新进程退出，已有实例显示主窗口（未部署时显示向导）。
- **开机自启**：`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 值 `LLM Translate` = `"<exe>" --minimized`。不用 tauri-plugin-autostart：它的 Windows 实现（auto-launch 的 Dynamic 模式）在进程以管理员身份运行时会写到 **HKLM**，对全机生效，与"当前用户安装"不符（M5 实测）。状态每次从注册表读取；任务管理器中"禁用"（`Explorer\StartupApproved\Run` 首字节为奇数）视为关闭，在设置中重新打开时清除该标记。以 `--minimized` 启动时只驻留托盘，照常预加载模型；未部署时仍打开向导。部署向导完成页有"开机自动启动"复选框：首次部署默认勾上，重新部署时显示当前状态。M6 卸载时需删除该注册表值。
- **快捷键**：三项——输入翻译（默认 Alt+Q）、截图翻译（默认 Alt+W）、划词翻译开关（默认不绑定），以显示文本（如 `Ctrl+Alt+Q`）存于设置 `hotkeys`。
  - 设置页点击快捷键框开始录制：录制期间注销本应用全部全局快捷键（否则按下已绑定的组合会直接触发），结束后重新注册。必须包含 Ctrl / Alt / Win 之一（Shift 只能作为附加键）；Esc 取消，Backspace / Delete 解除绑定；切到其他窗口即取消录制。
  - 冲突检测：与本应用另一项相同 → 提示"与'截图翻译'的快捷键相同"；先尝试注册新组合，失败 → 提示"Ctrl+X 已被其他程序占用"，不保存，原快捷键继续生效。
  - 启动时注册失败的快捷键在设置页标"未生效"，主窗口顶部横幅提示并可跳到设置页更换。重新录制同一组合即重试注册。
  - 插件的 register / unregister 会同步切到主线程执行，而快捷键回调也在主线程取我们的锁，因此注册调用期间不持有本模块的锁，相关命令为 async。
  - 划词开关快捷键只翻转设置里的总开关（托盘图标随之变灰 / 恢复），不另弹提示。

### 5.8 设置页

常规（自启、主题）· 快捷键 · 划词（开关、黑名单、触发方式细项）· 模型（已下载列表、下载/切换/删除、后端选择、闲置释放）· 关于（版本、日志目录）。

- 设置页是主窗口内的一个页面（左侧导航切换），Win11 设置风格的分组卡片。
- M2 可用：
  - 主题：跟随系统 / 浅色 / 深色，所有窗口同步；同时设置原生窗口主题（标题栏、Mica 色调）与页面 `data-theme`。
  - 模型：当前模型、状态、目录；推理后端（CPU / 检测到的各 Vulkan 设备）与 CPU 线程数，"应用并重新加载模型"后在后台重启推理进程；闲置释放（从不 / 5 / 10 / 30 / 60 分钟，立即生效）；"更换模型 / 重新部署"打开向导。显卡列表用 `llama-server --list-devices` 检测，不停止正在运行的模型，首次打开设置页时才检测。
  - 关于：版本、系统 build 与窗口效果、设置文件位置、打开日志目录。
- M3 可用：划词翻译总开关（默认开）、三种触发方式开关、黑名单。
- M4 可用：截图翻译快捷键一行显示 Alt+W 的真实状态（已生效 / 未生效）。
- M5 可用：
  - 常规：开机自启开关（每次打开设置页从注册表重新读取）。
  - 快捷键：三项都可录制，显示已生效 / 未生效与错误原因（见 5.7）。
  - 模型管理：模型分组顶部列出 `models.json` 中全部模型，显示大小、建议显存 / 内存与状态（使用中 / 已下载 / 已下载 n% / 未下载）。未下载可"下载"，部分下载可"继续下载"（断点续传）或删除；下载中显示进度、速度与下载源，可暂停 / 取消（取消删除 `.part`）。已下载可"使用"：按已保存的后端与向导相同的启动 + 自检（翻译"你好，世界"），GPU 失败自动退回 CPU 并提示；CPU 也失败则重新加载原模型并提示"已恢复为原来的模型"，设置只在成功后保存。删除需二次确认；正在使用的模型、正在下载的模型不能删除。模型目录仍只在向导中修改（"重新部署"）。
- llama-server 的错误日志带 ANSI 颜色码，拼进界面错误信息前先去掉（M5 发现，向导错误面板同样受益）。
- 每次设置变更都广播 `settings-changed` 事件，各窗口据此同步（含 GPU 失败自动退回 CPU 的变更）。

## 6. 安装包

M6 实现（需求方确认，2026-11-22：沿用 Tauri NSIS 当前用户默认安装目录；卸载时安全清理已知模型文件；替换 Fluent 蓝色"译"字应用图标；版本号保持 0.1.0）：

- NSIS（`targets: ["nsis"]`，`installMode: "currentUser"`，`languages: ["SimpChinese"]`，LZMA 固实压缩），当前用户默认安装到 `%LOCALAPPDATA%\LLM Translate`（安装时可自选目录），免管理员（`RequestExecutionLevel user`）。
- 内容：主程序（32.9 MiB，含静态链接的 ONNX Runtime）、llama.cpp Vulkan 版所需文件（约 89 MB 未压缩）、OCR 模型（约 31 MB，`resources/ocr/`）、`models.json`、运行时 DLL（`DirectML.dll` 1.15.4 + VC++ CRT）、内嵌 WebView2 bootstrapper；压缩后最终安装包 **56.91 MiB**（< 100 MB 目标）。
- 运行时 DLL 应用本地部署（`scripts/prepare-runtime.ps1`）：
  - 主程序同目录（`resources/runtime/` → `$INSTDIR\`）：`DirectML.dll`（取自 `ort-cache` 实体文件，18.5 MB）、`msvcp140.dll`、`msvcp140_1.dll`、`vcruntime140.dll`、`vcruntime140_1.dll`。
  - `llama/` 子目录（`resources/llama/` → `$INSTDIR\llama\`）：`msvcp140.dll`、`vcruntime140.dll`、`vcruntime140_1.dll`。
  - M6 已检查 release 版 `llm-translate.exe` PE 导入表（OS/Subsystem 6.0，无 delay-load，除系统 DLL / UCRT / `d3d12.dll` 外全部由上述 5 个 DLL 满足），并在安装版进程模块列表确认主程序与 `llama-server.exe` 均实际加载安装目录内的应用本地 DLL。
- 构建：`src-tauri/.cargo/config.toml` 把 `ORT_CACHE_DIR` 设为 `target/ort-cache`（默认位置在 C 盘，跨盘重命名会失败）；执行 `cargo` 命令需在 `src-tauri/` 目录下或通过 `pnpm tauri build`。
- WebView2：`webviewInstallMode: { type: "embedBootstrapper", silent: true }`（内嵌 `MicrosoftEdgeWebview2Setup.exe`，Win10 未预装时自动静默引导安装）。
- 卸载（`src-tauri/nsis/hooks.nsh`，UTF-8 with BOM）：
  - 非升级卸载（`$UpdateMode <> 1`）：删除 `HKCU\...\Run\LLM Translate`（Tauri 模板内置）、`Explorer\StartupApproved\Run\LLM Translate` 及 `HKCU\Software\LLMTranslate`；勾选"删除应用程序数据"时同步删除 `%APPDATA%\LLMTranslate` 与 `%LOCALAPPDATA%\LLMTranslate\logs`。
  - 模型清理：保存设置与启动时将 `modelsDir` 同步写入 `HKCU\Software\LLMTranslate\ModelsDir`（原生 UTF-16，支持中文/自定义路径）；非静默非升级卸载时检测到已知模型 `.gguf` / `.gguf.part` 则弹窗询问是否一并删除，选"是"时仅删除清单中的已知模型文件并仅在空目录时 `RMDir`，绝不误删自定义目录中的其他文件。升级重装（`/UPDATE`）时跳过清理，保留自启、设置与模型。

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
| Win10 Acrylic 拖动卡顿 | M6 已按系统版本（`build < 22000`）统一降级为 Fluent 纯色背景（浅 `#f3f3f3` / 深 `#202020`，关闭窗口透明并同步 WebView2 底色） |

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

- M6 验收（2026-11-22，Win11 build 26200 本机打包与性能实测；Win10 1809+ 按需求方要求通过代码与 PE 依赖静态检查）：
  - **安装包**：`LLM Translate_0.1.0_x64-setup.exe`（**56.91 MiB**，< 100 MB），简体中文界面，免管理员当前用户安装到 `%LOCALAPPDATA%\LLM Translate`，Fluent 蓝色"译"字图标，内嵌 WebView2 bootstrapper；开始菜单快捷方式已验证含 `PKEY_AppUserModel_ID` = `com.llmtranslate.app`。
  - **应用本地 DLL**：`llm-translate.exe` 实际从安装目录加载 `DirectML.dll`（1.15.4）、`MSVCP140.dll`、`MSVCP140_1.dll`、`VCRUNTIME140.dll`、`VCRUNTIME140_1.dll`；`llama-server.exe` 实际从 `llama/` 目录加载 `MSVCP140.dll`、`VCRUNTIME140.dll`、`VCRUNTIME140_1.dll`。
  - **性能优化**：`wizard` 与 `shot` 改为按需懒创建（已部署启动仅预创建 `main`/`mini`/`popup` 3 个窗口，`wizard` 结束后销毁，`shot` 在截图拖选期间后台预热），启动耗时由 ~640–790 ms 降至 **472–496 ms**（冷启 678 ms），WebView2 渲染进程由 5 个减至 3 个（节省 ~138 MB 工作集 / ~109 MB 私有内存）。
  - **性能实测 vs 第 7 节目标**（全部达标）：
    - 启动到托盘就绪（不含模型加载）：**472–496 ms**（冷启 678 ms，目标 < 1 s）✅
    - 2B 模型加载（SSD）：CPU **1.9 s**、Vulkan GPU **1.9–2.2 s**（目标 < 3 s）✅
    - 划词点击 → 首字（2B，短句，CPU）：UIA 取词 6–27 ms（M3 实测）+ 安装版 2B CPU 首字 **144–163 ms** ≈ **150–190 ms**（总耗时 353–713 ms；目标 < 500 ms）✅
    - 截图框选完成 → OCR 完成（1180×600，10 行中英混排，release 构建）：引擎冷加载 ~175 ms（与拖选重叠），热识别 **216–223 ms** + 段落合并 2 ms = **218–225 ms**（目标 < 300 ms）✅
    - 应用自身内存（不含模型，空闲）：`llm-translate.exe` 工作集 **43.0 MB** / 私有 **19.5 MB**（目标 < 120 MB）✅
  - **安装版行为与卸载验证**：关闭主窗口隐藏到托盘并触发首次通知（无错误、`trayHintShown` 置 true）；二次启动激活已有实例；`--minimized` 驻留托盘不弹窗；`/UPDATE /S` 升级卸载保留自启、设置与模型；`/S` 静默卸载清理安装目录与 `HKCU\...\Run` / `StartupApproved\Run` / `HKCU\Software\LLMTranslate` 并保留模型；交互式卸载勾选"删除应用程序数据"同步清理设置与日志，含中文路径的自定义 `modelsDir` 弹窗无乱码，选"是"仅安全删除已知 `.gguf`/`.part` 模型文件且保留同目录用户个人文件。
  - **Win10 1809 (build 17763)+ 代码检查通过**：`windows.rs` 对 `build < 22000` 统一降级为纯色（`transparent(false)` + WebView2 底色 + `data-effect="solid"` + 浮窗 1 px 内描边），单测覆盖 `17763`/`19045`/`21999`/`22000`/`26200`；全量审计 Rust Win32 API、依赖 crate（`tao` 暗色标题栏支持 `>= 17763`、`tauri-plugin-single-instance`、`tauri-winrt-notification`）及 release PE 导入表（Subsystem 6.0、所有 API Set 均为 Win10 1809 自带、`DirectML.dll` 1.15.4 最低支持 Win10 1809）。
- M5 验收（2026-11-20，Win11 build 26200，开发构建；界面经 CDP 驱动，快捷键用真实按键 `keybd_event` 触发，托盘菜单"退出"由需求方手动验证）：
  - 快捷键：录制 Ctrl+Alt+Q 后真实按键可呼出 / 隐藏迷你窗（测试机上 Alt+Q 被其他程序占用，启动时标"未生效"并有横幅）；录制 Alt+W 给划词开关提示与截图翻译相同；录制 Alt+Q 提示已被其他程序占用且原快捷键不变；Shift+A 被拒绝；Esc 取消。划词开关绑定 Ctrl+Alt+S 后真实按键切换总开关并写入设置。
  - 托盘：菜单项与快捷键文字、勾选状态正确；划词关闭时图标变灰、开启恢复；"设置"打开主窗口并切到设置页；"退出"正常（需求方验证），llama-server 随之结束。
  - 关闭主窗口隐藏到托盘，首次弹出通知，第二次不再提示；再次启动 exe 时新进程退出、已有实例显示主窗口。
  - 重启后：自定义快捷键全部重新注册；主窗口位置与大小与关闭前一致（300,200–1800,1200）；`--minimized` 启动不显示任何窗口，9B 在 5 s 内预加载完成。
  - 开机自启：开关写入 / 删除 HKCU Run（以管理员身份运行时也不写 HKLM）；模拟任务管理器禁用后读取为关闭，重新打开后清除禁用标记。
  - 模型管理：下载 2B（ModelScope 约 31 MB/s），中途暂停显示"已下载 8%"并保留 `.part`，继续下载完成并通过校验；切换到 2B（GPU 加载 2.1 s，自检 "Hello, world!"）；删除正在使用的模型被拒绝；用同体积的全零占位文件模拟损坏的 7B，切换时 GPU、CPU 均失败并自动恢复 2B，设置未改动；下载 7B 时删除被拒绝，取消后 `.part` 被删除；切回 9B（10.0 s）；二次确认删除 2B。测试结束后模型目录与自启状态恢复原样。
- M4 验收（2026-11-20，Win11 build 26200，单屏 2560×1600 @150%，9B GPU）：需求方确认多显示器不测（无环境）。用 DPI 感知的 Tk 测试窗（中英标题、三行英文段落、菜单列表、中文行、深色块白字、版本号）+ SendInput 驱动真实应用：
  - 位置：结果窗图像与截屏前的屏幕逐像素比对，译文区域以外 0 个像素不同（偏移 1 px 则有 240–2280 个不同）；"显示原图"与原屏幕 0 差异；"复制图片"与屏幕显示 0 差异。150% 的非整数缩放下 Canvas 为 1:1 绘制。
  - 内容：三行英文合并为一段，菜单三项各自独立，深色块正确取色（白字），中译英变长时扩展到右侧空白并换行；10 行识别出 8 段译文，复制译文 / 复制原文按阅读顺序。
  - 交互：Esc、右键取消遮罩；屏幕右下角的窄选区工具条在上方、窗口向左延伸且图像不动；再次 Alt+W 先隐藏旧结果；翻译页"截图"按钮先最小化主窗口再出遮罩；结果窗 Esc 关闭。
  - 耗时（debug 构建）：截屏 67 ms；OCR 引擎加载 354 ms（与拖选重叠）；1180×600 识别 10 行 453 ms（同图 release 构建的 spike 为 211 ms）。
- M3 验收范围（需求方确认，2026-11-20）：终端只用 Windows 自带终端验证，即 conhost 与 Windows Terminal（cmd / PowerShell）。其余终端与应用不在 M3 验收内。
- M3 验收（2026-11-20，Win11 build 26200，`src-tauri/examples/m3_check.rs`）：
  - `capture` 模式：conhost、Windows Terminal（cmd）、Windows Terminal（PowerShell）中各自运行 `ping -t`。无选区时 Ctrl+Insert、拖选后 UIA 取词、拖选后仅用 Ctrl+Insert 取词，ping 均存活；Ctrl+Insert 后用户剪贴板原样恢复。对照组真实 Ctrl+C 会终止 ping，证明测试有效。
  - `e2e` 模式（应用以 `LLMT_ACCEPT_INJECTED=1` 调试运行，钩子才接受注入的鼠标事件）：同样三个终端中拖选后图标出现，点击图标后浮窗显示并成为前台窗口，ping 均存活；日志显示取词走 UIA，耗时 6–27 ms。
  - 真实应用中另外验证了：记事本划词，从点图标到 9B GPU 出完整译文 0.86 s；空文件夹的资源管理器里取词失败时显示提示；黑名单加入 notepad.exe 后记事本不再弹图标，移除后恢复；总开关关闭 / 打开会卸载 / 重装钩子；设置页的程序选择器正常。

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
