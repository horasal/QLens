use crate::blob::{BlobStorage, BlobStorageError};
use crate::parse_sourcecode_args;
use crate::{MessageContent, Tool, ToolDescription, tools::FONT_DATA};
use deno_error::JsError;
use resvg::{tiny_skia, usvg};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::sync::{Arc, mpsc};
use uuid::Uuid;

use anyhow::{Error, anyhow};
use deno_core::{JsRuntime, OpState, RuntimeOptions, extension, op2, scope, v8};

#[derive(Deserialize, JsonSchema)]
pub struct JsInterpreterArgs(pub String);

#[async_trait::async_trait]
impl Tool for JsInterpreter {
    fn name(&self) -> String {
        "js_interpreter".to_string()
    }

    fn description(&self) -> ToolDescription {
        let raw_schema = serde_json::json!({
            "type": "string",
            "description": "The javascript source code to execute."
        });
        let capabilities = generate_capabilities_prompt();
        let description = format!(
            r##"Executes JavaScript code in a V8 sandbox with DOM support.
        **Capabilities:**
        {capabilities}
        **I/O & Files:**
        - **Print:** `console.log/error(...)` or `return ...`
        - **Images:** Images can be stored to the database by function `save_svg(svg_string):UUID` or `save_image(png_or_bmp: UInt8Array):UUID`.
        - **Image Access:** You can read these images using `fs.readFileSync(uuid)` as if they were local files.
          - Convert any valid image to png with `convert_to_png(UInt8Array)`.
        - **MemFS**: You have 50MB MemFS as a storage in your program.

        **Restrictions:**
        - NO Network (`fetch` is disabled). Use `curl_url` tool for networking.
        - NO Canvas. Use d3.js or UPNG for graphics.
        - Syntax: Top-level `await` is supported."##
        );

        ToolDescription {
            name_for_model: "js_interpreter".to_string(),
            name_for_human: "Javascript代码执行工具".to_string(),
            description_for_model: description,
            parameters: raw_schema,
            args_format: "输入内容**直接作为**js代码执行".to_string(),
        }
    }
    async fn call(&self, args: &str) -> Result<Vec<MessageContent>, anyhow::Error> {
        let code = parse_sourcecode_args(args)?;
        let db = self.db.clone();
        let result = tokio::task::spawn_blocking(move || run_code(db, code)).await??;

        let mut v = vec![MessageContent::Text(
            result.terminal + "\nReturn: " + &result.return_value,
        )];
        for (idx, &uuid) in result.uuids.iter().enumerate() {
            v.push(MessageContent::ImageRef(
                uuid,
                format!("JS Generated Image#{}", idx),
            ));
        }
        Ok(v)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CodeResult {
    return_value: String,
    terminal: String,
    #[serde(skip)]
    uuids: Vec<Uuid>,
}

struct LogSender(mpsc::Sender<String>);
struct ImageSender(mpsc::Sender<Uuid>);
struct DbHandle(Arc<dyn BlobStorage>);

#[op2(fast)]
fn console_op_print(state: &mut OpState, #[string] msg: String, is_err: bool) {
    if let Some(sender) = state.try_borrow::<LogSender>() {
        let prefix = if is_err { "[stderr] " } else { "" };
        let _ = sender.0.send(format!("{}{}", prefix, msg));
    }
}

#[derive(Debug, thiserror::Error, JsError)]
#[class(generic)]
enum ImageError {
    #[error("image binary is empty")]
    ImageEmpty,
    #[error("Invalid UUID {0}")]
    InvalidUuid(#[from] uuid::Error),
    #[error("Database save error {0}")]
    DatabaseError(#[from] BlobStorageError),
    #[error("Limit reached, can not save image any more")]
    MaxTries(usize),
    #[error("Invalid base64 {0}")]
    InvalidBase64(#[from] base64::DecodeError),
    #[allow(dead_code)]
    #[error("Uuid collision occured, please try again")]
    UuidCollision,
    #[error("Invalid SVG data {0}")]
    InvalidSVG(#[from] usvg::Error),
    #[error("Unable to create Pixmap with size {0}x{1}")]
    InternalErrorCreatePixMap(u32, u32),
    #[error("Unable to convert Pixmap to PNG.")]
    InternalErrorConvertPixMapToPng,
    #[error("Image IO Error, unable to save as PNG {0}.")]
    ImageIOError(#[from] image::error::ImageError),
}

struct Counter {
    put_count: usize,
}

const MAX_IMAGE_PUT_TRIES: usize = 10;

#[op2]
#[string]
fn op_save_svg(state: &mut OpState, #[string] svg_data: &str) -> Result<String, ImageError> {
    let mut font_db = usvg::fontdb::Database::new();
    font_db.load_font_data(FONT_DATA.to_vec());
    let family = font_db
        .faces()
        .next()
        .and_then(|x| x.families.first())
        .map(|x| x.0.to_string())
        .unwrap_or("MapleMono-NF-CN-Regular".to_string());

    font_db.set_sans_serif_family(&family);
    font_db.set_serif_family(&family);
    font_db.set_monospace_family(&family);
    font_db.set_cursive_family(&family);
    font_db.set_fantasy_family(&family);
    let usvg_options = usvg::Options {
        fontdb: Arc::new(font_db),
        font_family: family,
        ..Default::default()
    };

    let tree = usvg::Tree::from_str(svg_data, &usvg_options)?;

    let svg_size = tree.size();
    let width = svg_size.width().ceil() as u32;
    let height = svg_size.height().ceil() as u32;

    if width == 0 || height == 0 {
        return Err(ImageError::ImageEmpty);
    }

    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or(ImageError::InternalErrorCreatePixMap(width, height))?;

    pixmap.fill(tiny_skia::Color::TRANSPARENT);

    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());

    let output_buf = pixmap
        .encode_png()
        .map_err(|_| ImageError::InternalErrorConvertPixMapToPng)?;

    let db = state.borrow::<DbHandle>();
    match db.0.save(&output_buf) {
        Ok(uuid) => {
            if let Err(e) = state.borrow::<ImageSender>().0.send(uuid.clone()) {
                tracing::warn!("Error to send svg from javascript back to llm, {}.", e)
            }
            Ok(uuid.to_string())
        }
        Err(e) => Err(ImageError::DatabaseError(e)),
    }
}

#[op2]
#[string]
fn op_save_image(state: &mut OpState, #[buffer] img: &[u8]) -> Result<String, ImageError> {
    let _ = image::guess_format(img)?;
    if let Some(mut c) = state.try_take::<Counter>() {
        if c.put_count >= MAX_IMAGE_PUT_TRIES {
            return Err(ImageError::MaxTries(c.put_count));
        }
        c.put_count += 1;
        state.put(c);
    } else {
        state.put(Counter { put_count: 1 });
    }
    let db = state.borrow::<DbHandle>();
    match db.0.save(&img) {
        Ok(uuid) => {
            if let Err(e) = state.borrow::<ImageSender>().0.send(uuid.clone()) {
                tracing::warn!("Error to saved image from javascript back to llm, {}.", e)
            }
            Ok(uuid.to_string())
        }
        Err(e) => Err(ImageError::DatabaseError(e)),
    }
}

#[op2]
#[buffer]
fn op_convert_to_png(_: &mut OpState, #[buffer] img: &[u8]) -> Result<Vec<u8>, ImageError> {
    let mut v = Vec::new();
    let mut c = Cursor::new(&mut v);
    image::load_from_memory(img)?.write_to(&mut c, image::ImageFormat::Png)?;
    Ok(v)
}

#[op2]
#[buffer]
fn op_retrieve_image(
    state: &mut OpState,
    #[string] uuid_str: String,
) -> Result<Vec<u8>, ImageError> {
    let db = state.borrow::<DbHandle>();
    let uuid = uuid::Uuid::parse_str(&uuid_str).map_err(|e| ImageError::InvalidUuid(e))?;
    match db.0.get(uuid) {
        Ok(Some(bytes)) => Ok(bytes),
        Ok(None) => Err(ImageError::ImageEmpty),
        Err(e) => Err(ImageError::DatabaseError(e)),
    }
}

#[op2(fast)]
fn op_contain_image(state: &mut OpState, #[string] uuid_str: String) -> Result<bool, ImageError> {
    let db = state.borrow::<DbHandle>();
    let uuid = uuid::Uuid::parse_str(&uuid_str).map_err(|e| ImageError::InvalidUuid(e))?;
    match db.0.get(uuid) {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(e) => Err(ImageError::DatabaseError(e)),
    }
}

extension!(
    sandbox_ext,
    ops = [
        console_op_print,
        op_retrieve_image,
        op_save_image,
        op_save_svg,
        op_contain_image,
        op_convert_to_png
    ],
);

pub struct JsInterpreter {
    db: Arc<dyn BlobStorage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LibCategory {
    Environment,    // 基础环境 (DOM, Base64)
    DataProcessing, // 数据处理 (Lodash, Math, CSV)
    Visualization,  // 可视化 (D3, Plot)
    Utility,        // 其他工具
}

impl std::fmt::Display for LibCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LibCategory::Environment => write!(f, "Environment"),
            LibCategory::DataProcessing => write!(f, "Data Processing"),
            LibCategory::Visualization => write!(f, "Visualization"),
            LibCategory::Utility => write!(f, "Utilities"),
        }
    }
}

struct LibraryConfig {
    require_name: &'static str, // 用于 require('name')
    global_var: &'static str,   // 注入到 globalThis 的变量名
    src: &'static str,          // 源码内容
    category: LibCategory,      // 分类
    prompt_hint: &'static str,  // Prompt 中的展示文本 (例如: "`_` (Lodash)")
}
const LOAD_SOURCE: &[LibraryConfig] = &[
    // --- Environment ---
    LibraryConfig {
        require_name: "linkedom",
        global_var: "LinkeDOM",
        src: include_str!("prelude/linkedom.bundle.js"),
        category: LibCategory::Environment,
        prompt_hint: "`document`, `window`, `svg` (LinkeDOM simulated)",
    },
    LibraryConfig {
        require_name: "base64",
        global_var: "Base64",
        src: include_str!("prelude/base64.min.js"),
        category: LibCategory::Environment,
        prompt_hint: "`Base64` (Standard Base64)",
    },
    // --- Data Processing ---
    LibraryConfig {
        require_name: "lodash",
        global_var: "_",
        src: include_str!("prelude/lodash.min.js"),
        category: LibCategory::DataProcessing,
        prompt_hint: "`_` (Lodash)",
    },
    LibraryConfig {
        require_name: "mathjs",
        global_var: "math",
        src: include_str!("prelude/math.min.js"),
        category: LibCategory::DataProcessing,
        prompt_hint: "`math` (Mathjs)",
    },
    LibraryConfig {
        require_name: "dayjs",
        global_var: "dayjs",
        src: include_str!("prelude/dayjs.min.js"),
        category: LibCategory::DataProcessing,
        prompt_hint: "`dayjs`",
    },
    LibraryConfig {
        require_name: "papaparse",
        global_var: "Papa",
        src: include_str!("prelude/papaparse.min.js"),
        category: LibCategory::DataProcessing,
        prompt_hint: "`Papa` (CSV)",
    },
    LibraryConfig {
        require_name: "arquero",
        global_var: "aq",
        src: include_str!("prelude/arquero.min.js"),
        category: LibCategory::DataProcessing,
        prompt_hint: "`aq` (Arquero - Dataframes)",
    },
    LibraryConfig {
        require_name: "mustache",
        global_var: "Mustache",
        src: include_str!("prelude/mustache.min.js"),
        category: LibCategory::Utility,
        prompt_hint: "`Mustache` (Templating)",
    },
    // --- Visualization ---
    LibraryConfig {
        require_name: "d3",
        global_var: "d3",
        src: include_str!("prelude/d3.v7.min.js"),
        category: LibCategory::Visualization,
        prompt_hint: "`d3` (D3.js). *Prefer SVG `svg.node()` OR call `save_svg(html)`*",
    },
    LibraryConfig {
        require_name: "UPNG",
        global_var: "UPNG",
        src: include_str!("prelude/UPNG.min.js"),
        category: LibCategory::DataProcessing,
        prompt_hint: "`UPNG` (PNG encoder/decoder) for fast pixel manipulation",
    },
    //    LibraryConfig {
    //        require_name: "plot",
    //        global_var: "Plot",
    //        src: include_str!("prelude/plot.umd.min.js"),
    //        category: LibCategory::Visualization,
    //        prompt_hint: "`Plot` (Observable Plot). *High-level chart API*",
    //    },
];

impl JsInterpreter {
    pub fn new(db: Arc<dyn BlobStorage>) -> Self {
        Self { db }
    }
}

fn generate_capabilities_prompt() -> String {
    // 定义我们想要在 Prompt 中展示的顺序
    let display_order = [
        LibCategory::DataProcessing,
        LibCategory::Visualization,
        LibCategory::Environment,
        LibCategory::Utility,
    ];

    let mut lines = Vec::new();

    for cat in display_order {
        // 筛选出当前分类的所有库
        let items: Vec<String> = LOAD_SOURCE
            .iter()
            .filter(|lib| lib.category == cat)
            // 过滤掉那些我们不想在 prompt 里强调的库（如果 prompt_hint 为空）
            .filter(|lib| !lib.prompt_hint.is_empty())
            .map(|lib| lib.prompt_hint.to_string())
            .collect();

        if !items.is_empty() {
            lines.push(format!("- **{}:** {}.", cat, items.join(", ")));
        }
    }

    lines.join("\n")
}

fn get_env_script() -> &'static str {
    r#"
    // --- Console (最优先，防止库加载时打印报错看不到) ---
    globalThis.console = {
        log: (...args) => {
            let msg = args.map(String).join(" ");
            Deno.core.ops.console_op_print(msg + "\n", false);
        },
        error: (...args) => {
            let msg = args.map(String).join(" ");
            Deno.core.ops.console_op_print("stderr: " + msg + "\n", true);
        },
        // Table 稍微复杂点，也可以放后面，或者这里给个简单的
        table: (data) => {
                Deno.core.ops.console_op_print((Array.isArray(data) ? JSON.stringify(data) : String(data)) + "\n", false);
        }
    };
    if (typeof setTimeout === "undefined") {
                globalThis.setTimeout = (cb, delay, ...args) => {
                    queueMicrotask(() => {
                        if (typeof cb === 'string') {
                            (0, eval)(cb);
                        } else {
                            cb(...args);
                        }
                    });
                    return 1;
                };
                globalThis.clearTimeout = (_id) => {};
            }

            if (typeof setInterval === "undefined") {
                globalThis.setInterval = (cb, delay, ...args) => {
                    // 警告：为了防止 D3 timer 陷入死循环，这里 Mock 为只运行一次
                    queueMicrotask(() => cb(...args));
                    return 1;
                };
                globalThis.clearInterval = (_id) => {};
            }
    // --- TextEncoder / TextDecoder (UTF-8) ---
    if (typeof TextEncoder === "undefined") {
        globalThis.TextEncoder = class TextEncoder {
            encode(str) {
                const arr = [];
                for (let i = 0; i < str.length; i++) {
                    let code = str.charCodeAt(i);
                    if (code < 0x80) arr.push(code);
                    else if (code < 0x800) {
                        arr.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
                    } else if (code < 0xd800 || code >= 0xe000) {
                        arr.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
                    } else {
                        i++;
                        code = 0x10000 + (((code & 0x3ff) << 10) | (str.charCodeAt(i) & 0x3ff));
                        arr.push(0xf0 | (code >> 18), 0x80 | ((code >> 12) & 0x3f), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
                    }
                }
                return Uint8Array.from(arr);
            }
        };
    }
    if (typeof TextDecoder === "undefined") {
        globalThis.TextDecoder = class TextDecoder {
            decode(u8) {
                const arr = u8 instanceof Uint8Array ? u8 : new Uint8Array(u8);
                let str = "";
                const chunk = 8192;
                for (let i = 0; i < arr.length; i += chunk) {
                    str += String.fromCharCode.apply(null, arr.subarray(i, i + chunk));
                }
                return decodeURIComponent(escape(str));
            }
        };
    }

    // --- URL / URLSearchParams ---
    if (typeof URL === "undefined") {
        globalThis.URL = class URL {
            constructor(url, base) {
                this.href = url;
                this.searchParams = new URLSearchParams();
            }
        };
        globalThis.URLSearchParams = class URLSearchParams {
            constructor(init) { this.params = new Map(); }
            get(name) { return this.params.get(name); }
            set(name, val) { this.params.set(name, val); }
        };
    }

    // --- Performance ---
    if (typeof performance === "undefined") {
        globalThis.performance = { now: () => Date.now() };
    }
"#
}

fn get_setup_script() -> String {
    let memfs_polyfill = r#"
    const MEMFS_LIMIT = 50 * 1024 * 1024;
    const vfs = new Map();
    let currentSize = 0;

    function normalizePath(p) {
        return p.replace(/^[\.\/]+/, '');
    }

    const MemFS = {
        writeFileSync: (path, data, options) => {
            const key = normalizePath(path);
            let content;

            // 统一转为 Uint8Array
            if (typeof data === 'string') {
                content = new TextEncoder().encode(data);
            } else if (data instanceof Uint8Array) {
                content = data;
            } else {
                // 尝试处理 buffer-like
                content = new Uint8Array(data);
            }

            // 检查大小限制
            const newSize = content.length;
            const oldSize = vfs.has(key) ? vfs.get(key).length : 0;

            if (currentSize - oldSize + newSize > MEMFS_LIMIT) {
                throw new Error(`❌ MemFS Limit Exceeded: Cannot write file '${path}'. Storage full.`);
            }

            vfs.set(key, content);
            currentSize = currentSize - oldSize + newSize;
            return undefined;
        },

        readFileSync: (path, options) => {
            const key = normalizePath(path);

            if (vfs.has(key)) {
                const data = vfs.get(key);
                // 处理编码参数 (简单支持 utf8)
                if (options === 'utf8' || (typeof options === 'object' && options.encoding === 'utf8')) {
                    return new TextDecoder().decode(data);
                }
                return data;
            }

            try {
                const bytes = globalThis.retrieve_image(key);
                if (options === 'utf8' || (typeof options === 'object' && options.encoding === 'utf8')) {
                    return new TextDecoder().decode(bytes);
                }
                return bytes;
            } catch (e) {
                throw new Error(`ENOENT: no such file or directory, open '${path}'. \n(Also failed to retrieve as Image UUID from DB)`);
            }
        },

        existsSync: (path) => {
            const key = normalizePath(path);
            // 注意：这里没法 check 数据库，只能 check 内存
            return vfs.has(key) || globalThis.contain_image(key);
        },
        mkdirSync: () => {},
        statSync: (path) => {
            const key = normalizePath(path);
            if (!vfs.has(key)) throw new Error(`ENOENT: '${path}'`);
            return { isFile: () => true, isDirectory: () => false, size: vfs.get(key).length };
        },
        unlinkSync: (path) => {
                const key = normalizePath(path);
                if (vfs.has(key)) {
                    currentSize -= vfs.get(key).length;
                    vfs.delete(key);
                }
        },
        readdirSync: () => Array.from(vfs.keys()),

        // Promise 版本
        promises: {
            readFile: async (...args) => MemFS.readFileSync(...args),
            writeFile: async (...args) => MemFS.writeFileSync(...args),
            unlink: async (...args) => MemFS.unlinkSync(...args),
            mkdir: async () => {},
            stat: async (...args) => MemFS.statSync(...args),
            access: async (path) => { if (!MemFS.existsSync(path)) throw new Error('ENOENT'); }
        }
    };
    globalThis.fs = MemFS;
"#;
    let require_cases = LOAD_SOURCE
        .iter()
        .map(|lib| {
            format!(
                "case '{}': return globalThis['{}'];",
                lib.require_name, lib.global_var
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let available_libs = LOAD_SOURCE
        .iter()
        .map(|lib| lib.require_name)
        .collect::<Vec<_>>()
        .join(", ");
    r#"
        {memfs_polyfill}
        globalThis.require = function(name) {
            switch(name) {
                {require_cases}
                case 'fs': return MemFS;
                case 'fs/promises': return MemFS.promises;
                case 'path': return {
                    resolve: (...args) => args.join('/').replace(/\/+/g, '/'),
                    join: (...args) => args.join('/').replace(/\/+/g, '/'),
                    basename: (p) => p.split('/').pop(),
                    extname: (p) => {{ const i = p.lastIndexOf('.'); return i < 0 ? '' : p.slice(i); }}
                };

                case 'os': return {
                    platform: () => 'browser',
                    arch: () => 'x64',
                    tmpdir: () => '/tmp'
                };
                default: throw new Error(`❌ Module '${name}' not found. Available: {available_libs}`);
            }
        };
        if (globalThis.LinkeDOM) {
            const { parseHTML, XMLSerializer} = globalThis.LinkeDOM;
            const dom = parseHTML('<!doctype html><html><body></body></html>');
            globalThis.window = dom.window;
            globalThis.document = dom.document;
            globalThis.Element = dom.HTMLElement;
            globalThis.SVGElement = dom.SVGElement;
            globalThis.Node = dom.Node;
            if (XMLSerializer) {
                globalThis.XMLSerializer = XMLSerializer;
            } else if (dom.window && dom.window.XMLSerializer) {
                globalThis.XMLSerializer = dom.window.XMLSerializer;
            } else {
                Deno.core.ops.console_op_print("Notice: Using simple XMLSerializer polyfill.\n", true);
                globalThis.XMLSerializer = class {
                    serializeToString(node) {
                        return node.outerHTML || "";
                    }
                };
            }

            globalThis.requestAnimationFrame = (callback) => {
                return setTimeout(callback, 0);
            };
            globalThis.cancelAnimationFrame = (id) => {
                clearTimeout(id);
            };
            const originalSetAttribute = globalThis.Element.prototype.setAttribute;
            globalThis.Element.prototype.setAttribute = function(name, value) {
                originalSetAttribute.call(this, name, value);
                return this;
            };
            if (globalThis.SVGElement) {
                    globalThis.SVGElement.prototype.setAttribute = globalThis.Element.prototype.setAttribute;
            }
        } else {
            Deno.core.ops.console_op_print("stderr: LinkeDOM not loaded!\n", true);
        }

        function op_anybuffer_to_uint8array(data) {
            let buffer;
            if (data instanceof ArrayBuffer) {
                buffer = new Uint8Array(data);
            } else if (typeof data === 'string') {
                const binString = atob(data);
                buffer = new Uint8Array(binString.length);
                for (let i = 0; i < binString.length; i++) {
                    buffer[i] = binString.charCodeAt(i);
                }
            } else {
                buffer = data;
            }
            return buffer;
        }

        globalThis.btoa = Base64.encode;
        globalThis.atob = Base64.decode;

        globalThis.retrieve_image = (uuid) => Deno.core.ops.op_retrieve_image(uuid);
        globalThis.save_image = (img) => {
            const img_bin = op_anybuffer_to_uint8array(img);
            return Deno.core.ops.op_save_image(img_bin);
        };
        globalThis.save_svg = (svg) => Deno.core.ops.op_save_svg(svg);
        globalThis.contain_image = (uuid) => Deno.core.ops.op_contain_image(uuid);
        globalThis.convert_to_png = (img) => {
            const img_bin = op_anybuffer_to_uint8array(img);
            return Deno.core.ops.op_convert_to_png(img_bin);
        };
    "#.replace("{require_cases}", &require_cases)
    .replace("{available_libs}", &available_libs)
    .replace("{memfs_polyfill}", &memfs_polyfill)
}

fn run_code(db: Arc<dyn BlobStorage>, code: String) -> Result<CodeResult, Error> {
    let code = format!(
        r#"(async () => {{
            try {{
                globalThis.__internal_output = await (async () => {{
                    "use strict";
                    {}
                }})();
            }} catch (error) {{
                globalThis.__internal_output = error;
            }}
        }})()"#,
        code
    );
    let (tx, rx) = mpsc::channel::<String>();
    let (tx_img, rx_img) = mpsc::channel::<Uuid>();

    let mut js_runtime = JsRuntime::new(RuntimeOptions {
        extensions: vec![sandbox_ext::init()],
        ..Default::default()
    });

    {
        let state = js_runtime.op_state();
        let mut state = state.borrow_mut();
        state.put(LogSender(tx.clone()));
        state.put(ImageSender(tx_img.clone()));
        state.put(DbHandle(db));
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let res = rt.block_on(async {
        let setup_script = get_env_script();
        js_runtime.execute_script("<env>", setup_script)?;

        if !LOAD_SOURCE.is_empty() {
            for lib in LOAD_SOURCE {
                js_runtime.execute_script(lib.require_name, lib.src)?;
            }
        }
        let setup_script = get_setup_script();
        js_runtime.execute_script("<setup>", setup_script)?;

        let _ = js_runtime.execute_script("<user_code>", code)?;

        let _ = js_runtime.run_event_loop(Default::default()).await?;

        let result_str: String = {
            scope!(scope, js_runtime);
            let context = scope.get_current_context();
            let global = context.global(scope);
            let output_key = v8::String::new(scope, "__internal_output").unwrap();
            let output_val = global.get(scope, output_key.into()).unwrap();

            if output_val.is_native_error() {
                let e = v8::Local::<v8::Value>::try_from(output_val)
                    .map_err(|_| anyhow!("Failed to cast error object"))?;
                let js_error = deno_core::error::JsError::from_v8_exception(scope, e);
                Err(anyhow!("Runtime Error: {}", js_error.to_string()))
            } else if output_val.is_undefined() {
                Ok("undefined".to_string())
            } else {
                let serialized =
                    deno_core::serde_v8::from_v8::<serde_json::Value>(scope, output_val)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|_| output_val.to_rust_string_lossy(scope));
                Ok(serialized)
            }
        }?;

        Ok::<String, Error>(result_str)
    })?;

    drop(js_runtime);
    drop(tx);
    drop(tx_img);

    let logs: String = rx.into_iter().collect();
    let uuids: Vec<Uuid> = rx_img.into_iter().collect();
    Ok(CodeResult {
        return_value: res,
        terminal: logs,
        uuids: uuids,
    })
}
