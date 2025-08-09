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
                                    Some((mut content, mime_type)) => {
                                        // Inject polyfills for HTML files
                                        if mime_type == "text/html" && final_path.ends_with(".html") {
                                            content = inject_polyfills(content);
                                        }

                                        let request = Request { url: requested_url };
                                        let mut response = Response { headers: Default::default() };

                                        response.add_header("Content-Type", &mime_type);
                                        
                                        // Add CORS headers for better compatibility
                                        response.add_header("Access-Control-Allow-Origin", "*");
                                        response.add_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
                                        response.add_header("Access-Control-Allow-Headers", "Content-Type");
                                        
                                        // Add cache headers for better performance (especially for audio files)
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

/// Inject polyfills for Node.js compatibility
fn inject_polyfills(content: Vec<u8>) -> Vec<u8> {
    let html_content = String::from_utf8_lossy(&content);
    let polyfill_script = r#"
<!-- Node.js Polyfills from CDN -->
<script src="https://cdnjs.cloudflare.com/ajax/libs/path-browserify/1.0.1/path.min.js"></script>
<script src="https://cdnjs.cloudflare.com/ajax/libs/buffer/6.0.3/buffer.min.js"></script>
<script src="https://cdnjs.cloudflare.com/ajax/libs/util/0.12.5/util.min.js"></script>

<script>
// RPG Maker Tauri Polyfills

// Setup global Buffer if not available
if (!window.Buffer && window.buffer) {
    window.Buffer = window.buffer.Buffer;
}

// Enhanced require function with real polyfills
if (!window.require) {
    // Create a modules cache
    const moduleCache = new Map();
    
    window.require = function(module) {
        console.log('[TAURI_POLYFILL] require called for:', module);
        
        // Return cached module if available
        if (moduleCache.has(module)) {
            return moduleCache.get(module);
        }
        
        let moduleExports = null;
        
        // Use real path-browserify polyfill
        if (module === 'path') {
            if (window.path && window.path.posix) {
                moduleExports = window.path.posix; // Use POSIX version for consistency
            } else if (window.path) {
                moduleExports = window.path;
            } else {
                // Fallback minimal path implementation
                moduleExports = {
                    join: function(...paths) {
                        return paths.join('/').replace(/\/+/g, '/');
                    },
                    dirname: function(path) {
                        return path.split('/').slice(0, -1).join('/') || '.';
                    },
                    basename: function(path, ext) {
                        let name = path.split('/').pop() || '';
                        if (ext && name.endsWith(ext)) {
                            name = name.slice(0, -ext.length);
                        }
                        return name;
                    },
                    extname: function(path) {
                        const matches = path.match(/\.[^.]*$/);
                        return matches ? matches[0] : '';
                    },
                    resolve: function(...paths) {
                        return this.join(...paths);
                    },
                    sep: '/',
                    delimiter: ':'
                };
            }
        }
        
        // Enhanced fs polyfill with Tauri integration
        else if (module === 'fs') {
            moduleExports = {
                readFileSync: function(path, options) {
                    console.warn('[TAURI_POLYFILL] fs.readFileSync called for:', path);
                    
                    // For RPG Maker save files, try to use Tauri
                    if (path.includes('.rpgsave') && window.__TAURI__) {
                        // This is synchronous but RPG Maker might handle async
                        console.warn('readFileSync for save file, consider using async alternatives');
                    }
                    
                    // Default behavior - return empty or throw
                    if (options && options.encoding === 'utf8') {
                        return '';
                    }
                    return Buffer.alloc ? Buffer.alloc(0) : new ArrayBuffer(0);
                },
                
                writeFileSync: function(path, data, options) {
                    console.warn('[TAURI_POLYFILL] fs.writeFileSync called for:', path);
                    
                    // For RPG Maker save files, try to use Tauri
                    if (path.includes('.rpgsave') && window.__TAURI__) {
                        // Store the write request for async handling
                        console.log('Attempting to write save file via Tauri');
                        window.__TAURI__.invoke('write_save', { 
                            filename: path, 
                            data: data.toString() 
                        }).catch(console.error);
                    }
                },
                
                existsSync: function(path) {
                    console.warn('[TAURI_POLYFILL] fs.existsSync called for:', path);
                    
                    // For RPG Maker files, check with Tauri
                    if (window.__TAURI__) {
                        // This should be async but RPG Maker expects sync
                        // Store result in a cache for next calls
                        if (!window._fsExistsCache) window._fsExistsCache = new Map();
                        
                        if (window._fsExistsCache.has(path)) {
                            return window._fsExistsCache.get(path);
                        }
                        
                        // Async check, store result
                        window.__TAURI__.invoke('file_exists', { filepath: path })
                            .then(exists => {
                                window._fsExistsCache.set(path, exists);
                            })
                            .catch(() => {
                                window._fsExistsCache.set(path, false);
                            });
                        
                        return false; // Default to false for first call
                    }
                    
                    return false;
                },
                
                // Add other common fs methods
                readdirSync: function(path) {
                    console.warn('[TAURI_POLYFILL] fs.readdirSync not implemented:', path);
                    return [];
                },
                
                statSync: function(path) {
                    console.warn('[TAURI_POLYFILL] fs.statSync not implemented:', path);
                    return {
                        isFile: () => true,
                        isDirectory: () => false,
                        size: 0
                    };
                }
            };
        }
        
        // Buffer module
        else if (module === 'buffer') {
            if (window.Buffer) {
                moduleExports = { Buffer: window.Buffer };
            } else {
                moduleExports = {
                    Buffer: class Buffer extends Uint8Array {
                        constructor(input, encoding) {
                            if (typeof input === 'string') {
                                const encoder = new TextEncoder();
                                super(encoder.encode(input));
                            } else if (typeof input === 'number') {
                                super(input);
                            } else {
                                super(input || 0);
                            }
                        }
                        
                        toString(encoding = 'utf8') {
                            const decoder = new TextDecoder(encoding);
                            return decoder.decode(this);
                        }
                        
                        static from(input, encoding) {
                            return new Buffer(input, encoding);
                        }
                        
                        static alloc(size) {
                            return new Buffer(size);
                        }
                    }
                };
            }
        }
        
        // util module
        else if (module === 'util') {
            moduleExports = window.util || {
                inspect: function(obj) {
                    return JSON.stringify(obj, null, 2);
                },
                format: function(f, ...args) {
                    return f.replace(/%[sdj%]/g, (x) => {
                        if (args.length === 0) return x;
                        switch (x) {
                            case '%s': return String(args.shift());
                            case '%d': return Number(args.shift());
                            case '%j':
                                try {
                                    return JSON.stringify(args.shift());
                                } catch (_) {
                                    return '[Circular]';
                                }
                            default:
                                return x;
                        }
                    });
                }
            };
        }
        
        // NW.js GUI polyfill
        else if (module === 'nw.gui') {
            moduleExports = {
                Window: {
                    get: function() {
                        return {
                            showDevTools: async function() {
                                console.log('[TAURI_POLYFILL] Opening DevTools...');
                                if (window.__TAURI__) {
                                    try {
                                        await window.__TAURI__.invoke('show_dev_tools');
                                    } catch (e) {
                                        console.error('Failed to open DevTools:', e);
                                    }
                                }
                            },
                            closeDevTools: function() {
                                console.log('[TAURI_POLYFILL] closeDevTools called (not implemented)');
                            },
                            close: function() {
                                if (window.__TAURI__) {
                                    window.__TAURI__.window.getCurrentWindow().close();
                                }
                            },
                            reload: function() {
                                window.location.reload();
                            },
                            maximize: function() {
                                if (window.__TAURI__) {
                                    window.__TAURI__.window.getCurrentWindow().maximize();
                                }
                            },
                            minimize: function() {
                                if (window.__TAURI__) {
                                    window.__TAURI__.window.getCurrentWindow().minimize();
                                }
                            }
                        };
                    }
                }
            };
        }
        
        // os module basic polyfill
        else if (module === 'os') {
            moduleExports = {
                platform: function() {
                    return navigator.platform.toLowerCase().includes('win') ? 'win32' : 
                           navigator.platform.toLowerCase().includes('mac') ? 'darwin' : 'linux';
                },
                tmpdir: function() {
                    return '/tmp';
                },
                homedir: function() {
                    return '~';
                }
            };
        }
        
        else {
            console.warn('[TAURI_POLYFILL] Unknown module requested:', module);
            moduleExports = {};
        }
        
        // Cache the module
        if (moduleExports) {
            moduleCache.set(module, moduleExports);
        }
        
        return moduleExports || {};
    };

// Override localStorage for RPG Maker saves
const originalLocalStorage = window.localStorage;
const originalLocalStorage = window.localStorage;

// Create a more sophisticated save system
class TauriSaveManager {
    constructor() {
        this.saveCache = new Map();
        this.pendingWrites = new Map();
    }
    
    async getItem(key) {
        if (this.isSaveKey(key)) {
            try {
                // Check cache first
                if (this.saveCache.has(key)) {
                    return this.saveCache.get(key);
                }
                
                if (window.__TAURI__) {
                    const filename = this.keyToFilename(key);
                    const data = await window.__TAURI__.invoke('read_save', { filename });
                    this.saveCache.set(key, data);
                    return data;
                }
            } catch (e) {
                console.error('Error reading save:', e);
            }
            return null;
        }
        return originalLocalStorage.getItem(key);
    }
    
    async setItem(key, value) {
        if (this.isSaveKey(key)) {
            try {
                if (window.__TAURI__) {
                    const filename = this.keyToFilename(key);
                    
                    // Update cache immediately
                    this.saveCache.set(key, value);
                    
                    // Debounce writes to avoid excessive I/O
                    if (this.pendingWrites.has(key)) {
                        clearTimeout(this.pendingWrites.get(key));
                    }
                    
                    const timeoutId = setTimeout(async () => {
                        try {
                            await window.__TAURI__.invoke('write_save', { 
                                filename, 
                                data: value 
                            });
                            this.pendingWrites.delete(key);
                            console.log(`[TAURI_SAVE] Successfully saved: ${filename}`);
                        } catch (e) {
                            console.error('Error writing save:', e);
                        }
                    }, 500); // 500ms debounce
                    
                    this.pendingWrites.set(key, timeoutId);
                    return;
                }
            } catch (e) {
                console.error('Error writing save:', e);
            }
        }
        originalLocalStorage.setItem(key, value);
    }
    
    async removeItem(key) {
        if (this.isSaveKey(key)) {
            try {
                if (window.__TAURI__) {
                    const filename = this.keyToFilename(key);
                    await window.__TAURI__.invoke('delete_save', { filename });
                    this.saveCache.delete(key);
                    return;
                }
            } catch (e) {
                console.error('Error deleting save:', e);
            }
        }
        originalLocalStorage.removeItem(key);
    }
    
    isSaveKey(key) {
        return key && (
            key.includes('.rpgsave') || 
            key.startsWith('RPG') || 
            key.includes('save') ||
            key.includes('Save') ||
            key.match(/^(file|global|config)\d*$/i)
        );
    }
    
    keyToFilename(key) {
        if (key.endsWith('.rpgsave')) {
            return key;
        }
        return `${key}.rpgsave`;
    }
    
    // Synchronous versions for backward compatibility
    getItemSync(key) {
        if (this.isSaveKey(key) && this.saveCache.has(key)) {
            return this.saveCache.get(key);
        }
        return originalLocalStorage.getItem(key);
    }
}

const tauriSaveManager = new TauriSaveManager();

const tauriStorage = {
    getItem: function(key) {
        // For RPG Maker, try synchronous first (from cache), then async
        const syncResult = tauriSaveManager.getItemSync(key);
        if (syncResult !== null) {
            return syncResult;
        }
        
        // If not in cache, trigger async load but return null for now
        if (tauriSaveManager.isSaveKey(key)) {
            tauriSaveManager.getItem(key).catch(console.error);
            return null;
        }
        
        return originalLocalStorage.getItem(key);
    },
    
    setItem: function(key, value) {
        tauriSaveManager.setItem(key, value).catch(console.error);
    },
    
    removeItem: function(key) {
        tauriSaveManager.removeItem(key).catch(console.error);
    },
    
    key: originalLocalStorage.key.bind(originalLocalStorage),
    clear: originalLocalStorage.clear.bind(originalLocalStorage),
    get length() { return originalLocalStorage.length; }
};

// Replace localStorage with our enhanced version
Object.defineProperty(window, 'localStorage', {
    value: tauriStorage,
    writable: false,
    configurable: false
});

// Global DevTools shortcuts and better integration
document.addEventListener('keydown', function(e) {
    // F12 or Ctrl+Shift+I
    if (e.key === 'F12' || (e.ctrlKey && e.shiftKey && e.key === 'I')) {
        e.preventDefault();
        if (window.__TAURI__) {
            window.__TAURI__.invoke('show_dev_tools').catch(console.error);
        }
    }
    
    // Ctrl+R or F5 for reload
    if ((e.ctrlKey && e.key === 'r') || e.key === 'F5') {
        e.preventDefault();
        window.location.reload();
    }
});

// Enhanced error handling for RPG Maker
window.addEventListener('error', function(e) {
    if (e.message && e.message.includes('require')) {
        console.error('[TAURI_POLYFILL] Require error detected:', e.message);
        console.log('[TAURI_POLYFILL] Available polyfilled modules: path, fs, buffer, util, os, nw.gui');
    }
});

// Pre-load save data when RPG Maker starts
if (window.__TAURI__) {
    document.addEventListener('DOMContentLoaded', async function() {
        try {
            console.log('[TAURI_POLYFILL] Pre-loading save data...');
            const saves = await window.__TAURI__.invoke('list_saves');
            console.log(`[TAURI_POLYFILL] Found ${saves.length} save files`);
            
            // Pre-cache common save keys
            const commonKeys = ['global', 'config', 'file1', 'file2', 'file3'];
            for (const key of commonKeys) {
                try {
                    const data = await tauriSaveManager.getItem(key);
                    if (data) {
                        console.log(`[TAURI_POLYFILL] Pre-cached save: ${key}`);
                    }
                } catch (e) {
                    // Ignore errors for non-existent saves
                }
            }
        } catch (e) {
            console.error('[TAURI_POLYFILL] Error pre-loading saves:', e);
        }
    });
}

console.log('[TAURI_POLYFILL] Enhanced RPG Maker polyfills loaded with CDN dependencies');
</script>
"#;

    // Insert the polyfill script before the closing </head> tag or at the beginning of <body>
    let modified_html = if html_content.contains("</head>") {
        html_content.replace("</head>", &format!("{}\n</head>", polyfill_script))
    } else if html_content.contains("<body>") {
        html_content.replace("<body>", &format!("<body>\n{}", polyfill_script))
    } else {
        format!("{}\n{}", polyfill_script, html_content)
    };

    modified_html.into_bytes()
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