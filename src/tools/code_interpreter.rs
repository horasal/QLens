use std::str::FromStr;

use anyhow::{Error, anyhow};
use rustpython_vm::{PyResult, Settings, VirtualMachine, compiler::Mode, scope::Scope, vm};
use schemars::{JsonSchema, schema_for};
use serde::Deserialize;
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
Result will be the value of last expression and all other outputs will be dropped.
A special function `retrieve_image(string)` is available to get an image by uuid and return its binary.
"##.to_string(),
            parameters: serde_json::to_value(schema_for!(CodeInterpreterArgs)).unwrap(),
            args_format: "输入格式必须是有效的Python代码。".to_string(),
        }
    }
    fn call(&self, args: &str) -> Result<MessageContent, anyhow::Error> {
        let args: CodeInterpreterArgs = serde_json::from_str(args)?;
        let ret = self.run_code(&args.code)?;
        Ok(MessageContent::Text(ret))
    }
}

impl CodeInterpreter {
    pub fn new(db: sled::Tree) -> Self {
        Self { db }
    }

    fn run_code(&self, code: &str) -> Result<String, Error> {
        let mut setting = Settings::default();
        setting.isolated = true;
        setting.allow_external_library = false;

        let vm = vm::Interpreter::with_init(setting, move |_| {});

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

        vm.enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            scope
                .globals
                .set_item(
                    "retrieve_image",
                    vm.new_function("retrieve_image", retrieve_image).into(),
                    &vm,
                )
                .map_err(|e| anyhow!("Unable to set global functio retrieve_image"))?;
            vm.compile(code, Mode::Single, "".to_string())
                .map_err(|e| vm.new_syntax_error(&e, Some(code)))
                .and_then(|c| vm.run_code_obj(c, scope))
                .and_then(|res| res.str(vm).map(|s| s.to_string()))
                .map_err(|e| {
                    let mut s = String::new();
                    vm.write_exception(&mut s, &e)
                        .map(|_| anyhow!(s))
                        .unwrap_or(anyhow!("Unable to write exception to string"))
                })
        })
    }
}
