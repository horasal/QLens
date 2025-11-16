use crate::{MessageContent, Tool, ToolDescription, tools::FONT_DATA};
use base64::{Engine, prelude::BASE64_STANDARD};
use deno_error::JsError;
use resvg::{tiny_skia, usvg};
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};

use std::sync::{Arc, mpsc};
use std::thread;

use anyhow::{Error, anyhow};
use deno_core::{JsRuntime, OpState, RuntimeOptions, extension, op2, scope, v8};

#[derive(Deserialize, JsonSchema)]
pub struct JsInterpreterArgs {
    #[schemars(description = r##"Javascript code"##)]
    code: String,
}

impl Tool for JsInterpreter {
    fn name(&self) -> String {
        "js_interpreter".to_string()
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name_for_model: "js_interpreter".to_string(),
            name_for_human: "Javascript代码执行工具".to_string(),
            description_for_model:
r##"Javascript code interpreter.
**Environment:**
A V8-based JavaScript sandbox with a **simulated Browser DOM (LinkeDOM)**.
* **DOM Supported:** `window`, `document`, `HTMLElement`, `SVGElement`, and `XMLSerializer` are available. You can create and manipulate DOM elements just like in a browser.
* **NO Network:** **Network are DISABLED and REMOVED in JS sandbox**.
* **Syntax:** Supports ES6+ syntax. **Top-level `await` is allowed**.
* **Return Value** `return val;` `stdout` `stderr` will be returned as tool results.
* **No Layout Engine:** Note that while DOM is supported, layout calculation (e.g., `getBoundingClientRect`, `getComputedStyle`) is mocked and may not return accurate pixel values.
**Pre-loaded Libraries(Do not import/require):**
* lodash.min.js (Mustache): Utility library.
* math.min.js (math): Advanced math.
* papaparse.min.js (Papa): CSV parser/generator.
* dayjs.min.js (dayjs): Date manipulation.
* js-base64.min.js (Base64): bas64 encode/decode, alway use this to handle base64.
* d3.v7.min.js (d3): data visualizing, ALWAYS prefer **SVG** over Canvas and Use D3 selection API exclusively for DOM manipulation.
* Special functions:
  * `function save_svg(svg: string): string`: save a svg image as png to database and get its uuid.
  * `function retrieve_image(uuid: string): string`: get an image from database by its uuid and return base64-encoded data.
  * `function save_image(base64_encoded_image_binary: string): string`: save a non-svg image to database and get its uuid.
"##.to_string(),
            parameters: serde_json::to_value(schema_for!(JsInterpreterArgs)).unwrap(),
            args_format: "输入格式必须是有效的JSON，其中code储存Javascript代码。".to_string(),
        }
    }
    fn call(&self, args: &str) -> Result<MessageContent, anyhow::Error> {
        let args: JsInterpreterArgs = serde_json::from_str(args)?;
        let ret = self.run_code(&args.code)?;
        Ok(MessageContent::Text(serde_json::to_string(&ret)?))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CodeResult {
    last_expression: String,
    terminal: String,
}

struct LogSender(mpsc::Sender<String>);
struct DbHandle(sled::Tree);

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
    #[error("Database error {0}")]
    DatabaseError(#[from] sled::Error),
    #[error("Limit reached, can not save image any more")]
    MaxTries(usize),
    #[error("Invalid base64 {0}")]
    InvalidBase64(#[from] base64::DecodeError),
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

    let usvg_options = usvg::Options {
        fontdb: Arc::new(font_db),
        font_family: "MapleMono-NF-CN-Regular".into(),
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

    resvg::render(
        &tree,
        tiny_skia::Transform::default(),
        &mut pixmap.as_mut(),
    );

    let output_buf = pixmap.encode_png()
        .map_err(|_| ImageError::InternalErrorConvertPixMapToPng)?;

    let uuid = uuid::Uuid::new_v4();
    let db = state.borrow::<DbHandle>();
    match db.0.compare_and_swap(uuid, None as Option<&[u8]>, Some(output_buf)) {
        Ok(Ok(_)) => Ok(uuid.to_string()),
        Ok(Err(_)) => Err(ImageError::UuidCollision),
        Err(e) => Err(ImageError::DatabaseError(e)),
    }
}

#[op2]
#[string]
fn op_save_image(state: &mut OpState, #[string] img_base64: String) -> Result<String, ImageError> {
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
    match BASE64_STANDARD.decode(img_base64) {
        Ok(b) => {
            let uuid = uuid::Uuid::new_v4();
            match db.0.compare_and_swap(uuid, None as Option<&[u8]>, Some(b)) {
                Ok(Ok(_)) => Ok(uuid.to_string()),
                Ok(Err(_)) => Err(ImageError::UuidCollision),
                Err(e) => Err(ImageError::DatabaseError(e)),
            }
        }
        Err(e) => Err(ImageError::InvalidBase64(e)),
    }
}

#[op2]
#[string]
fn op_retrieve_image(
    state: &mut OpState,
    #[string] uuid_str: String,
) -> Result<String, ImageError> {
    let db = state.borrow::<DbHandle>();
    let uuid = uuid::Uuid::parse_str(&uuid_str).map_err(|e| ImageError::InvalidUuid(e))?;
    match db.0.get(uuid) {
        Ok(Some(bytes)) => Ok(BASE64_STANDARD.encode(bytes)),
        Ok(None) => Err(ImageError::ImageEmpty),
        Err(e) => Err(ImageError::DatabaseError(e)),
    }
}

extension!(
    sandbox_ext,
    ops = [console_op_print, op_retrieve_image, op_save_image, op_save_svg],
);

pub struct JsInterpreter {
    db: sled::Tree,
}
const LOAD_SOURCE: &[(&str, &str)] = &[
    ("linkedom", include_str!("prelude/linkedom.bundle.js")),
    ("lodash", include_str!("prelude/lodash.min.js")),
    ("math", include_str!("prelude/math.min.js")),
    ("mustache", include_str!("prelude/mustache.min.js")),
    ("papaparse", include_str!("prelude/papaparse.min.js")),
    ("dayjs", include_str!("prelude/dayjs.min.js")),
    ("d3", include_str!("prelude/d3.v7.min.js")),
    ("base64", include_str!("prelude/base64.min.js")),
];

impl JsInterpreter {
    pub fn new(db: sled::Tree) -> Self {
        Self { db }
    }

    fn run_code(&self, code: &str) -> Result<CodeResult, Error> {
        let db = self.db.clone();
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
        let builder = thread::Builder::new().stack_size(16 * 1024 * 1024);
        let thread_handle = builder.spawn(move || {
            let (tx, rx) = mpsc::channel::<String>();

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| anyhow!("Failed to build runtime: {}", e))?;

            let execution_result: Result<String, anyhow::Error> = rt.block_on(async {
                let mut js_runtime = JsRuntime::new(RuntimeOptions {
                    extensions: vec![sandbox_ext::init()],
                    ..Default::default()
                });

                {
                    let state = js_runtime.op_state();
                    let mut state = state.borrow_mut();
                    state.put(LogSender(tx.clone()));
                    state.put(DbHandle(db));
                }
                let setup_script = r#"
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
                            Deno.core.ops.console_op_print("stderr: Warning: Using simple XMLSerializer polyfill.\n", true);
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

                    globalThis.btoa = Base64.encode;
                    globalThis.atob = Base64.decode;
                    globalThis.console = {
                        log: (...args) => {
                            let msg = args.map(String).join(" ");
                            Deno.core.ops.console_op_print(msg + "\n", false);
                        },
                        error: (...args) => {
                            let msg = args.map(String).join(" ");
                            Deno.core.ops.console_op_print("stderr: " + msg + "\n", true);
                        }
                    };

                    globalThis.retrieve_image = (uuid) => Deno.core.ops.op_retrieve_image(uuid);
                    globalThis.save_image = (img_base64) => Deno.core.ops.op_save_image(img_base64);
                    globalThis.save_svg = (svg) => Deno.core.ops.op_save_svg(svg);
                "#;

                if !LOAD_SOURCE.is_empty() {
                    for (name, source) in LOAD_SOURCE {
                        js_runtime.execute_script(*name, *source)?;
                    }
                }
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

                Ok(result_str)
            });

            drop(tx);

            let logs: String = rx.into_iter().collect();
            match execution_result {
                Ok(res) => {
                    Ok(CodeResult {
                        last_expression: res,
                        terminal: logs,
                    })
                }
                Err(e) => {
                    Ok(CodeResult {
                        last_expression: e.to_string(),
                        terminal: logs,
                    })
                }
            }

        });

        match thread_handle?.join() {
            Ok(result) => result,
            Err(_) => Err(anyhow!("JsInterpreter thread panicked")),
        }
    }
}

#[test]
fn test_js_run() {
    let db = sled::Config::new().temporary(true).open().unwrap();
    let ci = JsInterpreter::new(db.open_tree("a").unwrap());
    let output = ci
        .run_code("const c = async () => { console.log(\"test\"); }; await c();")
        .unwrap();
    println!("{:?}", output);

    let output = ci.run_code(
r##"
const width = 500;
const height = 500;
const svg = d3.create("svg")
    .attr("width", width)
    .attr("height", height)
    .attr("xmlns", "http://www.w3.org/2000/svg");

svg.append("rect")
    .attr("x", 0)
    .attr("y", 0)
    .attr("width", width)
    .attr("height", height)
    .attr("fill", "#f0f0f0");

svg.append("circle")
    .attr("cx", 250)
    .attr("cy", 250)
    .attr("r", 100)
    .attr("fill", "red");

let svgString = svg.node().outerHTML;
if (!svgString) {
    const serializer = new XMLSerializer();
    svgString = serializer.serializeToString(svg.node());
}
const base64Svg = btoa(svgString);
const uuid = save_image(base64Svg);
console.log(uuid);
"##);
    println!("{:?}", output);
}
