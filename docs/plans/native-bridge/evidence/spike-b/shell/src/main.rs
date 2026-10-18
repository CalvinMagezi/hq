use tauri::{WebviewUrl, WebviewWindowBuilder};

const INIT: &str = include_str!("init.js");

fn main() {
    let url = std::env::var("SPIKE_URL").expect("set SPIKE_URL");
    tauri::Builder::default()
        .setup(move |app| {
            if let (Ok(bin), Ok(dir)) =
                (std::env::var("SPIKE_HOST"), std::env::var("SPIKE_HOST_DIR"))
            {
                let log = std::fs::File::create(format!("{dir}.child.log"))?;
                std::process::Command::new(bin)
                    .args(["host", "serve", "--dir", &dir])
                    .stdout(log.try_clone()?)
                    .stderr(log)
                    .spawn()?;
            }
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url.parse().unwrap()))
                .title("spike-shell")
                .initialization_script(INIT)
                .on_document_title_changed(|_w, title| {
                    if title.starts_with("RESULT:") {
                        println!("{title}");
                    }
                })
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("run");
}
