use aes::Aes128;
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use bme280::i2c::BME280;
use ctr::cipher::{KeyIvInit, StreamCipher};
use embedded_svc::mqtt::client::QoS;
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_hal::adc::attenuation::DB_12;
use esp_idf_hal::adc::oneshot::config::AdcChannelConfig;
use esp_idf_hal::adc::oneshot::{AdcChannelDriver, AdcDriver};
use esp_idf_hal::delay::{Ets, FreeRtos};
use esp_idf_hal::gpio::{InputOutput, PinDriver, Pull};
use esp_idf_hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::mqtt::client::{EspMqttClient, MqttClientConfiguration};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use log::{info, warn};
use p256::ecdh::EphemeralSecret;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::PublicKey;
use rand_core::{CryptoRng, RngCore};
use serde_json::json;
use sha2::{Digest, Sha256};

const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASS: &str = env!("WIFI_PASS");
const MQTT_BROKER: &str = env!("MQTT_BROKER");
const ECC_PUBLIC_KEY_HEX: &str = env!("ECC_PUBLIC_KEY_HEX");

const CLIENT_ID: &str = "esp32-g1-rust-ecc";
const TOPIC_ENCRYPTED: &str = "unmsm/iot2026/lab-08/g1/sensores/json_enc";
const TOPIC_ALARM: &str = "unmsm/iot2026/lab-08/g1/alarmas/gas";
const TOPIC_STATUS: &str = "unmsm/iot2026/lab-08/g1/system/status";
const GAS_THRESHOLD: u16 = 2500;
const PUBLISH_INTERVAL_MS: u32 = 10_000;

type Aes128Ctr = ctr::Ctr128BE<Aes128>;

struct EspRng;

impl RngCore for EspRng {
    fn next_u32(&mut self) -> u32 {
        let mut value = [0; 4];
        self.fill_bytes(&mut value);
        u32::from_ne_bytes(value)
    }

