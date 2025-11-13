use crate::{MessageContent, Tool, ToolDescription};
use base64::{Engine, prelude::BASE64_STANDARD};
use deno_error::JsError;
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};

use std::borrow::Cow;
use std::{sync::mpsc};
use std::thread;

use anyhow::{Error, anyhow};
use deno_core::{Extension, JsRuntime, Op, OpState, RuntimeOptions, extension, op2, scope, v8};

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
r##"Javascript code sandbox, which can be used to execute Javascript code.
The environment is pure V8 with Standard Built-in Objects(Math, JSON, etc.); not Node.js or Browser.
Last expression, stdout and stderr will be returned.
Pre-loaded Global Libraries (all libraries are already imported, any call ot 'require' will cause error):
* lodash.min.js (Mustache): Utility library.
* decimal.min.js (Decimal): arbitrary-precision Decimal
* math.min.js (math): Advanced math.
* papaparse.min.js (Papa): CSV parser/generator.
* dayjs.min.js (dayjs): Date manipulation.
* A special function `retrieve_image(string)`: get an image by its uuid and return base64-encoded binary.
"##.to_string(),
            parameters: serde_json::to_value(schema_for!(JsInterpreterArgs)).unwrap(),
            args_format: "输入格式必须是有效的JSON，其中code储Javascript代码。".to_string(),
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

extension!(sandbox_ext, ops = [console_op_print, op_retrieve_image],);

pub struct JsInterpreter {
    db: sled::Tree,
}
const LOAD_SOURCE: &[(&str, &str)] = &[
    ("lodash", include_str!("prelude/lodash.min.js")),
    ("math", include_str!("prelude/math.min.js")),
    ("decimal", include_str!("prelude/decimal.min.js")),
    ("mustache", include_str!("prelude/mustache.min.js")),
    ("papaparse", include_str!("prelude/papaparse.min.js")),
    ("dayjs", include_str!("prelude/dayjs.min.js")),
];

impl JsInterpreter {
    pub fn new(db: sled::Tree) -> Self {
        Self { db }
    }

    fn run_code(&self, code: &str) -> Result<CodeResult, Error> {
        let db = self.db.clone();
        let code = code.to_string();
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

                globalThis.retrieve_image = (uuid) => {
                    return Deno.core.ops.op_retrieve_image(uuid);
                };
            "#;

                if !LOAD_SOURCE.is_empty() {
                    for (name, source) in LOAD_SOURCE {
                        js_runtime.execute_script(*name, *source)?;
                    }
                }

                js_runtime.execute_script("<setup>", setup_script)?;
                let result = js_runtime.execute_script("<user_code>", code);

                if let Ok(_) = result {
                    let _ = js_runtime.run_event_loop(Default::default()).await;
                }

                match result {
                    Ok(global) => {
                        scope!(scope, js_runtime);
                        let value = v8::Local::new(scope, global);
                        Ok(value.to_rust_string_lossy(scope))
                    }
                    Err(e) => Err(anyhow!("Runtime Error: {}", e)),
                }
            });

            drop(tx);

            let logs: String = rx.into_iter().collect();
            let mut ret = String::new();
            match execution_result {
                Ok(res) => {
                    if !res.is_empty() && res != "undefined" {
                        ret.push_str(&res);
                    }
                }
                Err(e) => {
                    ret.push_str(&e.to_string());
                }
            }

            Ok(CodeResult {
                last_expression: ret,
                terminal: logs,
            })
        });

        // 等待线程结束
        match thread_handle?.join() {
            Ok(result) => result,
            Err(_) => Err(anyhow!("JsInterpreter thread panicked")),
        }
    }
}

#[test]
fn test_js_run() {
    let db = sled::Config::new()
        .temporary(true)
        .open().unwrap();
    let ci = JsInterpreter::new(db.open_tree("a").unwrap());
    let output = ci.run_code("console.log(\"test\");").unwrap();
    println!("{:?}", output);
}
