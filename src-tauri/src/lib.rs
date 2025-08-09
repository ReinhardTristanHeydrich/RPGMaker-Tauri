mod external_localhost_plugin;

use std::path::PathBuf;
use std::fs;
use std::io::{Read, Write};
use tauri::{WebviewUrl, WebviewWindowBuilder, command, State, Manager};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct SaveFile {
    name: String,
    data: String,
}

// Comando para listar saves
#[command]
fn list_saves(save_dir: State<PathBuf>) -> Result<Vec<String>, String> {
    let save_path = save_dir.inner();
    
    if !save_path.exists() {
        fs::create_dir_all(save_path).map_err(|e| format!("Failed to create save directory: {}", e))?;
    }
    
    let mut saves = Vec::new();
    
    if let Ok(entries) = fs::read_dir(save_path) {
        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if let Some(extension) = path.extension() {
                    if extension == "rpgsave" {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            saves.push(name.to_string());
                        }
                    }
                }
            }
        }
    }
    
    Ok(saves)
}

// Comando para ler um save
#[command]
fn read_save(filename: String, save_dir: State<PathBuf>) -> Result<String, String> {
    let save_path = save_dir.inner().join(&filename);
    
    if !save_path.exists() {
        return Err("Save file not found".to_string());
    }
    
    let mut file = fs::File::open(save_path).map_err(|e| format!("Failed to open save file: {}", e))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).map_err(|e| format!("Failed to read save file: {}", e))?;
    
    Ok(contents)
}

// Comando para escrever um save
#[command]
fn write_save(filename: String, data: String, save_dir: State<PathBuf>) -> Result<(), String> {
    let save_path = save_dir.inner();
    
    if !save_path.exists() {
        fs::create_dir_all(save_path).map_err(|e| format!("Failed to create save directory: {}", e))?;
    }
    
    let file_path = save_path.join(&filename);
    let mut file = fs::File::create(file_path).map_err(|e| format!("Failed to create save file: {}", e))?;
    file.write_all(data.as_bytes()).map_err(|e| format!("Failed to write save file: {}", e))?;
    
    Ok(())
}

// Comando para deletar um save
#[command]
fn delete_save(filename: String, save_dir: State<PathBuf>) -> Result<(), String> {
    let save_path = save_dir.inner().join(&filename);
    
    if save_path.exists() {
        fs::remove_file(save_path).map_err(|e| format!("Failed to delete save file: {}", e))?;
    }
    
    Ok(())
}

// Comando para abrir DevTools
#[command]
async fn show_dev_tools(app_handle: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("main") {
        window.open_devtools();
        Ok(())
    } else {
        Err("Window not found".to_string())
    }
}

// Comando para verificar se um arquivo existe
#[command]
fn file_exists(filepath: String, game_dir: State<PathBuf>) -> Result<bool, String> {
    let full_path = game_dir.inner().join(&filepath);
    Ok(full_path.exists())
}

// Comando para ler arquivo do jogo
#[command]
fn read_game_file(filepath: String, game_dir: State<PathBuf>) -> Result<String, String> {
    let full_path = game_dir.inner().join(&filepath);
    
    if !full_path.exists() {
        return Err("File not found".to_string());
    }
    
    let mut file = fs::File::open(full_path).map_err(|e| format!("Failed to open file: {}", e))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).map_err(|e| format!("Failed to read file: {}", e))?;
    
    Ok(contents)
}

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
            "../Game_Contents",
            "../../Game_Contents",
            "./dist/Game_Contents",
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
    
    // Define diretório de saves
    let save_dir = if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            exe_dir.join("saves")
        } else {
            PathBuf::from("./saves")
        }
    } else {
        PathBuf::from("./saves")
    };
    
    // Verificar se é realmente um diretório
    if game_contents_path.exists() && !game_contents_path.is_dir() {
        eprintln!("Warning: Game_Contents exists but is not a directory: {:?}", game_contents_path);
    }
    
    println!("Starting server on port {} serving from: {:?}", port, game_contents_path);
    println!("Save directory: {:?}", save_dir);
    
    let url_string = format!("http://127.0.0.1:{}/", port);
    let webview_url = WebviewUrl::External(url_string.parse().expect("Invalid localhost URL format"));
    
    tauri::Builder::default()
        .plugin(
            external_localhost_plugin::Builder::new(port)
                .host("127.0.0.1")
                .external_folder(&game_contents_path)
                .build()
        )
        .manage(save_dir)
        .manage(game_contents_path)
        .invoke_handler(tauri::generate_handler![
            list_saves,
            read_save,
            write_save,
            delete_save,
            show_dev_tools,
            file_exists,
            read_game_file
        ])
        .setup(move |app| {
            println!("Creating window with URL: {}", url_string);
            
            // Aguarda um pouco para garantir que o servidor esteja rodando
            std::thread::sleep(std::time::Duration::from_millis(500));
            
            let _window = WebviewWindowBuilder::new(app, "main", webview_url)
                .title("RPG Maker Game Launcher")
                .inner_size(1280.0, 720.0)
                .resizable(true)
                .build()?;
            
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}