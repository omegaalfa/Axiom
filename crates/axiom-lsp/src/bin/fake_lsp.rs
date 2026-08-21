use std::io::{self, BufReader};

use axiom_lsp::{read_message, write_message};
use serde_json::{Value, json};

fn main() {
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    while let Ok(Some(message)) = read_message(&mut reader) {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            continue;
        };
        let id = message.get("id").cloned();
        let result = match method {
            "initialize" => {
                json!({"capabilities":{"positionEncoding":"utf-16","textDocumentSync":1,"completionProvider":{"resolveProvider":true},"hoverProvider":true,"definitionProvider":true,"referencesProvider":true,"documentFormattingProvider":true,"signatureHelpProvider":{"triggerCharacters":["(",","]}}})
            }
            "textDocument/completion" => json!([{"label":"findByEmail","detail":"method"}]),
            "textDocument/hover" => {
                json!({"contents":{"kind":"markdown","value":"`UserRepository::findByEmail`"}})
            }
            "textDocument/definition" | "textDocument/references" => json!([]),
            "textDocument/formatting" => {
                json!([{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":""}])
            }
            "textDocument/signatureHelp" => {
                json!({"signatures":[{"label":"findByEmail(string $email): ?User"}],"activeSignature":0,"activeParameter":0})
            }
            "shutdown" => Value::Null,
            "exit" => break,
            "textDocument/didOpen" => {
                let params = &message["params"];
                let diagnostics = json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":params["textDocument"]["uri"],"version":params["textDocument"]["version"],"diagnostics":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":5}},"severity":2,"message":"fake warning","source":"fake-lsp"}]}});
                let _ = write_message(&mut writer, &diagnostics);
                continue;
            }
            _ => continue,
        };
        if let Some(id) = id {
            let _ = write_message(
                &mut writer,
                &json!({"jsonrpc":"2.0","id":id,"result":result}),
            );
        }
    }
}
