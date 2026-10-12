use std::{
    ffi::c_void,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::peripherals::Peripherals,
    mqtt::client::{EspMqttClient, EventPayload, MqttClientConfiguration, QoS},
    nvs::EspDefaultNvsPartition,
    sys::{esp_fill_random, esp_random},
    wifi::{BlockingWifi, ClientConfiguration, Configuration, EspWifi},
};
use log::{error, info, warn};
use p256::{ecdh::diffie_hellman, elliptic_curve::sec1::ToSec1Point, PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASS: &str = env!("WIFI_PASS");
const MQTT_URL: &str = env!("MQTT_URL");
const MQTT_TOPIC: &str = env!("MQTT_TOPIC");
const MQTT_CLIENT_ID: &str = env!("MQTT_CLIENT_ID");
const DEVICE_USER: &str = env!("DEVICE_USER");
const SERVER_PUBLIC_KEY_B64: &str = env!("SERVER_PUBLIC_KEY_B64");

const SAMPLE_INTERVAL_SECONDS: u64 = match option_env!("SAMPLE_INTERVAL_SECONDS") {
    Some(value) => parse_u64_const(value),
    None => 300,
};

const fn parse_u64_const(value: &str) -> u64 {
    let bytes = value.as_bytes();
    let mut index = 0;
    let mut number = 0_u64;

    while index < bytes.len() {
        let digit = bytes[index];
        if digit < b'0' || digit > b'9' {
            panic!("SAMPLE_INTERVAL_SECONDS must be numeric");
        }
        number = number * 10 + (digit - b'0') as u64;
        index += 1;
    }

    number
}

#[derive(Debug, Serialize, Deserialize)]
struct SensorReading {
    usuario: String,
    temperatura: f32,
    humedad: f32,
    calidad_aire: f32,
    device_timestamp_unix: Option<i64>,
}

#[derive(Debug, Serialize)]
struct EncryptedPayload {
    version: u8,
    ephemeral_public_key: String,
    nonce: String,
    ciphertext: String,
}

fn fill_random(destination: &mut [u8]) {
    unsafe {
        esp_fill_random(destination.as_mut_ptr().cast::<c_void>(), destination.len());
    }
}

fn generate_ephemeral_secret_key() -> SecretKey {
    loop {
        let mut secret_bytes = [0_u8; 32];
        fill_random(&mut secret_bytes);

        if let Ok(secret_key) = SecretKey::from_slice(&secret_bytes) {
            return secret_key;
        }
    }
}

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    info!("Starting secure environmental IoT node");

    let _wifi = connect_wifi()?;

    info!("Parsing server P-256 public key...");
    let server_public_key = parse_server_public_key()?;
    info!("Server public key parsed");

    info!("Creating MQTT client...");
    let mut mqtt = EspMqttClient::new_cb(
        MQTT_URL,
        &MqttClientConfiguration {
            client_id: Some(MQTT_CLIENT_ID),
            ..Default::default()
        },
        |event| match event.payload() {
            EventPayload::Connected(_) => info!("MQTT connected"),
            EventPayload::Disconnected => warn!("MQTT disconnected"),
            EventPayload::Error(error) => error!("MQTT error: {error:?}"),
            _ => {}
        },
    )
    .context("cannot create MQTT client")?;

    info!("MQTT client created");

    thread::sleep(Duration::from_secs(2));

    loop {
        let reading = read_environment();
        let payload = encrypt_reading(&server_public_key, &reading)?;
        let json = serde_json::to_vec(&payload)?;

        mqtt.enqueue(MQTT_TOPIC, QoS::AtLeastOnce, false, &json)
            .context("cannot enqueue MQTT message")?;

        info!(
            "Encrypted reading sent: temp={:.2}, humidity={:.2}, air={:.2}",
            reading.temperatura, reading.humedad, reading.calidad_aire
        );

        thread::sleep(Duration::from_secs(SAMPLE_INTERVAL_SECONDS));
    }
}

fn connect_wifi() -> Result<EspWifi<'static>> {
    let peripherals = Peripherals::take().context("peripherals already taken")?;
    let system_event_loop =
        EspSystemEventLoop::take().context("system event loop already taken")?;
    let nvs = EspDefaultNvsPartition::take().context("default NVS already taken")?;

    let mut esp_wifi = EspWifi::new(peripherals.modem, system_event_loop.clone(), Some(nvs))?;

    {
        let mut wifi = BlockingWifi::wrap(&mut esp_wifi, system_event_loop)?;

        wifi.set_configuration(&Configuration::Client(ClientConfiguration {
            ssid: WIFI_SSID
                .try_into()
                .map_err(|_| anyhow!("WIFI_SSID is too long"))?,
            password: WIFI_PASS
                .try_into()
                .map_err(|_| anyhow!("WIFI_PASS is too long"))?,
            ..Default::default()
        }))?;

        wifi.start()?;
        info!("Wi-Fi started");

        wifi.connect()?;
        info!("Wi-Fi connected");

        wifi.wait_netif_up()?;
        info!("Network interface is up");
    }

    Ok(esp_wifi)
}

fn parse_server_public_key() -> Result<PublicKey> {
    let bytes = STANDARD
        .decode(SERVER_PUBLIC_KEY_B64)
        .context("SERVER_PUBLIC_KEY_B64 is not valid Base64")?;

    PublicKey::from_sec1_bytes(&bytes)
        .context("SERVER_PUBLIC_KEY_B64 is not an uncompressed P-256 public key")
}

fn encrypt_reading(
    server_public_key: &PublicKey,
    reading: &SensorReading,
) -> Result<EncryptedPayload> {
    let ephemeral_private_key = generate_ephemeral_secret_key();
    let ephemeral_public_key = ephemeral_private_key.public_key();

    let shared_secret = diffie_hellman(
        ephemeral_private_key.to_nonzero_scalar(),
        server_public_key.as_affine(),
    );

    let aes_key = Sha256::digest(shared_secret.raw_secret_bytes());
    let cipher = Aes256Gcm::new_from_slice(&aes_key).map_err(|_| anyhow!("invalid AES key"))?;

    let mut nonce_bytes = [0_u8; 12];
    fill_random(&mut nonce_bytes);

    let nonce = Nonce::try_from(nonce_bytes.as_slice())
        .map_err(|_| anyhow!("AES-GCM nonce must contain exactly 12 bytes"))?;

    let plaintext = serde_json::to_vec(reading)?;
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_ref())
        .map_err(|_| anyhow!("AES-GCM encryption failed"))?;

    let encoded_point = ephemeral_public_key.to_sec1_point(false);

    Ok(EncryptedPayload {
        version: 1,
        ephemeral_public_key: STANDARD.encode(encoded_point.as_bytes()),
        nonce: STANDARD.encode(nonce_bytes),
        ciphertext: STANDARD.encode(ciphertext),
    })
}

fn read_environment() -> SensorReading {
    SensorReading {
        usuario: DEVICE_USER.to_owned(),
        temperatura: random_between(20.0, 45.0),
        humedad: random_between(40.0, 95.0),
        calidad_aire: random_between(10.0, 150.0),
        device_timestamp_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_secs() as i64)
            .filter(|timestamp| *timestamp > 1_700_000_000),
    }
}

fn random_between(minimum: f32, maximum: f32) -> f32 {
    let raw = unsafe { esp_random() };
    let ratio = raw as f64 / u32::MAX as f64;
    (minimum as f64 + ratio * (maximum - minimum) as f64) as f32
}
