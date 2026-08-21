use std::{path::Path, time::Duration};

use axiom_lsp::{LanguageServer, ServerEvent, ServerStatus, normalize_completions, path_to_uri};
use lsp_types::{CompletionResponse, GotoDefinitionResponse, Hover, Position};

#[test]
fn fake_server_covers_lifecycle_sync_and_requests() {
    let executable = env!("CARGO_BIN_EXE_fake-lsp");
    let mut server = LanguageServer::start(executable, &[]).unwrap();
    server
        .initialize(Path::new(env!("CARGO_MANIFEST_DIR")))
        .unwrap();
    assert_eq!(server.status(), ServerStatus::Ready);
    let uri = path_to_uri(&Path::new(env!("CARGO_MANIFEST_DIR")).join("test.php")).unwrap();
    server.did_open(uri.clone(), 1, "<?php".into()).unwrap();
    server
        .did_change(uri.clone(), 2, "<?php echo 1;".into())
        .unwrap();
    server.did_save(uri.clone(), None).unwrap();
    let completion: Option<CompletionResponse> = server
        .completion(uri.clone(), Position::new(0, 5))
        .unwrap()
        .recv(Duration::from_secs(2))
        .unwrap();
    assert_eq!(normalize_completions(completion)[0].label, "findByEmail");
    let hover: Option<Hover> = server
        .hover(uri.clone(), Position::new(0, 5))
        .unwrap()
        .recv(Duration::from_secs(2))
        .unwrap();
    assert!(hover.is_some());
    let definition: Option<GotoDefinitionResponse> = server
        .definition(uri.clone(), Position::new(0, 5))
        .unwrap()
        .recv(Duration::from_secs(2))
        .unwrap();
    assert!(definition.is_none() || matches!(definition, Some(GotoDefinitionResponse::Array(_))));
    let _: Vec<lsp_types::Location> = server
        .references(uri.clone(), Position::new(0, 5))
        .unwrap()
        .recv(Duration::from_secs(2))
        .unwrap();
    let mut saw_diagnostics = false;
    for _ in 0..20 {
        if matches!(server.try_event(), Some(ServerEvent::Diagnostics(_))) {
            saw_diagnostics = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(saw_diagnostics);
    server.did_close(uri).unwrap();
    server.shutdown().unwrap();
    server.wait().unwrap();
}

#[test]
fn unexpected_server_exit_is_controlled() {
    let executable = env!("CARGO_BIN_EXE_fake-lsp");
    let server = LanguageServer::start(executable, &[]).unwrap();
    server.notify("exit", serde_json::Value::Null).unwrap();
    server.wait().unwrap();
    for _ in 0..20 {
        if server.status() == ServerStatus::Stopped {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("server did not transition to stopped");
}
