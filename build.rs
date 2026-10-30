use std::{env, path::PathBuf, process::Command};

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") {
        return;
    }

    println!("cargo:rerun-if-changed=resources/logo.ico");

    // PNG sources are not needed at build time: the multi-size .ico is
    // committed under resources/.
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let ico = manifest_dir.join("resources").join("logo.ico");
    if !ico.is_file() {
        println!("cargo:warning=resources/logo.ico not found; exe icon not embedded");
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let rc = out_dir.join("app.rc");
    std::fs::write(&rc, gen_rc(&ico)).expect("failed to write app.rc");

    let tool = match env::var("WINDOWRES") {
        Ok(custom) if !custom.trim().is_empty() => custom,
        _ => {
            let host = env::var("HOST").unwrap_or_default();
            // The rustc target triple is x86_64-pc-windows-gnu, but the
            // mingw-w64 binutils prefix is x86_64-w64-mingw32.
            let candidates = if target == host || host.is_empty() {
                vec![String::from("windres")]
            } else {
                vec![
                    String::from("x86_64-w64-mingw32-windres"),
                    String::from("windres"),
                ]
            };
            match candidates
                .iter()
                .find(|t| Command::new(t).arg("--version").output().is_ok())
            {
                Some(t) => t.clone(),
                None => {
                    println!(
                        "cargo:warning=windres not found (set WINDOWRES); exe icon not embedded"
                    );
                    return;
                }
            }
        }
    };

    let obj = out_dir.join("app.res.o");
    let status = Command::new(&tool)
        .arg("-O")
        .arg("coff")
        .arg("-i")
        .arg(&rc)
        .arg("-o")
        .arg(&obj)
        .status()
        .expect("failed to spawn windres");
    if !status.success() || !obj.is_file() {
        println!("cargo:warning={tool} failed; exe icon not embedded");
        return;
    }

    println!("cargo:rustc-link-arg-bins={}", obj.display());
}

fn gen_rc(ico: &std::path::Path) -> String {
    // Path to the icon: windres handles windows paths only; convert the
    // separators when running on the toolchain's shell side. Absolute paths
    // with forward slashes are accepted by windres as-is.
    let ico_str = ico.to_string_lossy().replace('\\', "/");

    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".to_string());
    let parts: Vec<String> = version
        .split('.')
        .map(|s| s.split(['+', '-']).next().unwrap_or("0").to_string())
        .collect();
    let v = |i: usize| parts.get(i).cloned().unwrap_or_else(|| "0".to_string());
    let fv = format!("{},{},{},{}", v(0), v(1), v(2), "0");

    format!(
        r#"1 ICON "{ico_str}"

1 VERSIONINFO
FILEVERSION     {fv}
PRODUCTVERSION  {fv}
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904E4"
    BEGIN
      VALUE "FileDescription", "ngasak: ngate runner + Sakura 2FA"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "ngasak"
      VALUE "ProductName", "ngasak"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1252
  END
END
"#
    )
}
