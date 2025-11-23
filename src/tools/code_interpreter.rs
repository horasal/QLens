use crate::blob::{BlobStorage, BlobStorageError};
use crate::{FN_RAWHTML, FN_RAWSVG, parse_sourcecode_args};
use crate::{MessageContent, Tool, ToolDescription, tools::FONT_DATA};
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use deno_error::JsError;
use image::Luma;
use qrcode::QrCode;
use resvg::{tiny_skia, usvg};
use rqrr::PreparedImage;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::sync::{Arc, mpsc};
use tokio::time::Instant;
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
        let libs = generate_libs_list();
        let cheatsheet = generate_cheatsheet_prompt();
        let description = format!(
            r##"V8 sandbox environments.
        **Libs:** {libs}
        **API:**
        - `save_svg(str):uuid` / `save_blob('asset'|'image', bytes):uuid`
        - `load_blob('asset'|'image', uuid):bytes`
        - `convert_to_png(bytes):bytes`
        - `QRCode.save(str, 'png'|'svg'):uuid` / `QRCode.decode(bytes|uuid):str`
        **Notes:** NO Network. NO Canvas (Use d3/UPNG). Top-level await OK.
        **Cheatsheet:** {cheatsheet}"##
        );

        ToolDescription {
            name_for_model: "js_interpreter".to_string(),
            name_for_human: "Javascript代码执行工具".to_string(),
            description_for_model: description,
            parameters: raw_schema,
            args_format: "Raw JavaScript code string (NO quote/backticks). Use `return` or `console.log` to output.".to_string(),
        }
    }
    async fn call(&self, args: &str) -> Result<Vec<MessageContent>, anyhow::Error> {
        let code = parse_sourcecode_args(args)?;
        let image = self.image.clone();
        let asset = self.asset.clone();
        let result = tokio::task::spawn_blocking(move || run_code(image, asset, code)).await??;

        let mut v = vec![MessageContent::Text(
            result.terminal + "\nReturn: " + &result.return_value,
        )];
        for (idx, &uuid) in result.uuids_img.iter().enumerate() {
            v.push(MessageContent::ImageRef(
                uuid,
                format!("JS Generated Image#{}", idx),
            ));
        }
        for (idx, &uuid) in result.uuids_asset.iter().enumerate() {
            v.push(MessageContent::AssetRef(
                uuid,
                format!("JS Generated Asset#{}", idx),
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
    uuids_img: Vec<Uuid>,
    #[serde(skip)]
    uuids_asset: Vec<Uuid>,
}

struct LogSender(mpsc::Sender<String>);
struct UuidSender {
    image: mpsc::Sender<Uuid>,
    asset: mpsc::Sender<Uuid>,
}
struct DbHandle {
    image: Arc<dyn BlobStorage>,
    asset: Arc<dyn BlobStorage>,
}
struct TimeOrigin(Instant);

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
    #[error("Blob does not exist")]
    ImageEmpty,
    #[error("Invalid UUID {0}")]
    InvalidUuid(#[from] uuid::Error),
    #[error("Database save error {0}")]
    DatabaseError(#[from] BlobStorageError),
    #[error("Limit reached, can not save image any more")]
    MaxTries(usize),
    #[error("Invalid base64 {0}")]
    InvalidBase64(#[from] base64::DecodeError),
    #[error("Invalid schema {0}, expect image or asset")]
    InvalidSchema(String),
    #[error("Invalid SVG data {0}")]
    InvalidSVG(#[from] usvg::Error),
    #[error("Unable to create Pixmap with size {0}x{1}")]
    InternalErrorCreatePixMap(u32, u32),
    #[error("Unable to convert Pixmap to PNG.")]
    InternalErrorConvertPixMapToPng,
    #[error("Image IO Error, unable to save as PNG {0}.")]
    ImageIOError(#[from] image::error::ImageError),
    #[error("QRCode does not contain any valid data.")]
    NoDataInQRCode,
    #[error("QRCode decode error {0}.")]
    QRCodeDecodeError(#[from] rqrr::DeQRError),
    #[error("QRCode encode error {0}.")]
    QRCodeEncodeError(#[from] qrcode::types::QrError),
}

struct Counter {
    put_count: usize,
}

const MAX_BLOB_PUT_TRIES: usize = 20;

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
    match db.image.save(&output_buf) {
        Ok(uuid) => {
            if let Err(e) = state.borrow::<UuidSender>().image.send(uuid.clone()) {
                tracing::warn!("Error to send svg from javascript back to llm, {}.", e)
            }
            Ok(uuid.to_string())
        }
        Err(e) => Err(ImageError::DatabaseError(e)),
    }
}

enum Schema {
    Asset,
    Image,
}

impl Schema {
    fn parse(input: &str) -> Result<Self, ImageError> {
        match input.trim().to_lowercase().as_str() {
            "asset" | "binary" | "bin" => Ok(Self::Asset),
            "image" | "img" | "svg" | "png" | "jpeg" => Ok(Self::Image),
            _ => return Err(ImageError::InvalidSchema(input.to_string())),
        }
    }
}

#[op2]
#[string]
fn op_save_blob(
    state: &mut OpState,
    #[string] schema: String,
    #[buffer] img: &[u8],
) -> Result<String, ImageError> {
    let schema = Schema::parse(&schema)?;
    if let Some(mut c) = state.try_take::<Counter>() {
        if c.put_count >= MAX_BLOB_PUT_TRIES {
            return Err(ImageError::MaxTries(c.put_count));
        }
        c.put_count += 1;
        state.put(c);
    } else {
        state.put(Counter { put_count: 1 });
    }
    let db = state.borrow::<DbHandle>();
    match schema {
        Schema::Image => {
            let _ = image::guess_format(img)?;
            match db.image.save(&img) {
                Ok(uuid) => {
                    if let Err(e) = state.borrow::<UuidSender>().image.send(uuid.clone()) {
                        tracing::warn!("Error to saved image from javascript back to llm, {}.", e)
                    }
                    Ok(uuid.to_string())
                }
                Err(e) => Err(ImageError::DatabaseError(e)),
            }
        }
        Schema::Asset => match db.asset.save(&img) {
            Ok(uuid) => {
                if let Err(e) = state.borrow::<UuidSender>().asset.send(uuid.clone()) {
                    tracing::warn!("Error to saved asset from javascript back to llm, {}.", e)
                }
                Ok(uuid.to_string())
            }
            Err(e) => Err(ImageError::DatabaseError(e)),
        },
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
#[string]
fn op_qrcode_decode(#[buffer] data: &[u8]) -> Result<String, ImageError> {
    let img = image::load_from_memory(data)?;

    let gray_img = img.to_luma8();
    let mut prepared_img = PreparedImage::prepare(gray_img);
    let grids = prepared_img.detect_grids();
    if grids.is_empty() {
        return Err(ImageError::NoDataInQRCode);
    }
    let (_, content) = grids[0]
        .decode()
        .map_err(|e| ImageError::QRCodeDecodeError(e))?;

    Ok(content)
}

#[op2]
#[buffer]
fn op_load_blob(
    state: &mut OpState,
    #[string] schema: String,
    #[string] uuid_str: String,
) -> Result<Vec<u8>, ImageError> {
    let schema = Schema::parse(&schema)?;
    let uuid = uuid::Uuid::parse_str(&uuid_str).map_err(|e| ImageError::InvalidUuid(e))?;
    let db = state.borrow::<DbHandle>();
    match match schema {
        Schema::Asset => db.asset.get(uuid),
        Schema::Image => db.image.get(uuid),
    } {
        Ok(Some(bytes)) => Ok(bytes),
        Ok(None) => Err(ImageError::ImageEmpty),
        Err(e) => Err(ImageError::DatabaseError(e)),
    }
}

#[op2(fast)]
fn op_contain_blob(
    state: &mut OpState,
    #[string] schema: String,
    #[string] uuid_str: String,
) -> Result<bool, ImageError> {
    let schema = Schema::parse(&schema)?;
    let uuid = uuid::Uuid::parse_str(&uuid_str).map_err(|e| ImageError::InvalidUuid(e))?;
    let db = state.borrow::<DbHandle>();
    match match schema {
        Schema::Asset => db.asset.get(uuid),
        Schema::Image => db.image.get(uuid),
    } {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(e) => Err(ImageError::DatabaseError(e)),
    }
}

#[op2]
#[buffer]
fn op_qrcode_png(#[string] text: String) -> Result<Vec<u8>, ImageError> {
    let code = QrCode::new(text.as_bytes())?;

    let image = code.render::<Luma<u8>>().min_dimensions(200, 200).build();

    let mut bytes: Vec<u8> = Vec::new();
    let mut cursor = Cursor::new(&mut bytes);

    image::DynamicImage::ImageLuma8(image).write_to(&mut cursor, image::ImageFormat::Png)?;

    Ok(bytes)
}

#[op2]
#[string]
fn op_qrcode_svg(#[string] text: String) -> Result<String, ImageError> {
    let code = QrCode::new(text.as_bytes())?;

    let svg = code
        .render()
        .min_dimensions(200, 200)
        .dark_color(qrcode::render::svg::Color("#000000"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();

    Ok(svg)
}

#[op2]
#[buffer]
fn op_text_encode(#[string] text: String) -> Vec<u8> {
    text.into_bytes()
}

#[op2]
#[string]
fn op_text_decode(#[buffer] bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

#[op2]
#[string]
fn op_base64_encode(#[buffer] data: &[u8]) -> String {
    BASE64_STANDARD.encode(data)
}

#[op2]
#[buffer]
fn op_base64_decode(#[string] data: String) -> Result<Vec<u8>, ImageError> {
    Ok(BASE64_STANDARD.decode(data)?)
}

#[op2(fast)]
fn op_performance_now(state: &mut OpState) -> f64 {
    let origin = state.borrow::<TimeOrigin>().0;
    origin.elapsed().as_secs_f64() * 1000.0
}

extension!(
    sandbox_ext,
    ops = [
        console_op_print,
        op_load_blob,
        op_save_blob,
        op_save_svg,
        op_contain_blob,
        op_convert_to_png,
        op_text_encode,
        op_text_decode,
        op_base64_decode,
        op_base64_encode,
        op_performance_now,
        op_qrcode_png,
        op_qrcode_svg,
        op_qrcode_decode,
    ],
);

pub struct JsInterpreter {
    image: Arc<dyn BlobStorage>,
    asset: Arc<dyn BlobStorage>,
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
    require_name: &'static str,       // 用于 require('name')
    global_var: &'static str,         // 注入到 globalThis 的变量名
    src: &'static str,                // 源码内容
    category: LibCategory,            // 分类
    prompt_hint: &'static str,        // Prompt 中的展示文本 (例如: "`_` (Lodash)")
    after_hook: Option<&'static str>, // Polyfill用的配置脚本
}
const LOAD_SOURCE: &[LibraryConfig] = &[
    // --- Environment ---
    LibraryConfig {
        require_name: "linkedom",
        global_var: "LinkeDOM",
        src: include_str!("prelude/linkedom.bundle.js"),
        category: LibCategory::Environment,
        prompt_hint: "`document`, `window`; NO canvas.",
        after_hook: Some(
            r###"
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
        }"###,
        ),
    },
    // --- Data Processing ---
    LibraryConfig {
        require_name: "lodash",
        global_var: "_",
        src: include_str!("prelude/lodash.min.js"),
        category: LibCategory::DataProcessing,
        prompt_hint: r##"const users=[{n:'a',g:'tech'},{n:'b',g:'hr'},{n:'c',g:'tech'}];console.log(_.groupBy(users,'g'));"##,
        after_hook: Some(
            r#"
        if (typeof _ !== "undefined") {
            globalThis.structuredClone = _.cloneDeep;
        }"#,
        ),
    },
    LibraryConfig {
        require_name: "mathjs",
        global_var: "math",
        src: include_str!("prelude/math.min.js"),
        category: LibCategory::DataProcessing,
        prompt_hint: "math.evaluate('12.7 cm to inch').toString()",
        after_hook: None,
    },
    LibraryConfig {
        require_name: "dayjs",
        global_var: "dayjs",
        src: include_str!("prelude/dayjs.min.js"),
        category: LibCategory::DataProcessing,
        prompt_hint: "dayjs().format('YYYY-MM-DD HH:mm:ss')",
        after_hook: None,
    },
    LibraryConfig {
        require_name: "papaparse",
        global_var: "Papa",
        src: include_str!("prelude/papaparse.min.js"),
        category: LibCategory::DataProcessing,
        prompt_hint: "",
        after_hook: None,
    },
    LibraryConfig {
        require_name: "arquero",
        global_var: "aq",
        src: include_str!("prelude/arquero.min.js"),
        category: LibCategory::DataProcessing,
        prompt_hint: r##"const dt=aq.table({a:[1,2,3],b:[4,5,6]});
console.log(dt.filter(d=>d.a>1).derive({c:d=>d.a+d.b}).toCSV());"##,
        after_hook: None,
    },
    LibraryConfig {
        require_name: "mustache",
        global_var: "Mustache",
        src: include_str!("prelude/mustache.min.js"),
        category: LibCategory::Utility,
        prompt_hint: r##"const htmlStr=Mustache.render(template,{title:"A",list:["1","2"]});"##,
        after_hook: None,
    },
    LibraryConfig {
        require_name: "nerdamer",
        global_var: "nerdamer",
        src: include_str!("prelude/nerdamer.min.js"),
        category: LibCategory::DataProcessing,
        prompt_hint: "nerdamer('solve(x^2=4, x)').toString()",
        after_hook: None,
    },
    // --- Visualization ---
    LibraryConfig {
        require_name: "d3",
        global_var: "d3",
        src: include_str!("prelude/d3.v7.min.js"),
        category: LibCategory::Visualization,
        prompt_hint: "const svg=d3.create('svg').attr('width',400).attr('height',300);/*draw*/; save_svg(svg.node().outerHTML)",
        after_hook: None,
    },
    LibraryConfig {
        require_name: "vega",
        global_var: "vega",
        src: include_str!("prelude/vega.min.js"),
        category: LibCategory::Visualization,
        prompt_hint: r##"/*Spec must have width/height*/ const vegaSpec=vegaLite.compile(vlSpec).spec;
const v=new vega.View(vega.parse(vegaSpec),{renderer:'svg'}).initialize(); save_svg(await v.toSVG());"##,
        after_hook: None,
    },
    LibraryConfig {
        require_name: "vega-lite",
        global_var: "vegaLite",
        src: include_str!("prelude/vega-lite.min.js"),
        category: LibCategory::Visualization,
        prompt_hint: "",
        after_hook: None,
    },
    LibraryConfig {
        require_name: "UPNG",
        global_var: "UPNG",
        src: include_str!("prelude/UPNG.min.js"),
        category: LibCategory::DataProcessing,
        prompt_hint: r##"const b=new Uint8Array([255,0,0,255]).buffer;UPNG.encode([b],width,height,depth);"##,
        after_hook: None,
    },
];

impl JsInterpreter {
    pub fn new(image: Arc<dyn BlobStorage>, asset: Arc<dyn BlobStorage>) -> Self {
        Self { image, asset }
    }
}

const DISPLAY_ORDER: &[LibCategory] = &[
    LibCategory::DataProcessing,
    LibCategory::Visualization,
    LibCategory::Environment,
    LibCategory::Utility,
];

fn generate_libs_list() -> String {
    let mut lines = Vec::new();
    for &cat in DISPLAY_ORDER {
        let items: Vec<String> = LOAD_SOURCE
            .iter()
            .filter(|lib| lib.category == cat)
            .filter(|lib| !lib.prompt_hint.is_empty())
            .map(|lib| format!("{}({})", lib.require_name, lib.global_var))
            .collect();
        if !items.is_empty() {
            lines.push(format!("{}:{}", cat, items.join(",")));
        }
    }
    lines.join(";")
}

fn generate_cheatsheet_prompt() -> String {
    LOAD_SOURCE
        .iter()
        .filter(|lib| !lib.prompt_hint.is_empty())
        .map(|lib| format!("**{}**:{}", lib.require_name, lib.prompt_hint))
        .collect::<Vec<_>>()
        .join("\n")
}

fn get_env_script() -> &'static str {
    r#"
    globalThis.console = {
        log: (...args) => {
            let msg = args.map(String).join(" ");
            Deno.core.ops.console_op_print(msg + "\n", false);
        },
        error: (...args) => {
            let msg = args.map(String).join(" ");
            Deno.core.ops.console_op_print("stderr: " + msg + "\n", true);
        },
        warn: (...args) => {
            let msg = args.map(String).join(" ");
            Deno.core.ops.console_op_print("warn: " + msg + "\n", false);
        },
        info: (...args) => {
            let msg = args.map(String).join(" ");
            Deno.core.ops.console_op_print("info: " + msg + "\n", false);
        },
        trace: (...args) => {
            let msg = args.map(String).join(" ");
            Deno.core.ops.console_op_print("trace: " + msg + "\n", false);
        },
        table: (data) => {
            Deno.core.ops.console_op_print((Array.isArray(data) ? JSON.stringify(data) : String(data)) + "\n", false);
        }
    };
    globalThis.console.warn = globalThis.console.log;
    globalThis.console.info = globalThis.console.log;
    globalThis.console.trace = globalThis.console.log;

    if (typeof setTimeout === "undefined") {
        globalThis.setTimeout = (cb, delay, ...args) => {
            queueMicrotask(() => {
                if (typeof cb === 'string') {
                    (0, eval)(cb);
                } else {
                    cb(...args);
                }});
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
            get encoding() { return "utf-8"; }
            encode(input) {
                const str = input === undefined ? "" : String(input);
                return Deno.core.ops.op_text_encode(str);
            }
            encodeInto(source, destination) {
                const encoded = this.encode(source);
                const len = Math.min(encoded.length, destination.length);
                destination.set(encoded.subarray(0, len));
                return { read: source.length, written: len };
            }
        };
    }

    if (typeof TextDecoder === "undefined") {
        globalThis.TextDecoder = class TextDecoder {
            constructor(label = "utf-8", options = {}) {
                // 目前只支持 utf-8，忽略 label
                this.encoding = "utf-8";
                this.fatal = options.fatal || false;
                this.ignoreBOM = options.ignoreBOM || false;
            }
            decode(input, options) {
                let buffer;
                if (input === undefined) {
                    buffer = new Uint8Array(0);
                } else if (input instanceof ArrayBuffer) {
                    buffer = new Uint8Array(input);
                } else if (ArrayBuffer.isView(input)) {
                    buffer = new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
                } else {
                    throw new TypeError("Failed to execute 'decode' on 'TextDecoder': The provided value is not of type '(ArrayBuffer or ArrayBufferView)'");
                }
                return Deno.core.ops.op_text_decode(buffer);
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

    globalThis.btoa = (str) => {
        const encoder = new TextEncoder();
        return Deno.core.ops.op_base64_encode(str);
    };

    globalThis.atob = (base64) => {
        const bytes = Deno.core.ops.op_base64_decode(base64);
        return new TextDecoder().decode(bytes);
    };

    globalThis.Base64 = {
        encode: (data) => {
            const bytes = typeof data === 'string' ? new TextEncoder().encode(data) : data;
            return Deno.core.ops.op_base64_encode(bytes);
        },
        decode: (str) => Deno.core.ops.op_base64_decode(str) // 返回 Uint8Array
    };

    // --- Performance ---
    if (typeof performance === "undefined") {
        globalThis.performance = { now: () => Deno.core.ops.op_performance_now() };
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

            if (globalThis.contain_blob('asset', key)) {
                try {
                    const bytes = globalThis.load_blob('asset', key);
                    if (options === 'utf8' || (typeof options === 'object' && options.encoding === 'utf8')) {
                        return new TextDecoder().decode(bytes);
                    }
                    return bytes;
                } catch (e) {
                    throw new Error(`ENOENT: no such file or directory, open '${path}'. \n(Also failed to load blob by UUID from DB)`);
                }
            }

            if (globalThis.contain_blob('image', key)){
                try {
                    const bytes = globalThis.load_blob('image', key);
                    if (options === 'utf8' || (typeof options === 'object' && options.encoding === 'utf8')) {
                        return new TextDecoder().decode(bytes);
                    }
                    return bytes;
                } catch (e) {
                    throw new Error(`ENOENT: no such file or directory, open '${path}'. \n(Also failed to load blob by UUID from DB)`);
                }
            }
            throw new Error(`ENOENT: no such file or directory, open '${path}'. `);
        },

        existsSync: (path) => {
            const key = normalizePath(path);
            return vfs.has(key) || globalThis.contain_blob('image', key) || globalThis.contain_blob('asset', key);
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
    globalThis.process = {
        env: {},
        version: 'v16.0.0',
        versions: { node: '16.0.0' },
        platform: 'browser',
        browser: true,
        cwd: () => '/',
        stdout: { write: (msg) => console.log(msg) },
        stderr: { write: (msg) => console.error(msg) },
        nextTick: (cb, ...args) => queueMicrotask(() => cb(...args))
    };
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
    r#"if (typeof structuredClone === "undefined" && typeof _ !== "undefined") {
        globalThis.structuredClone = (value) => _.cloneDeep(value);
    } else if (typeof structuredClone === "undefined") {
        globalThis.structuredClone = (value) => JSON.parse(JSON.stringify(value));
    }
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
    function op_anybuffer_to_uint8array(data) {
        let buffer;
        if (data instanceof ArrayBuffer){
            buffer = new Uint8Array(data);
        } else if (data instanceof Array) {
            buffer = Uint8Array.from(data);
        } else if (typeof data === 'string') {
            const binString = atob(data);
            buffer = new Uint8Array(binString.length);
            for (let i = 0; i < binString.length; i++) {
                buffer[i] = binString.charCodeAt(i);
            }
        } else if (data instanceof Uint8Array) {
            buffer = data;
        }else {
            buffer = UInt8Array.from(data);
        }
        return buffer;
    }

    globalThis.html = (content) => {
        return "{FN_RAWHTML}" + content;
    };
    globalThis.svg = (content) => {
        return "{FN_RAWSVG}" + content;
    };

    globalThis.load_blob = (schema, uuid) => Deno.core.ops.op_load_blob(schema, uuid);
    globalThis.save_blob = (schema, img) => {
        const img_bin = op_anybuffer_to_uint8array(img);
        return Deno.core.ops.op_save_blob(schema, img_bin);
    };
    globalThis.save_svg = (svg) => Deno.core.ops.op_save_svg(svg);
    globalThis.contain_blob = (uuid) => Deno.core.ops.op_contain_blob(uuid);
    globalThis.convert_to_png = (img) => {
        const img_bin = op_anybuffer_to_uint8array(img);
        return Deno.core.ops.op_convert_to_png(img_bin);
    };
    globalThis.QRCode = {
        save: (text, format = 'png') => {
            const str = String(text);
            if (!str) throw new Error("QRCode: Text cannot be empty");
            switch (format.toLowerCase()) {
                case 'png': {
                    const bytes = Deno.core.ops.op_qrcode_png(str);
                    return save_blob('image', bytes);
                }
                case 'svg': {
                    const svgStr = Deno.core.ops.op_qrcode_svg(str);
                    return Deno.core.ops.op_save_svg(svgStr);
                }
                default:
                    throw new Error(`QRCode: Unsupported format '${format}'. Use 'png' or 'svg'.`);
            }
        },
        encode: (text, format = 'png') => {
            const str = String(text);
            switch (format.toLowerCase()) {
                case 'png': return Deno.core.ops.op_qrcode_png(str); // Returns Uint8Array
                case 'svg': return Deno.core.ops.op_qrcode_svg(str); // Returns String
                default: throw new Error(`QRCode: Unsupported format '${format}'`);
            }
        },
        decode: (input) => {
            let buffer;
            if (typeof input === 'string') {
                try {
                    buffer = globalThis.load_blob('image', input);
                } catch (e) {
                    throw new Error("QRCode.decode: Failed to retrieve image from UUID.");
                }
            } else if (input instanceof Uint8Array) {
                buffer = input;
            } else {
                throw new Error("QRCode.decode: Input must be Uint8Array or Image UUID string.");
            }
            return Deno.core.ops.op_qrcode_decode(buffer);
        }
    };"#
    .replace("{require_cases}", &require_cases)
    .replace("{available_libs}", &available_libs)
    .replace("{memfs_polyfill}", &memfs_polyfill)
    .replace("{RAWHTML}", FN_RAWHTML)
    .replace("{RAWSVG}", FN_RAWSVG)
}

fn run_code(
    image: Arc<dyn BlobStorage>,
    asset: Arc<dyn BlobStorage>,
    code: String,
) -> Result<CodeResult, Error> {
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
    let (tx_asset, rx_asset) = mpsc::channel::<Uuid>();

    let mut js_runtime = JsRuntime::new(RuntimeOptions {
        extensions: vec![sandbox_ext::init()],
        ..Default::default()
    });

    {
        let state = js_runtime.op_state();
        let mut state = state.borrow_mut();
        state.put(LogSender(tx.clone()));
        state.put(UuidSender {
            image: tx_img.clone(),
            asset: tx_asset.clone(),
        });
        state.put(DbHandle { image, asset });
        state.put(TimeOrigin(Instant::now()));
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
                if let Some(hook) = lib.after_hook {
                    let hook_name = format!("{}_after_hook", lib.require_name);
                    js_runtime.execute_script(hook_name, hook)?;
                }
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
    drop(tx_asset);

    let logs: String = rx.into_iter().collect();
    let uuids_img: Vec<Uuid> = rx_img.into_iter().collect();
    let uuids_asset: Vec<Uuid> = rx_asset.into_iter().collect();
    Ok(CodeResult {
        return_value: res,
        terminal: logs,
        uuids_img: uuids_img,
        uuids_asset: uuids_asset,
    })
}
