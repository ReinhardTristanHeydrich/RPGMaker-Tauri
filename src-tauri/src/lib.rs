mod external_localhost_plugin;

use std::path::PathBuf;
use tauri::{WebviewUrl, WebviewWindowBuilder, GlobalShortcutEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let port = portpicker::pick_unused_port().expect("failed to find unused port");
    
    // Função para encontrar a pasta Game_Contents
    fn find_game_contents() -> Option<PathBuf> {
        // 1. Primeiro, tenta o diretório onde o executável está
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let game_contents = exe_dir.join("Game_Contents");
                if game_contents.exists() {
                    println!("Found Game_Contents at: {:?}", game_contents);
                    return Some(game_contents);
                }
            }
        }
        
        // 2. Tenta o diretório de trabalho atual
        if let Ok(current_dir) = std::env::current_dir() {
            let game_contents = current_dir.join("Game_Contents");
            if game_contents.exists() {
                println!("Found Game_Contents at: {:?}", game_contents);
                return Some(game_contents);
            }
        }
        
        // 3. Tenta alguns diretórios comuns relativos
        let common_paths = [
            "Game_Contents",
            "www",
        ];
        
        for path in &common_paths {
            let game_contents = PathBuf::from(path);
            if game_contents.exists() {
                println!("Found Game_Contents at: {:?}", game_contents.canonicalize().unwrap_or(game_contents.clone()));
                return Some(game_contents);
            }
        }
        
        None
    }
    
    // Busca a pasta Game_Contents
    let game_contents_path = match find_game_contents() {
        Some(path) => {
            println!("Using Game_Contents folder: {:?}", path);
            path
        }
        None => {
            eprintln!("Error: Game_Contents folder not found!");
            eprintln!("Searched in the following locations:");
            if let Ok(exe_path) = std::env::current_exe() {
                if let Some(exe_dir) = exe_path.parent() {
                    eprintln!("  - {:?}", exe_dir.join("Game_Contents"));
                }
            }
            if let Ok(current_dir) = std::env::current_dir() {
                eprintln!("  - {:?}", current_dir.join("Game_Contents"));
            }
            eprintln!("  - Game_Contents");
            eprintln!("  - ../Game_Contents");
            eprintln!("  - ../../Game_Contents");
            eprintln!("  - ./dist/Game_Contents");
            eprintln!("");
            eprintln!("Please create a symlink or copy your RPG Maker game files to one of these locations.");
            
            // Em caso de desenvolvimento, permite continuar sem a pasta
            std::env::current_dir().unwrap_or_default()
        }
    };
    
    // Verificar se é realmente um diretório
    if game_contents_path.exists() && !game_contents_path.is_dir() {
        eprintln!("Warning: Game_Contents exists but is not a directory: {:?}", game_contents_path);
    }
    
    println!("Starting server on port {} serving from: {:?}", port, game_contents_path);
    
    let url_string = format!("http://127.0.0.1:{}/", port);
    let webview_url = WebviewUrl::External(url_string.parse().expect("Invalid localhost URL format"));
    
    tauri::Builder::default()
        .plugin(
            external_localhost_plugin::Builder::new(port)
                .host("127.0.0.1")
                .external_folder(&game_contents_path)
                .build()
        )
        .setup(move |app| {
            println!("Creating window with URL: {}", url_string);
            
            // Aguarda um pouco para garantir que o servidor esteja rodando
            std::thread::sleep(std::time::Duration::from_millis(500));
            
            let window = WebviewWindowBuilder::new(app, "main", webview_url)
                .title("RPG Maker Game Launcher")
                .inner_size(1280.0, 720.0)
                .resizable(true)
                .devtools(true) //Activate Devtools
                .build()?;
            
            // Registrar atalho global para F12
            let app_handle = app.handle().clone();
            app.global_shortcut().register("F12", move || {
                if let Some(window) = app_handle.get_webview_window("main") {
                    // Toggle DevTools
                    if window.is_devtools_open() {
                        let _ = window.close_devtools();
                    } else {
                        let _ = window.open_devtools();
                    }
                }
            })?;
            
            // Também registrar Ctrl+Shift+I (atalho alternativo comum)
            let app_handle2 = app.handle().clone();
            app.global_shortcut().register("Ctrl+Shift+I", move || {
                if let Some(window) = app_handle2.get_webview_window("main") {
                    if window.is_devtools_open() {
                        let _ = window.close_devtools();
                    } else {
                        let _ = window.open_devtools();
                    }
                }
            })?;
            
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}