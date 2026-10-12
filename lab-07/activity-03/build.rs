use std::{env, path::PathBuf};

fn main() {
    embuild::espidf::sysenv::output();

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    // activity-N/../.env -> lab-07/.env
    let root_env = manifest_dir
        .parent()
        .expect("activity folder should have a parent")
        .join(".env");

    println!("cargo:rerun-if-changed={}", root_env.display());

    if !root_env.exists() {
        panic!("Missing root .env file at {}", root_env.display());
    }

    for item in dotenvy::from_path_iter(&root_env).expect("failed to read root .env") {
        let (key, value) = item.expect("invalid .env entry");

        match key.as_str() {
            "WIFI_SSID" | "WIFI_PASS" | "MQTT_BROKER" => {
                println!("cargo:rustc-env={}={}", key, value);
            }
            _ => {}
        }
    }
}
