use anyhow::{bail, Result};
use embedded_svc::mqtt::client::QoS;
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_hal::delay::{Ets, FreeRtos};
use esp_idf_hal::gpio::{PinDriver, Pull};
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

const CLIENT_ID: &str = "esp32-activity-01-rust-dht22";

const TOPIC_TEMP: &str = "unmsm/iot2026/activity-01/sensores/temperatura";
const TOPIC_HUM: &str = "unmsm/iot2026/activity-01/sensores/humedad";
const TOPIC_JSON: &str = "unmsm/iot2026/activity-01/sensores/dht22";
const TOPIC_STATUS: &str = "unmsm/iot2026/activity-01/status";

fn micros() -> i64 {
    unsafe { esp_idf_svc::sys::esp_timer_get_time() }
}

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // DHT22 DATA on GPIO4.
    // Use open-drain + pull-up.
    let mut dht = PinDriver::input_output_od(peripherals.pins.gpio4, Pull::Up)?;
    dht.set_high()?;

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

    let mut mqtt = EspMqttClient::new_cb(
        MQTT_BROKER,
        &MqttClientConfiguration {
            client_id: Some(CLIENT_ID),
            ..Default::default()
        },
        |_event| {
            info!("MQTT event recieved");
        },
    )?;

    info!("MQTT client created, waiting for connection...");
    FreeRtos::delay_ms(4000);

    if let Err(err) = mqtt.publish(TOPIC_STATUS, QoS::AtLeastOnce, true, b"online") {
        warn!("Could not publish status yet: {:?}", err);
    }

    loop {
        let reading = (|| -> Result<(f32, f32)> {
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

            // Start signal for DHT22.
            dht.set_low()?;
            Ets::delay_us(1200);

            // Release line.
            dht.set_high()?;
            Ets::delay_us(40);

            // Sensor response: LOW ~80us, HIGH ~80us, then LOW.
            wait_level!(false, 100);
            wait_level!(true, 100);
            wait_level!(false, 100);

            let mut data = [0u8; 5];

            for bit_index in 0..40 {
                // Each bit starts with LOW ~50us, then HIGH.
                wait_level!(true, 100);

                let high_start = micros();

                // HIGH duration determines 0 or 1.
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
                    "DHT22 checksum error. Data: {:?}, expected checksum: {}, got: {}",
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
        })();

        match reading {
            Ok((temperature, humidity)) => {
                info!(
                    "DHT22 reading → temp: {:.2} °C, humidity: {:.2} %",
                    temperature, humidity
                );

                let temp_string = format!("{temperature:.2}");
                let hum_string = format!("{humidity:.2}");

                if let Err(err) =
                    mqtt.publish(TOPIC_TEMP, QoS::AtMostOnce, false, temp_string.as_bytes())
                {
                    warn!("Failed to publish temperature: {:?}", err);
                }

                if let Err(err) =
                    mqtt.publish(TOPIC_HUM, QoS::AtMostOnce, false, hum_string.as_bytes())
                {
                    warn!("Failed to publish humidity: {:?}", err);
                }

                let payload = json!({
                    "temperatura": temperature,
                    "humedad": humidity,
                    "unidad_temp": "C",
                    "dispositivo": CLIENT_ID
                });

                let payload_string = payload.to_string();

                if let Err(err) = mqtt.publish(
                    TOPIC_JSON,
                    QoS::AtMostOnce,
                    false,
                    payload_string.as_bytes(),
                ) {
                    warn!("Failed to publish JSON: {:?}", err);
                }

                info!("Published JSON: {}", payload_string);
            }
            Err(err) => {
                warn!("DHT22 read failed: {:?}", err);
            }
        }

        FreeRtos::delay_ms(5000);
    }
}
