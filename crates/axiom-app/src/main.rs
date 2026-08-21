//! Axiom desktop application.

mod editor_view;
mod lsp_bridge;
mod syntax_theme;
mod terminal_view;
mod ui;
mod workspace_view;

use axiom_app::shell_state::resolve_startup_target;
use gpui::{
    App, AppContext, Application, Bounds, SharedString, TitlebarOptions, WindowBounds,
    WindowOptions, px, size,
};
use ui::icons::AxiomAssets;
use workspace_view::WorkspaceView;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args: Vec<_> = std::env::args_os().collect();
    let cwd = std::env::current_dir()?;
    let project_override = std::env::var_os("AXIOM_PROJECT")
        .or_else(|| std::env::var_os("RUSTSTORM_PROJECT"))
        .map(std::path::PathBuf::from);
    let startup = resolve_startup_target(&args, project_override.as_deref(), &cwd);

    Application::new()
        .with_assets(AxiomAssets)
        .run(move |cx: &mut App| {
            cx.bind_keys(editor_view::key_bindings());
            cx.bind_keys(workspace_view::key_bindings());
            cx.bind_keys(terminal_view::key_bindings());

            let bounds = Bounds::centered(None, size(px(1280.0), px(800.0)), cx);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        titlebar: Some(TitlebarOptions {
                            title: Some(SharedString::from("Axiom")),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    |_window, cx| {
                        let startup = startup.clone();
                        cx.new(|cx| WorkspaceView::new(startup, cx))
                    },
                )
                .expect("falha ao abrir a janela principal do Axiom");

            window
                .update(cx, |editor, window, cx| {
                    use gpui::Focusable;
                    window.focus(&editor.focus_handle(cx));
                    cx.activate(true);
                })
                .expect("falha ao focar o editor");
        });

    Ok(())
}
