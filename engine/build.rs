use std::path::Path;

fn main() {
    // A UI compilada (ui/dist) é embutida no executável. Se ainda não foi
    // gerada, cria uma pasta vazia para compilar mesmo assim; o servidor
    // mostra uma mensagem pedindo `npm run build`.
    let ui = Path::new("../ui/dist");
    if !ui.join("index.html").is_file() {
        println!("cargo:warning=ui/dist não encontrada; rode `npm run build` em ui/ para embutir a interface");
        let _ = std::fs::create_dir_all(ui);
    }
    println!("cargo:rerun-if-changed=../ui/dist");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("ProductName", "ScreamO")
            .set("FileDescription", "ScreamO - pedaleira de guitarra em tempo real")
            .set("LegalCopyright", "© Just-Gabe");
        if let Err(e) = res.compile() {
            println!("cargo:warning=ícone não embutido: {e}");
        }
    }
}
