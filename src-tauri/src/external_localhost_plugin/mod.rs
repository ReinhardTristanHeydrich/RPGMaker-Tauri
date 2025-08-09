// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Expose external folder assets through a localhost server for RPG Maker games.
//!
//! **Note: This plugin brings considerable security risks and you should only use it if you know what you are doing.**

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;
use std::io::Read;

use http::Uri;
use percent_encoding::percent_decode_str;
use tauri::{
    plugin::{Builder as PluginBuilder, TauriPlugin},
    Runtime,
};
use tiny_http::{Header, Response as HttpResponse, Server};

pub struct Request {
    url: String,
}

impl Request {
    pub fn url(&self) -> &str {
        &self.url
    }
}

pub struct Response {
    headers: HashMap<String, String>,
}

impl Response {
    pub fn add_header<H: Into<String>, V: Into<String>>(&mut self, header: H, value: V) {
        self.headers.insert(header.into(), value.into());
    }
}

type OnRequest = Option<Box<dyn Fn(&Request, &mut Response) + Send + Sync>>;

pub struct Builder {
    port: u16,
    host: Option<String>,
    on_request: OnRequest,
    external_folder: Option<PathBuf>,
}

impl Builder {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            host: None,
            on_request: None,
            external_folder: None,
        }
    }

    /// Change the host the plugin binds to. Defaults to `localhost`.
    pub fn host<H: Into<String>>(mut self, host: H) -> Self {
        self.host = Some(host.into());
        self
    }

    /// Set the external folder to serve files from (e.g., "Game_Contents").
    pub fn external_folder<P: AsRef<Path>>(mut self, folder: P) -> Self {
        self.external_folder = Some(folder.as_ref().to_path_buf());
        self
    }

    pub fn on_request<F: Fn(&Request, &mut Response) + Send + Sync + 'static>(
        mut self,
        f: F,
    ) -> Self {
        self.on_request.replace(Box::new(f));
        self
    }

    pub fn build<R: Runtime>(mut self) -> TauriPlugin<R> {
        let port = self.port;
        let host = self.host.unwrap_or("localhost".to_string());
        let on_request = self.on_request.take();
        let external_folder = self.external_folder;

        PluginBuilder::new("external-localhost")
            .setup(move |_app, _api| {
                let server_address = format!("{host}:{port}");
                let on_request_clone = on_request.map(|f| std::sync::Arc::new(f));

                std::thread::spawn(move || {
                    let server = match Server::http(&server_address) {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("Failed to create server: {}", e);
                            return;
                        }
                    };
                    
                    for req in server.incoming_requests() {
                        let requested_url = req.url().to_string();
                        let path_result = requested_url
                            .parse::<Uri>()
                            .map(|uri| uri.path().to_string())
                            .map_err(|e| format!("Error parsing URI '{}': {}", requested_url, e));

                        match path_result {
                            Ok(mut path) => {
                                // Decode percent-encoded URLs (critical for RPG Maker compatibility)
                                let decoded_path_cow = percent_decode_str(&path).decode_utf8_lossy();
                                path = decoded_path_cow.to_string();

                                // Handle root path and remove leading slash
                                if path == "/" {
                                    path = "/index.html".to_string();
                                }
                                
                                let file_path = if path.starts_with('/') {
                                    &path[1..]
                                } else {
                                    &path
                                };

                                // Default to index.html if path is empty
                                let final_path = if file_path.is_empty() {
                                    "index.html"
                                } else {
                                    file_path
                                };

                                let file_content = if let Some(ref external_folder) = external_folder {
                                    // Use external folder
                                    let full_path = external_folder.join(final_path);
                                    load_external_file(&full_path)
                                } else {
                                    // Fallback to current directory + Game_Contents
                                    let current_dir = std::env::current_exe()
                                        .ok()
                                        .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
                                        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                                    
                                    let full_path = current_dir.join("Game_Contents").join(final_path);
                                    load_external_file(&full_path)
                                };

                                match file_content {
                                    Some((content, mime_type)) => {
                                        let request = Request { url: requested_url };
                                        let mut response = Response { headers: Default::default() };

                                        // Use uma referência para mime_type na primeira vez
                                        response.add_header("Content-Type", &mime_type);
                                        
                                        // Add CORS headers for better compatibility
                                        response.add_header("Access-Control-Allow-Origin", "*");
                                        response.add_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
                                        response.add_header("Access-Control-Allow-Headers", "Content-Type");
                                        
                                        // Add cache headers for better performance (especially for audio files)
                                        // Agora mime_type ainda está disponível para uso
                                        if mime_type.starts_with("audio/") || mime_type.starts_with("image/") {
                                            response.add_header("Cache-Control", "public, max-age=31536000");
                                        }

                                        if let Some(on_req_fn) = &on_request_clone {
                                            on_req_fn(&request, &mut response);
                                        }

                                        let mut resp = HttpResponse::from_data(content);
                                        for (header, value) in response.headers {
                                            if let Ok(h) = Header::from_bytes(header.as_bytes(), value.as_bytes()) {
                                                resp.add_header(h);
                                            }
                                        }
                                        
                                        let _ = req.respond(resp);
                                    }
                                    None => {
                                        let response_404 = HttpResponse::from_string("Not Found")
                                            .with_status_code(404)
                                            .with_header(Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..]).unwrap());
                                        let _ = req.respond(response_404);
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("URI Parse Error: {}", e);
                                let response_500 = HttpResponse::from_string("Internal Server Error - URI Parse Error")
                                    .with_status_code(500)
                                    .with_header(Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..]).unwrap());
                                let _ = req.respond(response_500);
                            }
                        }
                    }
                });
                Ok(())
            })
            .build()
    }
}

/// Load a file from the external filesystem
fn load_external_file(file_path: &Path) -> Option<(Vec<u8>, String)> {
    if !file_path.exists() || !file_path.is_file() {
        return None;
    }

    let mut file = fs::File::open(file_path).ok()?;
    let mut content = Vec::new();
    file.read_to_end(&mut content).ok()?;

    let mime_type = get_mime_type(file_path);
    Some((content, mime_type))
}

/// Get MIME type based on file extension
fn get_mime_type(file_path: &Path) -> String {
    let extension = file_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();

    match extension.as_str() {
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "rpgmvo" => "audio/ogg", // RPG Maker encrypted audio
        "rpgmvm" => "audio/mp4", // RPG Maker encrypted audio
        "rpgmvp" => "image/png", // RPG Maker encrypted image
        "rpgmvw" => "audio/wav", // RPG Maker encrypted audio
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "txt" => "text/plain",
        "xml" => "application/xml",
        _ => "application/octet-stream",
    }.to_string()
}