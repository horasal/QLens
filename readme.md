## QLens - Frontend Designed Specifically for Qwen3-VL series

[中文文档](readme_cn.md)

QLens is an interactive frontend designed specifically for local multimodal (Qwen3-VL series) large language models.
QLens allows LLMs to actively observe images using tools (Tools). Models can "feel unclear" and proactively zoom in on specific areas of the image or draw bounding boxes (BBox) on the image to aid in reasoning.

![DemoVideo](assets/Demo.mp4
<video src="assets/Demo.mp4" controls muted></video>

---

## ✨ Features

- **👁️ Think-with-Images**: Supports models to call tools during reasoning, enabling true visual CoT

![Demo](assets/demo.jpg)

- **🔍 Built-in Visual Tools:**

  - `Zoom In`: Models autonomously crop and zoom into specific regions of images to view details
  - `Draw BBox`: Drawing bounding boxes on images for labeling or counting
  - `Image Memo`: Allows note-taking with SVG
- **⚡ Local-First**:
  - Backend implemented in Rust (Axum) for ultra-fast response and low memory usage
  - Frontend built with SvelteKit for smooth streaming experience
  - Embedded database (Sled) storing all chat history locally
- **llama.cpp**: Specifically designed for llama.cpp's server mode, ready to use out-of-the-box
- **Frontend**: Real-time rendering of Markdown, LaTeX formulas, and code highlighting while smoothly displaying tool invocation during streaming

---

## 🛠️ Prerequisites
Before using QLens, you need to run a backend compatible with OpenAI API. I recommend using [llama.cpp](https://github.com/ggml-org/llama.cpp).

1. Download the [Qwen3-VL](https://huggingface.co/collections/Qwen/qwen3-vl)  GGUF Model
2. Start llama.cpp server：(Recommended thinking series for better tool performance)

```bash
./llama-server \
    -m qwen3-vl-30b-a3b-thinking-q5_k_m.gguf \
    --mmproj mmproj-bf16.gguf \
    --port 8080 \
    --ctx-size 32768
```

## 📦 Installation

### Option 1: Direct Download (Recommended)

Download the single executable file for your system from Releases page and run it:

```bash
# Linux/macOS
chmod +x qlens
./qlens

# Windows
qlens.exe
```

### Option 2: Build from Source

You'll need Rust and Deno installed.

```bash
cd frontend_clean
deno Install
deno task build
cd ..
cargo build --release
```

## ⚙️ Configuration

QLens supports highly customizable configuration via command-line flags or config files.

### Launch Command Examples

```bash
# Connect to local llama.cpp on port 8080, serve on port 3000
qlens --provider http://127.0.0.1:8080 --port-serve 3000

# Use config file (dump default with --dump-config)
qlens --config config.json

# Use custom system prompt language (Auto, English, Chinese, Korean, Japanese)
qlens --system-prompt-language Chinese
```

## 📝 Usage
- Open browser to http://127.0.0.1:3000
- Click + New Chat on the left
- Drag and drop images into input field

## 📄 License

MIT License
