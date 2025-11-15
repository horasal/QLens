## QLens - 专为本地多模态大模型设计的交互式前端

QLens 是一个专为本地多模态（Qwen3-VL 系列）设计的交互式前端。
QLens 允许 LLM 使用**工具（Tools）**来主动观察图片。模型可以"觉得看不清"而主动放大图片的某个区域，或者在图片上绘制边界框（BBox）来辅助思考。

![Think with images](assets/demo.jpg)

Code Interpreter             |  Fetch Image from web
:-------------------------:|:-------------------------:
![](assets/code_run.png)  |  ![](assets/fetch.png)

---

## ✨ 核心特性 (Features)

- **👁️ 视觉思维链 (Think-with-Images)**: 支持模型在推理过程中调用工具，实现真正的视觉 CoT
- **🔍 内置视觉工具**:
  - `Zoom In`: 模型自主裁剪并放大图片特定区域以查看细节
  - `Draw BBox`: 在图片上绘制边界框进行标记或计数
  - `Code Interpreter`: 执行Javascript代码，还能用d3画图
  - `Fetch URL`: 下载分析HTML以及图片
  - `Image Memo`: 允许用SVG记笔记
- **⚡ 本地优先**:
  - 后端采用 Rust (Axum) 编写，极速响应，内存占用低
  - 前端采用 SvelteKit，提供流畅的流式（Streaming）体验
  - 采用嵌入式数据库（Sled），所有聊天记录存储在本地
- **兼容 llama.cpp**: 专为 llama.cpp 的 server 模式设计，开箱即用
- **流式体验**: 实时渲染 Markdown、LaTeX 公式，支持代码高亮，且能在流式传输中平滑展示工具调用过程

---

## 🛠️ 准备工作 (Prerequisites)

在使用 QLens 之前，你需要启动一个兼容OpenAI API的推理后端。推荐使用 [llama.cpp](https://github.com/ggml-org/llama.cpp)。

1. 下载 [Qwen3-VL](https://huggingface.co/collections/Qwen/qwen3-vl) 的 GGUF 模型
2. 启动 llama.cpp server：(推荐thinking系列以获得更好的工具性能)

```bash
./llama-server \
    -m qwen3-vl-30b-a3b-thinking-q5_k_m.gguf \
    --mmproj mmproj-bf16.gguf \
    --port 8080 \
    --ctx-size 32768
```

## 📦 安装与运行 (Installation)

### 方式一：直接下载 (推荐)

从 Releases 页面下载适合你系统的单一可执行文件，运行后访问`http://localhost:3000`即可开始使用。

```bash
# Linux/macOS
chmod +x qlens
./qlens

# Windows
qlens.exe
```

## 方式二：从源码构建

你需要安装 Rust 和 Deno。

```bash
# 构建前端
cd frontend_clean
deno Install
deno task build
cd ..

# 构建并运行后端 (Rust 会自动将构建好的前端文件嵌入到二进制中)
cargo build --release
```

## ⚙️ 配置参数 (Configuration)

QLens 支持通过命令行参数或配置文件进行高度定制。

### 常用启动命令

```bash
# 连接到本地 8080 端口的 llama.cpp，并将本服务运行在 3000 端口
qlens --provider http://127.0.0.1:8080 --port-serve 3000

# 使用配置文件，可以用--dump-config获取默认设置
qlens --config config.json

# 使用自定义的系统提示词语言（支持 Auto, English, Chinese, Korean, Japanese）
qlens --system-prompt-language Chinese
```

## 📝 使用指南 (Usage)

- 打开浏览器访问 http://127.0.0.1:3000
- 点击左侧 + 新对话
- 拖拽图片到输入框

## 📄 许可证 (License)

MIT License