    fn next_u64(&mut self) -> u64 {
        let mut value = [0; 8];
        self.fill_bytes(&mut value);
        u64::from_ne_bytes(value)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        unsafe {
            esp_idf_svc::sys::esp_fill_random(dest.as_mut_ptr().cast(), dest.len());
        }
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> core::result::Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl CryptoRng for EspRng {}

fn micros() -> i64 {
    unsafe { esp_idf_svc::sys::esp_timer_get_time() }
}

fn read_dht22(dht: &mut PinDriver<'_, InputOutput>) -> Result<(f32, f32)> {
    macro_rules! wait_level {
        ($want_high:expr, $timeout_us:expr) => {{
            let start = micros();
            loop {
                if dht.is_high() == $want_high {
                    break;
                }
                if micros() - start > $timeout_us {
                    bail!("DHT22 timeout waiting for level {}", $want_high);
                }
            }
        }};
    }

    dht.set_low()?;
    Ets::delay_us(1200);
    dht.set_high()?;
    Ets::delay_us(40);
    wait_level!(false, 100);
    wait_level!(true, 100);
    wait_level!(false, 100);

    let mut data = [0_u8; 5];
    for bit_index in 0..40 {
        wait_level!(true, 100);
        let high_start = micros();
        wait_level!(false, 120);
        if micros() - high_start > 45 {
            data[bit_index / 8] |= 1 << (7 - bit_index % 8);
        }
    }

    let checksum = data[0]
        .wrapping_add(data[1])
        .wrapping_add(data[2])
        .wrapping_add(data[3]);
    if checksum != data[4] {
        bail!("DHT22 checksum error: expected {checksum}, got {}", data[4]);
    }

    let humidity = u16::from_be_bytes([data[0], data[1]]) as f32 / 10.0;
    let raw_temperature = u16::from_be_bytes([data[2] & 0x7f, data[3]]) as f32 / 10.0;
    let temperature = if data[2] & 0x80 == 0 {
        raw_temperature
    } else {
        -raw_temperature
    };

    Ok((temperature, humidity))
}

fn recipient_public_key() -> Result<PublicKey> {
    let xy = hex::decode(ECC_PUBLIC_KEY_HEX).context("invalid ECC public key hex")?;
    let mut sec1 = [0_u8; 65];
    sec1[0] = 0x04;
    sec1[1..].copy_from_slice(&xy);
    PublicKey::from_sec1_bytes(&sec1).map_err(|err| anyhow!("invalid P-256 public key: {err}"))
}

/// Wire format before Base64: version(1) | ephemeral X/Y(64) | nonce(16) | ciphertext(N).
fn ecies_encrypt(plaintext: &[u8], recipient: &PublicKey) -> Result<Vec<u8>> {
    let mut rng = EspRng;
    let ephemeral_secret = EphemeralSecret::random(&mut rng);
    let ephemeral_public = PublicKey::from(&ephemeral_secret);
    let ephemeral_sec1 = ephemeral_public.to_encoded_point(false);
    let ephemeral_xy = &ephemeral_sec1.as_bytes()[1..];

    let shared = ephemeral_secret.diffie_hellman(recipient);
    let mut hasher = Sha256::new();
    hasher.update(shared.raw_secret_bytes());
    hasher.update(&ephemeral_xy[..32]);
    let digest = hasher.finalize();

    let mut nonce = [0_u8; 16];
    rng.fill_bytes(&mut nonce);
    let mut ciphertext = plaintext.to_vec();
    Aes128Ctr::new((&digest[..16]).into(), (&nonce).into()).apply_keystream(&mut ciphertext);

    let mut output = Vec::with_capacity(1 + 64 + 16 + ciphertext.len());
    output.push(1);
    output.extend_from_slice(ephemeral_xy);
    output.extend_from_slice(&nonce);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let recipient = recipient_public_key()?;

    let mut dht = PinDriver::input_output_od(peripherals.pins.gpio4, Pull::Up)?;
    dht.set_high()?;

    let i2c = I2cDriver::new(
        peripherals.i2c0,
        peripherals.pins.gpio8,
        peripherals.pins.gpio9,
        &I2cConfig::new().baudrate(100_000.into()),
    )?;
    let mut delay = Ets;
    let mut bme280 = BME280::new_primary(i2c);
    bme280
        .init(&mut delay)
        .map_err(|err| anyhow!("BME280 init failed: {err:?}"))?;

    let adc = AdcDriver::new(peripherals.adc1)?;
    let mut mq2 = AdcChannelDriver::new(
        &adc,
        peripherals.pins.gpio1,
        &AdcChannelConfig {
            attenuation: DB_12,
            ..Default::default()
        },
    )?;

    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sysloop.clone(), Some(nvs))?,
        sysloop,
    )?;
    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: WIFI_SSID
            .try_into()
            .map_err(|_| anyhow!("WIFI_SSID is too long"))?,
        password: WIFI_PASS
            .try_into()
            .map_err(|_| anyhow!("WIFI_PASS is too long"))?,
        auth_method: AuthMethod::WPA2Personal,
        ..Default::default()
    }))?;
    wifi.start()?;
    wifi.connect()?;
    wifi.wait_netif_up()?;
    info!(
        "WiFi connected: {:?}",
        wifi.wifi().sta_netif().get_ip_info()?
    );

    let mut mqtt = EspMqttClient::new_cb(
        MQTT_BROKER,
        &MqttClientConfiguration {
            client_id: Some(CLIENT_ID),
            lwt: Some(esp_idf_svc::mqtt::client::LwtConfiguration {
                topic: TOPIC_STATUS,
                payload: b"offline",
                qos: QoS::AtLeastOnce,
                retain: true,
            }),
            ..Default::default()
        },
        |_| {},
    )?;
    FreeRtos::delay_ms(3000);
    mqtt.publish(TOPIC_STATUS, QoS::AtLeastOnce, true, b"online")?;

    loop {
        let result = (|| -> Result<()> {
            let (dht_temperature, dht_humidity) = read_dht22(&mut dht)?;
            let environmental = bme280
                .measure(&mut delay)
                .map_err(|err| anyhow!("BME280 read failed: {err:?}"))?;
            let gas_raw = adc.read(&mut mq2)?;
            let gas_alert = gas_raw >= GAS_THRESHOLD;
            let rssi = wifi.wifi().get_rssi()?;

            let plaintext = json!({
                "dispositivo": CLIENT_ID,
                "temperatura": (dht_temperature * 100.0).round() / 100.0,
                "humedad": (dht_humidity * 100.0).round() / 100.0,
                "temperatura_bme": (environmental.temperature * 100.0).round() / 100.0,
                "presion_hpa": ((environmental.pressure / 100.0) * 10.0).round() / 10.0,
                "gas_raw": gas_raw,
                "alerta_gas": gas_alert,
                "rssi": rssi
            })
            .to_string();

            let encrypted = ecies_encrypt(plaintext.as_bytes(), &recipient)?;
            let encoded = BASE64.encode(&encrypted);
            mqtt.publish(TOPIC_ENCRYPTED, QoS::AtLeastOnce, false, encoded.as_bytes())?;

            if gas_alert {
                mqtt.publish(
                    TOPIC_ALARM,
                    QoS::AtLeastOnce,
                    false,
                    b"ALERTA: gas threshold exceeded",
                )?;
            }

            info!(
                "Encrypted reading published: plaintext={} B, wire={} B, base64={} B",
                plaintext.len(),
                encrypted.len(),
                encoded.len()
            );
            Ok(())
        })();

        if let Err(err) = result {
            warn!("Measurement cycle failed: {err:#}");
        }
        FreeRtos::delay_ms(PUBLISH_INTERVAL_MS);
    }
}
