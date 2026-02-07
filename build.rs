use sevenz_rust2;
use std::{
    env,
    path::Path,
};

fn main() {
    println!("cargo::rerun-if-changed=Cargo.toml");

    // 设置 EasyTier 版本
    let version = {
        let mut input = std::fs::read_to_string(
            Path::new(&get_var("CARGO_MANIFEST_DIR").unwrap()).join("Cargo.toml"),
        )
            .unwrap()
            .parse::<toml::Table>()
            .unwrap();

        for key in "package.metadata.easytier".split(".") {
            input = match input.into_iter().find(|(k, _)| k == key).unwrap().1 {
                toml::Value::Table(map) => map,
                _ => panic!("Expecting a table for key: {}", key),
            }
        }

        input.get("version").unwrap().as_str().unwrap().to_string()
    };
    println!("cargo::rustc-env=TERRACOTTA_ET_VERSION={}", version);

    sevenz_rust2::compress_to_path(
        "web",
        Path::new(&get_var("OUT_DIR").unwrap()).join("webstatics.7z"),
    )
    .unwrap();
    println!("cargo::rerun-if-changed=web");

    let desc = get_var("TARGET").unwrap().replace('-', "_").to_uppercase();

    let version = get_var("TERRACOTTA_VERSION").unwrap_or_else(|_| "ver. PCL.Proto 0.7.0".to_string());
    println!("cargo::rustc-env=TERRACOTTA_VERSION={}", version);

    let target_family = get_var("CARGO_CFG_TARGET_FAMILY").unwrap().to_string();
    if target_family == "windows" {
        println!("cargo::rerun-if-changed=build/windows/icon.ico");
        let mut compiler = winresource::WindowsResource::new();

        {
            let current = Path::new(&get_var("CARGO_MANIFEST_DIR").unwrap()).to_owned();

            if let Ok(windres) = get_var(&format!("CARGO_TARGET_{}_WINDRES_PATH", desc)) {
                let windres = current.join(windres);
                compiler.set_windres_path(windres.to_str().unwrap());
            }
            if let Ok(ar) = get_var(&format!("CARGO_TARGET_{}_AR", desc)) {
                let ar = current.join(ar);
                compiler.set_ar_path(ar.to_str().unwrap());
            }
        }

        for vs in ["FileVersion", "ProductVersion"] {
            compiler.set(vs, &version);
        }
        compiler.set_icon("build/windows/icon.ico");
        compiler.compile().unwrap();
    }

    for (key, value) in env::vars() {
        if key.starts_with("CARGO_CFG_") {
            println!("cargo::rustc-env={}={}", key, value);
        }
    }
}



pub fn get_var<K: AsRef<std::ffi::os_str::OsStr>>(key: K) -> Result<String, env::VarError> {
    println!(
        "cargo::rerun-if-env-changed={}",
        key.as_ref().to_string_lossy()
    );
    env::var(key.as_ref())
}
