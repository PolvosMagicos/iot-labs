use std::{
    collections::HashSet,
    env,
    path::PathBuf,
};

const REQUIRED_VARIABLES: &[&str] = &[
    "WIFI_SSID",
    "WIFI_PASS",
    "MQTT_URL",
    "MQTT_TOPIC",
    "MQTT_CLIENT_ID",
    "DEVICE_USER",
    "SERVER_PUBLIC_KEY_B64",
];

const OPTIONAL_VARIABLES: &[&str] = &[
    "SAMPLE_INTERVAL_SECONDS",
];

fn main() {
    // Required by esp-idf-sys.
    embuild::espidf::sysenv::output();

    let manifest_dir = PathBuf::from(
        env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR should be defined by Cargo"),
    );

    // esp32-node/.env
    let env_path = manifest_dir.join(".env");

    println!("cargo:rerun-if-changed={}", env_path.display());

    if !env_path.exists() {
        panic!(
            "Missing ESP32 environment file at {}. Copy .env.example to .env.",
            env_path.display()
        );
    }

    let allowed_variables: HashSet<&str> = REQUIRED_VARIABLES
        .iter()
        .chain(OPTIONAL_VARIABLES.iter())
        .copied()
        .collect();

    let mut loaded_variables = HashSet::new();

    let entries = dotenvy::from_path_iter(&env_path)
        .unwrap_or_else(|error| {
            panic!(
                "Failed to read {}: {error}",
                env_path.display()
            )
        });

    for entry in entries {
        let (key, value) = entry.unwrap_or_else(|error| {
            panic!(
                "Invalid entry in {}: {error}",
                env_path.display()
            )
        });

        if allowed_variables.contains(key.as_str()) {
            println!("cargo:rustc-env={key}={value}");
            loaded_variables.insert(key);
        }
    }

    for variable in REQUIRED_VARIABLES {
        if !loaded_variables.contains(*variable) {
            panic!(
                "Missing required variable {variable} in {}",
                env_path.display()
            );
        }
    }

    // Default to five minutes when the variable is omitted.
    if !loaded_variables.contains("SAMPLE_INTERVAL_SECONDS") {
        println!("cargo:rustc-env=SAMPLE_INTERVAL_SECONDS=300");
    }
}
