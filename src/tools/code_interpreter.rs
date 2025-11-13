use std::{
    str::FromStr,
    sync::{Arc, Mutex},
};

use anyhow::{Error, anyhow};
use rustpython_vm::{PyResult, Settings, VirtualMachine, compiler::Mode, scope::Scope, vm};
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{MessageContent, Tool, ToolDescription};

#[derive(Deserialize, JsonSchema)]
pub struct CodeInterpreterArgs {
    #[schemars(description = r##"The python code"##)]
    code: String,
}

pub struct CodeInterpreter {
    db: sled::Tree,
}

impl Tool for CodeInterpreter {
    fn name(&self) -> String {
        "python_interpreter".to_string()
    }

    fn description(&self) -> ToolDescription {
        ToolDescription {
            name_for_model: "python_interpreter".to_string(),
            name_for_human: "Python代码执行工具".to_string(),
            description_for_model:
r##"Python code sandbox, which can be used to execute Python code.
Last expression, stdout and stderr will be returned.
A special function `retrieve_image(string)` is available to get an image by uuid and return its binary.
"##.to_string(),
            parameters: serde_json::to_value(schema_for!(CodeInterpreterArgs)).unwrap(),
            args_format: "输入格式必须是有效的JSON，其中code储存原始python代码。".to_string(),
        }
    }
    fn call(&self, args: &str) -> Result<MessageContent, anyhow::Error> {
        let args: CodeInterpreterArgs = serde_json::from_str(args)?;
        let ret = self.run_code(&args.code)?;
        Ok(MessageContent::Text(serde_json::to_string(&ret)?))
    }
}

#[derive(Deserialize, Serialize)]
struct CodeResult {
    last_expression: String,
    stdout: String,
    stderr: String,
}

const SETUP_CODE: &str = r#"
import sys

class RustOutput:
    def __init__(self, is_stderr=False):
        self.is_stderr = is_stderr

    def write(self, s):
        if self.is_stderr:
            _rust_stderr_write(s)
        else:
            _rust_stdout_write(s)

    def flush(self):
        pass

sys.stdout = RustOutput(is_stderr=False)
sys.stderr = RustOutput(is_stderr=True)
"#;

impl CodeInterpreter {
    pub fn new(db: sled::Tree) -> Self {
        Self { db }
    }

    fn run_code(&self, code: &str) -> Result<CodeResult, Error> {
        let mut setting = Settings::default();
        setting.isolated = true;
        setting.allow_external_library = false;

        let stdout_buffer = Arc::new(Mutex::new(String::new()));
        let stderr_buffer = Arc::new(Mutex::new(String::new()));

        let vm = vm::Interpreter::with_init(setting, move |vm| {
            vm.add_native_modules(rustpython_vm::stdlib::get_module_inits());
        });

        let db = self.db.clone();
        let retrieve_image = move |s: String, vm: &VirtualMachine| -> PyResult<Vec<u8>> {
            match db.get(Uuid::from_str(&s).map_err(|e| {
                vm.new_system_error(format!("Unable to convert parameter to uuid: {}", e))
            })?) {
                Ok(Some(s)) => Ok(s.to_vec()),
                Ok(None) => Err(vm.new_system_error(format!("image {} retrieved but is empty", s))),
                Err(e) => {
                    Err(vm.new_system_error(format!("Unable to get image with uuid {}: {}", s, e)))
                }
            }
        };

        let stdout_buffer_clone = stdout_buffer.clone();
        let write_stdout = move |s: String| {
            let mut buf = stdout_buffer_clone.lock().unwrap();
            buf.push_str(&s);
        };

        let stderr_buffer_clone = stderr_buffer.clone();
        let write_stderr = move |s: String| {
            let mut buf = stderr_buffer_clone.lock().unwrap();
            buf.push_str(&s);
        };
        vm.enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            scope
                .globals
                .set_item(
                    "retrieve_image",
                    vm.new_function("retrieve_image", retrieve_image).into(),
                    &vm,
                )
                .map_err(|_| anyhow!("Unable to set global function `retrieve_image`"))?;
            scope
                .globals
                .set_item(
                    "_rust_stdout_write",
                    vm.new_function("_rust_stdout_write", write_stdout).into(),
                    &vm,
                )
                .map_err(|_| anyhow!("Unable to set internal function `_rust_stdout_write`"))?;

            scope
                .globals
                .set_item(
                    "_rust_stderr_write",
                    vm.new_function("_rust_stderr_write", write_stderr).into(),
                    &vm,
                )
                .map_err(|_| anyhow!("Unable to set internal function `_rust_stderr_write`"))?;
            vm.run_code_string(scope.clone(), SETUP_CODE, "".to_string())
                .map_err(|_| anyhow!("Unable to setup stdout/stderr mapping."))?;
            let last_expression = vm
                .compile(code, Mode::Single, "".to_string())
                .map_err(|e| vm.new_syntax_error(&e, Some(code)))
                .and_then(|c| vm.run_code_obj(c, scope))
                .and_then(|res| res.str(vm).map(|s| s.to_string()))
                .map_err(|e| {
                    let mut s = String::new();
                    vm.write_exception(&mut s, &e)
                        .map(|_| anyhow!(s))
                        .unwrap_or(anyhow!("Unable to write exception to string"))
                })?;

            let stdout = stdout_buffer.lock().unwrap().clone();
            let stderr = stderr_buffer.lock().unwrap().clone();
            Ok(CodeResult {
                last_expression,
                stdout,
                stderr,
            })
        })
    }
}
