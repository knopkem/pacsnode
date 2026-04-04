use std::{
    env,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha256};
use walkdir::WalkDir;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let frontend_dir = manifest_dir.join("../../web/pacsleaf-viewer");
    let frontend_dir = frontend_dir.canonicalize().map_err(|error| {
        format!(
            "failed to locate pacsleaf viewer frontend at {}: {error}",
            frontend_dir.display()
        )
    })?;
    let dist_dir = frontend_dir.join("dist");

    println!("cargo:rerun-if-env-changed=PACSNODE_SKIP_PACSLEAF_WEB_BUILD");
    println!("cargo:rerun-if-env-changed=PACSNODE_PACSLEAF_NPM");
    print_rerun_instructions(&frontend_dir)?;

    if env::var_os("PACSNODE_SKIP_PACSLEAF_WEB_BUILD").is_none() {
        ensure_frontend_build(&frontend_dir)?;
    }

    if !dist_dir.join("index.html").is_file() {
        return Err(format!(
            "built pacsleaf viewer assets are missing index.html under {}",
            dist_dir.display()
        )
        .into());
    }

    let files = collect_viewer_files(&dist_dir)?;
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let archive_path = out_dir.join("embedded-pacsleaf-viewer.zip");
    write_archive(&dist_dir, &files, &archive_path)?;

    let archive_bytes = fs::read(&archive_path)?;
    let mut hasher = Sha256::new();
    hasher.update(&archive_bytes);
    let hash = hex_string(&hasher.finalize());

    println!("cargo:rustc-env=PACSNODE_EMBEDDED_PACSLEAF_VIEWER_BUNDLE_HASH={hash}");
    Ok(())
}

fn print_rerun_instructions(frontend_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for relative_path in [
        "package.json",
        "package-lock.json",
        "tsconfig.json",
        "tsconfig.app.json",
        "tsconfig.node.json",
        "vite.config.ts",
        "tailwind.config.js",
        "postcss.config.js",
        "eslint.config.js",
        "index.html",
    ] {
        let path = frontend_dir.join(relative_path);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    print_rerun_for_dir(&frontend_dir.join("src"))?;
    print_rerun_for_dir(&frontend_dir.join("public"))?;
    Ok(())
}

fn print_rerun_for_dir(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(());
    }

    println!("cargo:rerun-if-changed={}", path.display());
    for entry in WalkDir::new(path).into_iter().filter_map(Result::ok) {
        println!("cargo:rerun-if-changed={}", entry.path().display());
    }
    Ok(())
}

fn ensure_frontend_build(frontend_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let node_modules = frontend_dir.join("node_modules");
    if !node_modules.is_dir() {
        run_npm(frontend_dir, &["ci", "--no-audit", "--no-fund"])?;
    } else if !has_required_rolldown_binding(&node_modules) {
        println!(
            "cargo:warning=pacsleaf-viewer node_modules is missing the host rolldown binding; running npm ci"
        );
        run_npm(frontend_dir, &["ci", "--no-audit", "--no-fund"])?;
    }

    run_npm(frontend_dir, &["run", "build"])
}

fn has_required_rolldown_binding(node_modules: &Path) -> bool {
    let Some(package_name) = required_rolldown_binding_package() else {
        return true;
    };

    node_modules
        .join("@rolldown")
        .join(package_name)
        .join("package.json")
        .is_file()
}

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
fn required_rolldown_binding_package() -> Option<&'static str> {
    Some("binding-linux-x64-gnu")
}

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "musl"))]
fn required_rolldown_binding_package() -> Option<&'static str> {
    Some("binding-linux-x64-musl")
}

#[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
fn required_rolldown_binding_package() -> Option<&'static str> {
    Some("binding-linux-arm64-gnu")
}

#[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "musl"))]
fn required_rolldown_binding_package() -> Option<&'static str> {
    Some("binding-linux-arm64-musl")
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn required_rolldown_binding_package() -> Option<&'static str> {
    Some("binding-darwin-x64")
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn required_rolldown_binding_package() -> Option<&'static str> {
    Some("binding-darwin-arm64")
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn required_rolldown_binding_package() -> Option<&'static str> {
    Some("binding-win32-x64-msvc")
}

#[cfg(all(target_os = "windows", target_arch = "aarch64"))]
fn required_rolldown_binding_package() -> Option<&'static str> {
    Some("binding-win32-arm64-msvc")
}

#[cfg(not(any(
    all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"),
    all(target_os = "linux", target_arch = "x86_64", target_env = "musl"),
    all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"),
    all(target_os = "linux", target_arch = "aarch64", target_env = "musl"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "windows", target_arch = "aarch64")
)))]
fn required_rolldown_binding_package() -> Option<&'static str> {
    None
}

fn run_npm(frontend_dir: &Path, args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let npm = npm_command();
    let status = Command::new(&npm)
        .current_dir(frontend_dir)
        .args(args)
        .status()
        .map_err(|error| {
            format!(
                "failed to run `{}` with args `{}` in {}: {error}",
                npm,
                args.join(" "),
                frontend_dir.display()
            )
        })?;

    if !status.success() {
        return Err(format!(
            "`{} {}` failed with status {status}",
            npm,
            args.join(" ")
        )
        .into());
    }

    Ok(())
}

fn npm_command() -> String {
    env::var("PACSNODE_PACSLEAF_NPM").unwrap_or_else(|_| default_npm_command().into())
}

#[cfg(target_os = "windows")]
fn default_npm_command() -> &'static str {
    "npm.cmd"
}

#[cfg(not(target_os = "windows"))]
fn default_npm_command() -> &'static str {
    "npm"
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
