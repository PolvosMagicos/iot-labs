use std::{env, time::Duration};

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use p256::{ecdh::diffie_hellman, elliptic_curve::sec1::ToSec1Point, PublicKey, SecretKey};

use rumqttc::{AsyncClient, MqttOptions, QoS};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Serialize)]
struct SensorReading {
    usuario: String,
    temperatura: f32,
    humedad: f32,
    calidad_aire: f32,
    device_timestamp_unix: Option<i64>,
}

#[derive(Serialize)]
struct EncryptedPayload {
    version: u8,
    ephemeral_public_key: String,
    nonce: String,
    ciphertext: String,
}

fn generate_secret_key() -> Result<SecretKey> {
    loop {
        let mut secret_bytes = [0_u8; 32];

        getrandom::fill(&mut secret_bytes)
            .map_err(|error| anyhow!("failed to obtain random bytes: {error}"))?;

        if let Ok(secret_key) = SecretKey::from_slice(&secret_bytes) {
            return Ok(secret_key);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let host = env::var("MQTT_HOST").unwrap_or_else(|_| "broker.hivemq.com".into());
    let port = env::var("MQTT_PORT")
        .unwrap_or_else(|_| "1883".into())
        .parse::<u16>()
        .context("MQTT_PORT must be a valid u16")?;
    let topic = env::var("MQTT_TOPIC").context("missing MQTT_TOPIC")?;
    let public_key = parse_public_key(
        &env::var("SERVER_PUBLIC_KEY_B64").context("missing SERVER_PUBLIC_KEY_B64")?,
    )?;

    let args: Vec<String> = env::args().collect();
    let temperature = parse_arg(&args, 1, 40.0)?;
    let humidity = parse_arg(&args, 2, 90.0)?;
    let air_quality = parse_arg(&args, 3, 130.0)?;

    let reading = SensorReading {
        usuario: "Rust_Desktop_Simulator".into(),
        temperatura: temperature,
        humedad: humidity,
        calidad_aire: air_quality,
        device_timestamp_unix: Some(Utc::now().timestamp()),
    };

    let encrypted = encrypt(&public_key, &reading)?;
    let payload = serde_json::to_vec(&encrypted)?;

    let mut options =
        MqttOptions::new(format!("rust-simulator-{}", std::process::id()), host, port);
    options.set_keep_alive(Duration::from_secs(10));

    let (client, mut event_loop) = AsyncClient::new(options, 10);
    let event_task = tokio::spawn(async move {
        loop {
            if event_loop.poll().await.is_err() {
                break;
            }
        }
    });

    client
        .publish(&topic, QoS::AtLeastOnce, false, payload)
        .await
        .context("cannot publish simulator message")?;

    tokio::time::sleep(Duration::from_secs(2)).await;
    event_task.abort();

    println!(
        "Encrypted test published: temperature={temperature}, humidity={humidity}, air_quality={air_quality}"
    );
    Ok(())
}

fn parse_arg(args: &[String], index: usize, default: f32) -> Result<f32> {
    match args.get(index) {
        Some(value) => value
            .parse::<f32>()
            .with_context(|| format!("argument {index} must be numeric")),
        None => Ok(default),
    }
}

fn parse_public_key(encoded: &str) -> Result<PublicKey> {
    let bytes = STANDARD
        .decode(encoded)
        .context("SERVER_PUBLIC_KEY_B64 is not valid Base64")?;
    PublicKey::from_sec1_bytes(&bytes).context("invalid P-256 public key")
}

fn encrypt(server_public_key: &PublicKey, reading: &SensorReading) -> Result<EncryptedPayload> {
    let ephemeral_private_key = generate_secret_key()?;
    let ephemeral_public_key = ephemeral_private_key.public_key();

    let shared_secret = diffie_hellman(
        ephemeral_private_key.to_nonzero_scalar(),
        server_public_key.as_affine(),
    );

    let aes_key = Sha256::digest(shared_secret.raw_secret_bytes());
    let cipher = Aes256Gcm::new_from_slice(&aes_key).map_err(|_| anyhow!("invalid AES key"))?;

    let mut nonce_bytes = [0_u8; 12];

    getrandom::fill(&mut nonce_bytes)
        .map_err(|error| anyhow!("failed to generate AES-GCM nonce: {error}"))?;

    let nonce = Nonce::try_from(nonce_bytes.as_slice())
        .map_err(|_| anyhow!("AES-GCM nonce must contain exactly 12 bytes"))?;

    let plaintext = serde_json::to_vec(reading)?;
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_ref())
        .map_err(|_| anyhow!("AES-GCM encryption failed"))?;

    Ok(EncryptedPayload {
        version: 1,
        ephemeral_public_key: STANDARD.encode(ephemeral_public_key.to_sec1_point(false).as_bytes()),
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
    })
}
