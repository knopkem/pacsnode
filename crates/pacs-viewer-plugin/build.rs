use std::{
    env,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use walkdir::WalkDir;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let viewer_dir = manifest_dir.join("../../web/viewer");
    let viewer_dir = viewer_dir.canonicalize().map_err(|error| {
        format!(
            "failed to locate embedded viewer assets at {}: {error}",
            viewer_dir.display()
        )
    })?;

    let files = collect_viewer_files(&viewer_dir)?;
    if !files
        .iter()
        .any(|path| path.file_name().is_some_and(|name| name == "index.html"))
    {
        return Err(format!(
            "embedded viewer assets under {} do not contain index.html",
            viewer_dir.display()
        )
        .into());
    }

    println!("cargo:rerun-if-changed={}", viewer_dir.display());
    for path in WalkDir::new(&viewer_dir).into_iter().filter_map(Result::ok) {
        println!("cargo:rerun-if-changed={}", path.path().display());
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let archive_path = out_dir.join("embedded-viewer.zip");
    write_archive(&viewer_dir, &files, &archive_path)?;

    let archive_bytes = fs::read(&archive_path)?;
    let mut hasher = Sha256::new();
    hasher.update(&archive_bytes);
    let hash = hex_string(&hasher.finalize());

    println!("cargo:rustc-env=PACSNODE_EMBEDDED_VIEWER_BUNDLE_HASH={hash}");
    Ok(())
}

fn collect_viewer_files(root: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut files = WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| !is_ignored_entry(entry.path()))
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            entry
                .path()
                .strip_prefix(root)
                .map(Path::to_path_buf)
                .map_err(|error| error.into())
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;

    files.sort();
    Ok(files)
}

fn is_ignored_entry(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

fn write_archive(
    root: &Path,
    files: &[PathBuf],
    archive_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::create(archive_path)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    for relative_path in files {
        let archive_name = relative_path.to_string_lossy().replace('\\', "/");
        zip.start_file(&archive_name, options)?;

        let mut source = File::open(root.join(relative_path))?;
        let mut buffer = Vec::new();
        source.read_to_end(&mut buffer)?;
        zip.write_all(&buffer)?;
    }

    zip.finish()?;
    Ok(())
}

fn hex_string(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
