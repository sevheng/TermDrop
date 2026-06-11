use std::{env, fs, path::Path};

fn main() {
    let target = env::var("TARGET").unwrap_or_default();

    // Map Rust target triples to the platform-specific binary directories.
    let (os, arch, ext) = match target.as_str() {
        "x86_64-unknown-linux-gnu" => ("linux", "x86_64", ""),
        "aarch64-unknown-linux-gnu" => ("linux", "aarch64", ""),
        "x86_64-apple-darwin" => ("macos", "x86_64", ""),
        "aarch64-apple-darwin" => ("macos", "aarch64", ""),
        "x86_64-pc-windows-msvc" => ("windows", "x86_64", ".exe"),
        "aarch64-pc-windows-msvc" => ("windows", "aarch64", ".exe"),
        _ => {
            println!(
                "cargo:warning=Unsupported target '{}'. Bundled MongoDB tools will not be available.",
                target
            );
            tauri_build::build();
            return;
        }
    };

    let platform_dir = format!("{}-{}", os, arch);
    let config_name = match os {
        "linux" => "tauri.linux.conf.json",
        "macos" => "tauri.macos.conf.json",
        "windows" => "tauri.windows.conf.json",
        _ => unreachable!(),
    };

    // Verify the source binaries exist so the build fails early with a clear message.
    let dump_src = format!("binaries/{}/mongodump{}", platform_dir, ext);
    let restore_src = format!("binaries/{}/mongorestore{}", platform_dir, ext);
    for src in [&dump_src, &restore_src] {
        if !Path::new(src).exists() {
            panic!(
                "Bundled MongoDB tool not found for target {}: {}",
                target, src
            );
        }
    }

    // Generate the platform-specific Tauri config overlay that bundles only the
    // correct MongoDB tools for the current target.
    let config = format!(
        "{{\n  \"bundle\": {{\n    \"resources\": {{\n      \"{}\": \"mongodump{}\",\n      \"{}\": \"mongorestore{}\"\n    }}\n  }}\n}}\n",
        dump_src, ext, restore_src, ext
    );

    fs::write(config_name, config).expect("failed to write platform Tauri config");

    tauri_build::build();
}
