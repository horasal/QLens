use whatlang::Lang;

pub struct SystemPromptTemplates {
    /// "工具" 部分的模板
    pub tool_info_template: &'static str,
    /// "并行调用" 部分的模板
    pub parallel_call_template: &'static str,
    /// "单次/多次调用" 部分的模板
    pub single_call_template: &'static str,
}

/// 根据语言获取对应的模板
pub fn get_templates(lang: Lang) -> SystemPromptTemplates {
    match lang {
        // -----------------------------------------------------------------
        // 中文 (Chinese)
        // -----------------------------------------------------------------
        Lang::Cmn => SystemPromptTemplates {
            tool_info_template: r###" 工具
## 你拥有如下工具：
{tool_descs}"###,
            parallel_call_template:
"## 你可以在回复中插入以下命令以并行调用N个工具\
工具调用可以分成一轮或者多轮进行，\
回复结束后你会获得工具的回应：\n\
{FN_NAME}: 工具1的名称，必须是[{tool_names}]之一\n\
{FN_ARGS}: 工具1的输入\n\
{FN_NAME}: 工具2的名称\n\
{FN_ARGS}: 工具2的输入\n\
...\n\
{FN_NAME}: 工具N的名称\n\
{FN_ARGS}: 工具N的输入\n\
## 工具会返回以下结果(**你不需要在调用时输出{FN_RESULT}和{FN_EXIT}，未来你有机会组织语言**)：\n\
{FN_RESULT}: 工具1的结果\n\
{FN_RESULT}: 工具2的结果\n\
...\n\
{FN_RESULT}: 工具N的结果\n\
## 在收到工具返回后：\n\
{FN_EXIT} 检查结果是否正确，并根据结果进行进一步处理，纠正或回复，可以使用`![描述](/api/image/{{uuid}})`来引用图片\n",
            single_call_template:
"## 你可以在回复中插入零次、一次或多次以下命令以调用工具，\
如果多个工具调用之间又依存关系，你可以将调用分割成多轮，每轮工具调用的结果会保存并能够在未来使用。\
比如首先调用工具1结束对话，工具1返回之后根据工具1的结果调用工具2...\
回复结束后你会获得工具的回应：\n\
{FN_NAME}: 工具名称，必须是[{tool_names}]之一。\n\
{FN_ARGS}: 工具输入\n\n\
## 工具会返回以下结果(**你不需要在调用时输出{FN_RESULT}和{FN_EXIT}，未来你有机会组织语言**)\n\
{FN_RESULT}: 工具结果\n\
## 在收到工具返回后：\n\
{FN_EXIT} 检查结果是否正确，并根据结果进行进一步处理，纠正或回复，可以使用`![描述](/api/image/{{uuid}})`来引用图片\n",
        },

        // -----------------------------------------------------------------
        // 日语 (Japanese)
        // -----------------------------------------------------------------
        Lang::Jpn => SystemPromptTemplates {
            tool_info_template: r###" ツール
## 以下のツールが利用可能です：
{tool_descs}"###,
            parallel_call_template:
"## 応答に以下のコマンドを挿入することで、\
ツールの利用は複数ターンに分けることも可能で、\
N個のツールを並行して呼び出すことができます：\n\n\
{FN_NAME}: ツール1の名前、[{tool_names}]のいずれかである必要があります\n\
{FN_ARGS}: ツール1への入力\n\
{FN_NAME}: ツール2の名前\n\
{FN_ARGS}: ツール2への入力\n\
...\n\
{FN_NAME}: ツールNの名前\n\
{FN_ARGS}: ツールNへの入力\n\
## ツールは以下の結果を返します（**呼び出し時に{FN_RESULT}や{FN_EXIT}を出力する必要はありません。後で応答を構成する機会があります**）：\n\
{FN_RESULT}: ツール1の結果\n\
{FN_RESULT}: ツール2の結果\n\
...\n\
{FN_RESULT}: ツールNの結果\n\
## ツールの結果を受け取った後：\n\
{FN_EXIT} 結果が正しいか確認し、その結果に基づいてさらなる処理、修正、または応答を行ってください。`![説明](/api/image/{{uuid}})` を使用して画像を参照できます。\n",
            single_call_template:
"## 応答に以下のコマンドを0回、1回、または複数回挿入することでツールを呼び出せます。\
呼び出しは1ターンまたは複数ターンで行うことができ、各ターンの結果は保存され、将来的に使用可能です。\
例えば、まずツール1を呼び出して会話を終了し、ツール1の戻り値に基づいてツール2を呼び出す...といった具合です。\
応答が完了した後、ツールの応答を受け取ります：\n\
{FN_NAME}: ツール名、[{tool_names}]のいずれかである必要があります。\n\
{FN_ARGS}: ツールへの入力\n\n\
## ツールは以下の結果を返します（**呼び出し時に{FN_RESULT}や{FN_EXIT}を出力する必要はありません。後で応答を構成する機会があります**）\n\
{FN_RESULT}: ツールの結果\n\
## ツールの結果を受け取った後：\n\
{FN_EXIT} 結果が正しいか確認し、その結果に基づいてさらなる処理、修正、または応答を行ってください。`![説明](/api/image/{{uuid}})` を使用して画像を参照できます。\n",
        },

        // -----------------------------------------------------------------
        // 韩语 (Korean)
        // -----------------------------------------------------------------
        Lang::Kor => SystemPromptTemplates {
            tool_info_template: r###" 도구
## 다음 도구들을 사용할 수 있습니다：
{tool_descs}"###,
            parallel_call_template:
"## 응답에 다음 명령을 삽입하여 N개의 도구를 병렬로 호출할 수 있습니다：\n\n\
{FN_NAME}: 도구 1의 이름, [{tool_names}] 중 하나여야 합니다\n\
{FN_ARGS}: 도구 1의 입력\n\
{FN_NAME}: 도구 2의 이름\n\
{FN_ARGS}: 도구 2의 입력\n\
...\n\
{FN_NAME}: 도구 N의 이름\n\
{FN_ARGS}: 도구 N의 입력\n\
## 도구는 다음 결과를 반환합니다 (**호출 시 {FN_RESULT} 및 {FN_EXIT}를 출력할 필요가 없습니다. 나중에 응답을 구성할 기회가 있습니다**)：\n\
{FN_RESULT}: 도구 1의 결과\n\
{FN_RESULT}: 도구 2의 결과\n\
...\n\
{FN_RESULT}: 도구 N의 결과\n\
## 도구 결과를 받은 후：\n\
{FN_EXIT} 결과가 올바른지 확인하고, 그 결과에 따라 추가 처리, 수정 또는 응답하십시오. `![설명](/api/image/{{uuid}})`을 사용하여 이미지를 참조할 수 있습니다.\n",
            single_call_template:
"## 응답에 다음 명령을 0번, 1번 또는 여러 번 삽입하여 도구를 호출할 수 있습니다.\
호출은 한 턴 또는 여러 턴으로 진행될 수 있으며, 각 턴의 도구 호출 결과는 저장되어 나중에 사용할 수 있습니다.\
예를 들어 먼저 도구 1을 호출하여 대화를 마치고, 도구 1이 반환된 후 그 결과에 따라 도구 2를 호출하는 식입니다.\
응답이 끝나면 도구의 응답을 받게 됩니다：\n\
{FN_NAME}: 도구 이름, [{tool_names}] 중 하나여야 합니다.\n\
{FN_ARGS}: 도구 입력\n\n\
## 도구는 다음 결과를 반환합니다 (**호출 시 {FN_RESULT} 및 {FN_EXIT}를 출력할 필요가 없습니다. 나중에 응답을 구성할 기회가 있습니다**)\n\
{FN_RESULT}: 도구 결과\n\
## 도구 결과를 받은 후：\n\
{FN_EXIT} 결과가 올바른지 확인하고, 그 결과에 따라 추가 처리, 수정 또는 응답하십시오. `![설명](/api/image/{{uuid}})`을 사용하여 이미지를 참조할 수 있습니다.\n",
        },

        // -----------------------------------------------------------------
        // 英语 (English) - 默认
        // -----------------------------------------------------------------
        Lang::Eng | _ => SystemPromptTemplates {
            tool_info_template: r###" Tools
## You have the following tools available:
{tool_descs}"###,
            parallel_call_template:
"## You can insert the following commands in your response to call N tools in parallel:\n\n\
{FN_NAME}: Name of tool 1, must be one of [{tool_names}]\n\
{FN_ARGS}: Input for tool 1\n\
{FN_NAME}: Name of tool 2\n\
{FN_ARGS}: Input for tool 2\n\
...\n\
{FN_NAME}: Name of tool N\n\
{FN_ARGS}: Input for tool N\n\
## The tools will return the following results (**You do not need to output {FN_RESULT} and {FN_EXIT} when calling; you will have a chance to formulate a response later**):\n\
{FN_RESULT}: Result from tool 1\n\
{FN_RESULT}: Result from tool 2\n\
...\n\
{FN_RESULT}: Result from tool N\n\
## After receiving the tool results:\n\
{FN_EXIT} Check if the results are correct, and process, correct, or respond based on them. You can use `![description](/api/image/{{uuid}})` to reference images.\n",
            single_call_template:
"## You can insert the following command zero, one, or multiple times in your response to call tools.\
Calls can be performed in one or multiple turns; results from each turn are saved and can be used in the future.\
For example, call Tool 1 first, end the turn, and then use Tool 1's result to call Tool 2.\
You will receive the tool's response after your reply:\n\
{FN_NAME}: Tool name, must be one of [{tool_names}].\n\
{FN_ARGS}: Tool input\n\n\
## The tool will return the following result (**You do not need to output {FN_RESULT} and {FN_EXIT} when calling; you will have a chance to formulate a response later**)\n\
{FN_RESULT}: Tool result\n\
## After receiving the tool result:\n\
{FN_EXIT} Check if the result is correct, and process, correct, or respond based on it. You can use `![description](/api/image/{{uuid}})` to reference images.\n",
        },
    }
}
