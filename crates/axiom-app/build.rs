//! Build script para axiom-app.
//!
//! Configura recursos específicos do Windows:
//! - Ícone do executável (.ico)
//! - Informações de versão (version info)
//! - Manifesto de aplicação (DPI awareness)

fn main() {
    // Apenas compila recursos Windows se estivermos no Windows
    #[cfg(target_os = "windows")]
    {
        // Se existir um arquivo de ícone, compila ele
        if std::path::Path::new("assets/axiom.ico").exists() {
            embed_resource::compile("assets/axiom.rc", embed_resource::NONE);
        }
    }
}
