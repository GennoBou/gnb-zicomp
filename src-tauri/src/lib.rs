use std::fs;
use std::io;
use std::path::PathBuf;
use tempfile::tempdir;
use zip::ZipArchive;
use zip::write::{FileOptions, ZipWriter};
use zip::CompressionMethod;
use tauri::{Manager, Emitter};
use webp::{Encoder, PixelLayout};
use rayon::prelude::*;

#[derive(Clone, serde::Serialize)]
struct ProgressPayload {
    step: String,
    status: String,
    output_path: Option<String>,
}

impl ProgressPayload {
    fn new(step: &str, status: &str, output_path: Option<String>) -> Self {
        Self {
            step: step.to_string(),
            status: status.to_string(),
            output_path,
        }
    }
}

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn unzip_file(app: tauri::AppHandle, zip_path: &str) -> Result<(), String> {
    let app_handle = app.clone();
    let emit_progress = move |step: &str, status: &str, output_path: Option<String>| {
        if let Err(e) = app_handle.emit("progress_update", ProgressPayload::new(step, status, output_path)) {
            log::error!("Failed to emit progress update: {}", e);
        }
    };

    log::info!("Attempting to unzip file: {}", zip_path);

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
                    return Err(err_msg);
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
            return Err(err_msg);
        }
    };

    let temp_path_buf = temp_dir.path().to_path_buf();
    let temp_path_str = match temp_path_buf.to_str() {
        Some(s) => s.to_string(),
        None => {
            let err_msg = "Temporary path contains invalid UTF-8".to_string();
            log::error!("{}", err_msg);
            emit_progress("Error", &err_msg, None);
            return Err(err_msg);
        }
    };

    log::info!("Temporary directory created at: {}", temp_path_str);
    emit_progress("Unzipping", "Extracting files...", None);

    // Open the ZIP file
    let file = match fs::File::open(zip_path) {
        Ok(f) => f,
        Err(e) => {
            let err_msg = format!("Failed to open zip file: {}", e);
            log::error!("Failed to open zip file '{}': {}", zip_path, e);
            emit_progress("Error", &err_msg, None);
            return Err(err_msg);
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
            return Err(err_msg);
        }
    };

    // Extract all files
    if let Err(e) = archive.extract(temp_dir.path()) {
        let err_msg = format!("Failed to extract zip archive: {}", e);
        log::error!("{}", err_msg);
        emit_progress("Error", &err_msg, None);
        return Err(err_msg);
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
                let rgba_image = img.to_rgba8();
                let (width, height) = rgba_image.dimensions();
                let encoder = Encoder::new(&rgba_image, PixelLayout::Rgba, width, height);
                let encoded_webp = encoder.encode(94.0);

                match fs::write(&output_path, &*encoded_webp) {
                    Ok(_) => {
                        log::info!("Converted {} to {}", original_path.display(), output_path.display());
                        if let Err(e) = fs::remove_file(&original_path) {
                            log::warn!("Failed to delete original image {}: {}", original_path.display(), e);
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to save  image {}: {}", output_path.display(), e);
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
    let original_zip_filename = PathBuf::from(zip_path)
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
            return Err(err_msg);
        }
    };

    let mut zip_writer = ZipWriter::new(output_file);
    let options: FileOptions<'_, ()> = FileOptions::default()
        .compression_method(CompressionMethod::Deflated)
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
            if let Err(e) = zip_writer.start_file(name, options) {
                let err_msg = format!("Failed to add file to zip: {}", e);
                log::error!("Failed to start file {} in zip: {}", name, e);
                emit_progress("Error", &err_msg, None);
                return Err(err_msg);
            }
            let mut file_content = match fs::File::open(path) {
                Ok(f) => f,
                Err(e) => {
                    let err_msg = format!("Failed to open file for zipping: {}", e);
                    log::error!("Failed to open file {} for zipping: {}", path.display(), e);
                    emit_progress("Error", &err_msg, None);
                    return Err(err_msg);
                }
            };
            if let Err(e) = io::copy(&mut file_content, &mut zip_writer) {
                let err_msg = format!("Failed to write file content to zip: {}", e);
                log::error!("Failed to write file {} content to zip: {}", path.display(), e);
                emit_progress("Error", &err_msg, None);
                return Err(err_msg);
            }
        } else if path.is_dir() && name.is_empty() {
            continue;
        } else if path.is_dir() {
            log::info!("Adding directory to new zip: {}", name);
            if let Err(e) = zip_writer.add_directory(name, options) {
                let err_msg = format!("Failed to add directory to zip: {}", e);
                log::error!("Failed to add directory {} to zip: {}", name, e);
                emit_progress("Error", &err_msg, None);
                return Err(err_msg);
            }
        }
    }

    if let Err(e) = zip_writer.finish() {
        let err_msg = format!("Failed to finish zip archive: {}", e);
        log::error!("{}", err_msg);
        emit_progress("Error", &err_msg, None);
        return Err(err_msg);
    }

    let success_msg = format!("Successfully created {}", final_output_zip_path.display());
    log::info!("{}", success_msg);
    emit_progress("Completed", &success_msg, Some(output_dir.display().to_string()));

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
        .invoke_handler(tauri::generate_handler![greet, unzip_file])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}