use anyhow::{anyhow, bail, Result};
use bme280::i2c::BME280;
use embedded_svc::mqtt::client::QoS;
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
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
use serde_json::json;

const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASS: &str = env!("WIFI_PASS");

const MQTT_BROKER: &str = env!("MQTT_BROKER");

const CLIENT_ID: &str = "esp32-g1-rust-dht22-bme280";

const TOPIC_DHT22: &str = "unmsm/iot2026/activity-04/sensores/dht22";
const TOPIC_BME280: &str = "unmsm/iot2026/activity-04/sensores/ambiental";
const TOPIC_STATUS: &str = "unmsm/iot2026/activity-04/status";

fn micros() -> i64 {
    unsafe { esp_idf_svc::sys::esp_timer_get_time() }
}

fn read_dht22(dht: &mut PinDriver<'_, InputOutput>) -> Result<(f32, f32)> {
    macro_rules! wait_level {
        ($want_high:expr, $timeout_us:expr) => {{
            let start = micros();

            loop {
                let high = dht.is_high();

                if high == $want_high {
                    break;
                }

                if micros() - start > $timeout_us {
                    bail!("DHT22 timeout waiting for level {}", $want_high);
                }
            }
        }};
    }

    // Start signal.
    dht.set_low()?;
    Ets::delay_us(1200);

    // Release line.
    dht.set_high()?;
    Ets::delay_us(40);

    // Sensor response.
    wait_level!(false, 100);
    wait_level!(true, 100);
    wait_level!(false, 100);

    let mut data = [0u8; 5];

    for bit_index in 0..40 {
        wait_level!(true, 100);

        let high_start = micros();

        wait_level!(false, 120);

        let high_time = micros() - high_start;

        if high_time > 45 {
            data[bit_index / 8] |= 1 << (7 - (bit_index % 8));
        }
    }

    let checksum = data[0]
        .wrapping_add(data[1])
        .wrapping_add(data[2])
        .wrapping_add(data[3]);

    if checksum != data[4] {
        bail!(
            "DHT22 checksum error. Data: {:?}, expected: {}, got: {}",
            data,
            checksum,
            data[4]
        );
    }

    let raw_humidity = ((data[0] as u16) << 8) | data[1] as u16;
    let raw_temperature = (((data[2] & 0x7F) as u16) << 8) | data[3] as u16;

    let humidity = raw_humidity as f32 / 10.0;

    let mut temperature = raw_temperature as f32 / 10.0;
    if data[2] & 0x80 != 0 {
        temperature = -temperature;
    }

    Ok((temperature, humidity))
}

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // ---------- DHT22 on GPIO4 ----------
    let mut dht = PinDriver::input_output_od(peripherals.pins.gpio4, Pull::Up)?;
    dht.set_high()?;

    // ---------- BME280 on I2C GPIO8/GPIO9 ----------
    let sda = peripherals.pins.gpio8;
    let scl = peripherals.pins.gpio9;

    let i2c_config = I2cConfig::new().baudrate(100_000.into());
    let i2c = I2cDriver::new(peripherals.i2c0, sda, scl, &i2c_config)?;

    let mut delay = Ets;

    // Primary is usually 0x76.
    // If your board uses 0x77, change this to BME280::new_secondary(i2c).
    let mut bme280 = BME280::new_primary(i2c);

    bme280
        .init(&mut delay)
        .map_err(|err| anyhow!("BME280 init failed: {:?}", err))?;

    info!("BME280 initialized");

    // ---------- WiFi ----------
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sysloop.clone(), Some(nvs))?,
        sysloop,
    )?;

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: WIFI_SSID.try_into().unwrap(),
        password: WIFI_PASS.try_into().unwrap(),
        auth_method: AuthMethod::WPA2Personal,
        ..Default::default()
    }))?;

    info!("Starting WiFi...");
    wifi.start()?;

    info!("Connecting to WiFi SSID: {}", WIFI_SSID);
    wifi.connect()?;

    info!("Waiting for DHCP/IP...");
    wifi.wait_netif_up()?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;
    info!("WiFi connected. IP info: {:?}", ip_info);

    // ---------- MQTT ----------
    let mut mqtt = EspMqttClient::new_cb(
        MQTT_BROKER,
        &MqttClientConfiguration {
            client_id: Some(CLIENT_ID),
            ..Default::default()
        },
        |_event| {
            info!("MQTT event received");
        },
    )?;

    info!("MQTT client created, waiting for connection...");
    FreeRtos::delay_ms(4000);

    if let Err(err) = mqtt.publish(TOPIC_STATUS, QoS::AtLeastOnce, true, b"online") {
        warn!("Could not publish status: {:?}", err);
    }

    loop {
        // ---------- Read DHT22 ----------
        match read_dht22(&mut dht) {
            Ok((temperature, humidity)) => {
                let payload = json!({
                    "temperatura": temperature,
                    "humedad": humidity,
                    "unidad_temp": "C",
                    "dispositivo": "esp32-rust-dht22"
                });

                let payload_string = payload.to_string();

                if let Err(err) =
                    mqtt.publish(TOPIC_DHT22, QoS::AtLeastOnce, false, payload_string.as_bytes())
                {
                    warn!("Failed to publish DHT22: {:?}", err);
                } else {
                    info!("DHT22: {}", payload_string);
                }
            }
            Err(err) => {
                warn!("DHT22 read failed: {:?}", err);
            }
        }

        // ---------- Read BME280 ----------
        match bme280.measure(&mut delay) {
            Ok(measurements) => {
                let temperature = measurements.temperature;
                let humidity = measurements.humidity;
                let pressure_hpa = measurements.pressure / 100.0;

                let payload = json!({
                    "temperatura": (temperature * 100.0).round() / 100.0,
                    "humedad": (humidity * 100.0).round() / 100.0,
                    "presion_hpa": (pressure_hpa * 100.0).round() / 100.0,
                    "dispositivo": "esp32-rust-bme280"
                });

                let payload_string = payload.to_string();

                if let Err(err) =
                    mqtt.publish(TOPIC_BME280, QoS::AtLeastOnce, false, payload_string.as_bytes())
                {
                    warn!("Failed to publish BME280: {:?}", err);
                } else {
                    info!("BME280: {}", payload_string);
                }
            }
            Err(err) => {
                warn!("BME280 read failed: {:?}", err);
            }
        }

        FreeRtos::delay_ms(10_000);
    }
}
