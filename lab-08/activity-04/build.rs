use std::{env, path::PathBuf};

fn main() {
    embuild::espidf::sysenv::output();

    let root_env = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .expect("activity folder should have a parent")
        .join(".env");

    println!("cargo:rerun-if-changed={}", root_env.display());

    let file_values = if root_env.exists() {
        dotenvy::from_path_iter(&root_env)
            .expect("failed to read root .env")
            .map(|item| item.expect("invalid .env entry"))
            .collect::<std::collections::HashMap<_, _>>()
    } else {
        std::collections::HashMap::new()
    };

    for key in [
        "WIFI_SSID",
        "WIFI_PASS",
        "MQTT_BROKER",
        "ECC_PUBLIC_KEY_HEX",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
        let value = env::var(key)
            .ok()
            .or_else(|| file_values.get(key).cloned())
            .unwrap_or_else(|| {
                panic!(
                    "{key} is missing. Set it in the environment or copy {} to {}",
                    root_env.with_file_name(".env.example").display(),
                    root_env.display()
                )
            });

        if key == "ECC_PUBLIC_KEY_HEX" {
            let value = value.trim().strip_prefix("04").unwrap_or(value.trim());
            assert!(
                value.len() == 128 && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "ECC_PUBLIC_KEY_HEX must contain exactly 64 P-256 X/Y bytes (128 hex chars)"
            );
            println!("cargo:rustc-env={key}={value}");
        } else {
            println!("cargo:rustc-env={key}={value}");
        }
    }
}
