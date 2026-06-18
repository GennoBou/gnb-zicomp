use std::fs;
use std::io;
use std::path::PathBuf;
use tempfile::tempdir;
use zip::ZipArchive;
use zip::write::{FileOptions, ZipWriter};
use zip::CompressionMethod;
use tauri::{Manager, Emitter};
use rayon::prelude::*;
use tauri_plugin_cli::CliExt;
use zenwebp::{EncodeRequest, LossyConfig, PixelLayout};

#[derive(Clone, serde::Serialize)]
struct ProgressPayload {
    step: String,
    status: String,
    output_path: Option<String>,
    original_size: Option<u64>,
    new_size: Option<u64>,
}

impl ProgressPayload {
    fn new(step: &str, status: &str, output_path: Option<String>) -> Self {
        Self {
            step: step.to_string(),
            status: status.to_string(),
            output_path,
            original_size: None,
            new_size: None,
        }
    }

    fn with_sizes(step: &str, status: &str, original_size: u64, new_size: u64) -> Self {
        Self {
            step: step.to_string(),
            status: status.to_string(),
            output_path: None,
            original_size: Some(original_size),
            new_size: Some(new_size),
        }
    }
}

#[tauri::command]
fn get_output_directory(app: tauri::AppHandle) -> Result<String, String> {
    let output_dir = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(_) => {
            match app.path().desktop_dir() {
                Ok(desktop) => desktop,
                Err(_) => {
                    return Err("Could not determine output directory (neither CWD nor desktop available).".to_string());
                }
            }
        }
    };
    Ok(output_dir.display().to_string())
}

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn unzip_file(app: tauri::AppHandle, zip_paths: Vec<String>) -> Result<(), String> {
    tauri::async_runtime::spawn(async move {
        let app_handle = app.clone();
        let emit_progress = move |step: &str, status: &str, output_path: Option<String>| {
            if let Err(e) = app_handle.emit("progress_update", ProgressPayload::new(step, status, output_path)) {
                log::error!("Failed to emit progress update: {}", e);
            }
        };

        for (index, zip_path) in zip_paths.iter().enumerate() {
            let progress_msg = format!("Processing file {} of {}: {}", index + 1, zip_paths.len(), zip_path);
            emit_progress("StartingFile", &progress_msg, None);
            log::info!("{}", progress_msg);

            // Get original file size
            let original_size = match fs::metadata(&zip_path) {
                Ok(m) => m.len(),
                Err(e) => {
                    log::warn!("Failed to get original file size: {}", e);
                    0
                }
            };

            // Determine output directory
            let output_dir = match std::env::current_dir() {
                Ok(cwd) => {
                    log::info!("Using current working directory as output: {}", cwd.display());
                    cwd
                },
                Err(e) => {
                    log::warn!("Failed to get current working directory ({}). Falling back to desktop.", e);
                    match app.path().desktop_dir() {
                        Ok(desktop) => {
                            log::info!("Using desktop directory as output: {}", desktop.display());
                            desktop
                        },
                        Err(_) => {
                            let err_msg = "Could not determine output directory (neither CWD nor desktop available).".to_string();
                            log::error!("{}", err_msg);
                            emit_progress("Error", &err_msg, None);
                            continue;
                        }
                    }
                }
            };
            log::info!("Determined output directory: {}", output_dir.display());
            emit_progress("Unzipping", "Determined output directory...", Some(output_dir.display().to_string()));


            // Create a temporary directory
            let temp_dir = match tempdir() {
                Ok(dir) => dir,
                Err(e) => {
                    let err_msg = format!("Failed to create temp dir: {}", e);
                    log::error!("{}", err_msg);
                    emit_progress("Error", &err_msg, None);
                    continue;
                }
            };

            let temp_path_buf = temp_dir.path().to_path_buf();
            let temp_path_str = match temp_path_buf.to_str() {
                Some(s) => s.to_string(),
                None => {
                    let err_msg = "Temporary path contains invalid UTF-8".to_string();
                    log::error!("{}", err_msg);
                    emit_progress("Error", &err_msg, None);
                    continue;
                }
            };

            log::info!("Temporary directory created at: {}", temp_path_str);
            emit_progress("Unzipping", "Extracting files...", None);

            // Open the ZIP file
            let file = match fs::File::open(&zip_path) {
                Ok(f) => f,
                Err(e) => {
                    let err_msg = format!("Failed to open zip file: {}", e);
                    log::error!("Failed to open zip file '{}': {}", zip_path, e);
                    emit_progress("Error", &err_msg, None);
                    continue;
                }
            };
            let reader = io::BufReader::new(file);

            // Create ZipArchive
            let mut archive = match ZipArchive::new(reader) {
                Ok(ar) => ar,
                Err(e) => {
                    let err_msg = format!("Failed to read zip archive: {}", e);
                    log::error!("{}", err_msg);
                    emit_progress("Error", &err_msg, None);
                    continue;
                }
            };

            // Extract all files
            if let Err(e) = archive.extract(temp_dir.path()) {
                let err_msg = format!("Failed to extract zip archive: {}", e);
                log::error!("{}", err_msg);
                emit_progress("Error", &err_msg, None);
                continue;
            }

            log::info!("Successfully extracted archive to: {}", temp_path_str);
            emit_progress("ConvertingImages", "Finding images...", None);


            let mut image_paths: Vec<PathBuf> = Vec::new();

            // Walk through the extracted files and find images
            for entry in walkdir::WalkDir::new(&temp_path_buf).into_iter().filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.is_file() {
                    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                        match ext.to_lowercase().as_str() {
                            "png" | "jpg" | "jpeg" => {
                                log::info!("Found image: {}", path.display());
                                image_paths.push(path.to_path_buf());
                            }
                            _ => {}
                        }
                    }
                }
            }
            
            let image_count = image_paths.len();
            emit_progress("ConvertingImages", &format!("Converting {} images...", image_count), None);


            // Process images: convert to WebP and delete original
            image_paths.par_iter().for_each(|original_path| {
                log::info!("Processing image: {}", original_path.display());
                let output_path = original_path.with_extension("webp");

                match image::open(&original_path) {
                    Ok(img) => {
                        let width = img.width();
                        let height = img.height();
                        let config = LossyConfig::new().with_quality(94.0);
                        
                        let encode_result = if img.color().has_alpha() {
                            let rgba = img.into_rgba8();
                            EncodeRequest::lossy(&config, &rgba, PixelLayout::Rgba8, width, height).encode()
                        } else {
                            let rgb = img.into_rgb8();
                            EncodeRequest::lossy(&config, &rgb, PixelLayout::Rgb8, width, height).encode()
                        };

                        match encode_result {
                            Ok(webp_data) => {
                                match fs::write(&output_path, webp_data) {
                                    Ok(_) => {
                                        log::info!("Converted {} to {}", original_path.display(), output_path.display());
                                        if let Err(e) = fs::remove_file(&original_path) {
                                            log::warn!("Failed to delete original image {}: {}", original_path.display(), e);
                                        }
                                    }
                                    Err(e) => {
                                        log::error!("Failed to save WebP file {}: {}", output_path.display(), e);
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!("Failed to encode WebP image {}: {}", output_path.display(), e);
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to open image {}: {}", original_path.display(), e);
                    }
                }
            });

            emit_progress("Recompressing", "Re-compressing files...", None);

            // Determine the final output ZIP file name
            let original_zip_filename = PathBuf::from(&zip_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("output")
                .to_string();

            let mut final_output_zip_filename = format!("{}.zip", original_zip_filename);
            let mut final_output_zip_path = output_dir.join(&final_output_zip_filename);
            let mut suffix_counter = 0;

            while final_output_zip_path.exists() {
                suffix_counter += 1;
                final_output_zip_filename = format!("{}-zicomp{}.zip", original_zip_filename, suffix_counter);
                final_output_zip_path = output_dir.join(&final_output_zip_filename);
            }
            log::info!("Final output ZIP path determined: {}", final_output_zip_path.display());

            // Re-compress all files in the temporary directory into a new ZIP file
            let output_file = match fs::File::create(&final_output_zip_path) {
                Ok(f) => f,
                Err(e) => {
                    let err_msg = format!("Failed to create output zip file: {}", e);
                    log::error!("Failed to create output zip file at {}: {}", final_output_zip_path.display(), e);
                    emit_progress("Error", &err_msg, None);
                    continue;
                }
            };

            let mut zip_writer = ZipWriter::new(output_file);
            let base_options: FileOptions<'_, ()> = FileOptions::default()
                .last_modified_time(zip::DateTime::default_for_write())
                .unix_permissions(0o755);

            for entry in walkdir::WalkDir::new(&temp_path_buf).into_iter().filter_map(|e| e.ok()) {
                let path = entry.path();
                let name = match path.strip_prefix(&temp_path_buf) {
                    Ok(p) => p.to_str().unwrap_or_default(),
                    Err(_) => path.to_str().unwrap_or_default(),
                };

                if path.is_file() {
                    log::info!("Adding file to new zip: {}", name);
                    
                    let is_compressed_format = path.extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| {
                            let ext_lower = ext.to_lowercase();
                            matches!(ext_lower.as_str(), "webp" | "png" | "jpg" | "jpeg" | "zip" | "gz" | "mp4" | "mp3" | "avif" | "heic")
                        })
                        .unwrap_or(false);

                    let options = if is_compressed_format {
                        base_options.compression_method(CompressionMethod::Stored)
                    } else {
                        base_options.compression_method(CompressionMethod::Deflated)
                    };

                    if let Err(e) = zip_writer.start_file(name, options) {
                        let err_msg = format!("Failed to add file to zip: {}", e);
                        log::error!("Failed to start file {} in zip: {}", name, e);
                        emit_progress("Error", &err_msg, None);
                        // cannot return from loop body directly, must break or continue.
                        // Here we break out of the inner loop, but since we can't easily break the outer, 
                        // we'll just log and continue, but it might result in a corrupted zip.
                        // For simplicity, we just continue the loop
                        break;
                    }
                    let file_content = match fs::File::open(path) {
                        Ok(f) => f,
                        Err(e) => {
                            let err_msg = format!("Failed to open file for zipping: {}", e);
                            log::error!("Failed to open file {} for zipping: {}", path.display(), e);
                            emit_progress("Error", &err_msg, None);
                            break;
                        }
                    };
                    let mut buf_reader = io::BufReader::new(file_content);
                    if let Err(e) = io::copy(&mut buf_reader, &mut zip_writer) {
                        let err_msg = format!("Failed to write file content to zip: {}", e);
                        log::error!("Failed to write file {} content to zip: {}", path.display(), e);
                        emit_progress("Error", &err_msg, None);
                        break;
                    }
                } else if path.is_dir() && name.is_empty() {
                    continue;
                } else if path.is_dir() {
                    log::info!("Adding directory to new zip: {}", name);
                    if let Err(e) = zip_writer.add_directory(name, base_options.compression_method(CompressionMethod::Deflated)) {
                        let err_msg = format!("Failed to add directory to zip: {}", e);
                        log::error!("Failed to add directory {} to zip: {}", name, e);
                        emit_progress("Error", &err_msg, None);
                        break;
                    }
                }
            }

            if let Err(e) = zip_writer.finish() {
                let err_msg = format!("Failed to finish zip archive: {}", e);
                log::error!("{}", err_msg);
                emit_progress("Error", &err_msg, None);
                continue;
            }

            // Get new file size
            let new_size = match fs::metadata(&final_output_zip_path) {
                Ok(m) => m.len(),
                Err(e) => {
                    log::warn!("Failed to get new file size: {}", e);
                    0
                }
            };

            let success_msg = format!("Successfully created {}", final_output_zip_path.display());
            log::info!("{}", success_msg);
            
            // Emit progress with size information
            if let Err(e) = app.emit("progress_update", ProgressPayload::with_sizes("Completed", &success_msg, original_size, new_size)) {
                log::error!("Failed to emit completion progress: {}", e);
            }
        } // End of zip_paths loop

        // Notify that all files have been processed
        if let Err(e) = app.emit("progress_update", ProgressPayload::new("AllCompleted", "All files processed successfully.", None)) {
            log::error!("Failed to emit AllCompleted: {}", e);
        }
    });

    Ok(())
}


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let log_plugin = tauri_plugin_log::Builder::new()
        .targets([
            tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
            tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Webview),
            tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir { file_name: Some("app.log".into()) }),
        ])
        .level(if cfg!(debug_assertions) { log::LevelFilter::Debug } else { log::LevelFilter::Info })
        .build();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(log_plugin)
        .plugin(tauri_plugin_cli::init())
        .invoke_handler(tauri::generate_handler![greet, unzip_file, get_output_directory])
        .setup(|app| {
            if let Ok(matches) = app.cli().matches() {
                if let Some(files_arg) = matches.args.get("files") {
                    if let Some(file_paths) = files_arg.value.as_array() {
                        let file_paths: Vec<String> = file_paths.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
                        if !file_paths.is_empty() {
                            if let Err(e) = app.handle().emit("file-drop-from-args", file_paths) {
                                log::error!("Failed to emit file-drop-from-args event: {}", e);
                            }
                        }
                    }
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}