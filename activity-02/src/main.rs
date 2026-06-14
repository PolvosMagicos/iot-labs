use anyhow::Result;
use bme280::i2c::BME280;
use embedded_svc::mqtt::client::QoS;
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_hal::adc::attenuation::DB_12;
use esp_idf_hal::adc::oneshot::config::AdcChannelConfig;
use esp_idf_hal::adc::oneshot::{AdcChannelDriver, AdcDriver};
use esp_idf_hal::delay::{Ets, FreeRtos};
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

const CLIENT_ID: &str = "esp32-activity-02-rust-multisensor";

const TOPIC_AMBIENTAL: &str = "unmsm/iot2026/activity-02/sensores/ambiental";
const TOPIC_GAS: &str = "unmsm/iot2026/activity-02/sensores/gas";
const TOPIC_ALARMA: &str = "unmsm/iot2026/activity-02/alarmas/gas";
const TOPIC_STATUS: &str = "unmsm/iot2026/activity-02/status";

const GAS_THRESHOLD: u16 = 2500;

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // ---------- I2C for BME280 ----------
    let sda = peripherals.pins.gpio8;
    let scl = peripherals.pins.gpio9;

    let i2c_config = I2cConfig::new().baudrate(100_000.into());
    let i2c = I2cDriver::new(peripherals.i2c0, sda, scl, &i2c_config)?;

    let mut delay = Ets;

    let mut bme280 = BME280::new_primary(i2c);
    bme280
        .init(&mut delay)
        .map_err(|err| anyhow::anyhow!("BME280 init failed: {:?}", err))?;

    info!("BME280 initialized");

    // ---------- ADC for MQ-2 AO ----------
    let adc = AdcDriver::new(peripherals.adc1)?;

    let adc_config = AdcChannelConfig {
        attenuation: DB_12,
        ..Default::default()
    };

    // ESP32-S3 ADC1 pin.
    let mut mq2_ao = AdcChannelDriver::new(&adc, peripherals.pins.gpio1, &adc_config)?;

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
        warn!("Could not publish status yet: {:?}", err);
    }

    loop {
        // ---------- Read BME280 ----------
        let measurements = match bme280.measure(&mut delay) {
            Ok(m) => m,
            Err(err) => {
                warn!("BME280 read failed: {:?}", err);
                FreeRtos::delay_ms(3000);
                continue;
            }
        };

        let temperature = measurements.temperature;
        let humidity = measurements.humidity;
        let pressure_hpa = measurements.pressure / 100.0;

        // ---------- Read MQ-2 ----------
        let gas_raw: u16 = match adc.read(&mut mq2_ao) {
            Ok(value) => value,
            Err(err) => {
                warn!("MQ-2 ADC read failed: {:?}", err);
                FreeRtos::delay_ms(3000);
                continue;
            }
        };

        let gas_alert = gas_raw >= GAS_THRESHOLD;

        // ---------- Publish environmental JSON ----------
        let ambiental_payload = json!({
            "temperatura": (temperature * 100.0).round() / 100.0,
            "humedad": (humidity * 100.0).round() / 100.0,
            "presion_hpa": (pressure_hpa * 100.0).round() / 100.0,
            "dispositivo": CLIENT_ID
        });

        let ambiental_string = ambiental_payload.to_string();

        if let Err(err) = mqtt.publish(
            TOPIC_AMBIENTAL,
            QoS::AtLeastOnce,
            false,
            ambiental_string.as_bytes(),
        ) {
            warn!("Failed to publish ambiental: {:?}", err);
        }

        // ---------- Publish gas JSON ----------
        let gas_payload = json!({
            "valor_raw": gas_raw,
            "alerta": gas_alert,
            "threshold": GAS_THRESHOLD,
            "dispositivo": CLIENT_ID
        });

        let gas_string = gas_payload.to_string();

        if let Err(err) = mqtt.publish(TOPIC_GAS, QoS::AtLeastOnce, false, gas_string.as_bytes()) {
            warn!("Failed to publish gas: {:?}", err);
        }

        if gas_alert {
            if let Err(err) = mqtt.publish(
                TOPIC_ALARMA,
                QoS::AtLeastOnce,
                false,
                b"ALERTA: gas threshold exceeded",
            ) {
                warn!("Failed to publish gas alarm: {:?}", err);
            }
        }

        info!("Ambiental: {}", ambiental_string);
        info!("Gas: {}", gas_string);

        FreeRtos::delay_ms(10_000);
    }
}
