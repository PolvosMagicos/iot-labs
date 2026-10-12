use anyhow::Result;
use embedded_svc::mqtt::client::QoS;
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::PinDriver;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::mqtt::client::{EspMqttClient, EventPayload, MqttClientConfiguration};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use log::{info, warn};
use std::sync::mpsc;

const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASS: &str = env!("WIFI_PASS");

const MQTT_BROKER: &str = env!("MQTT_BROKER");

const CLIENT_ID: &str = "esp32-activity-03-rust-relay";

const TOPIC_CMD: &str = "unmsm/iot2026/activity-03/actuadores/relay";
const TOPIC_STATUS: &str = "unmsm/iot2025/activity-03/actuadores/relay/status";

#[derive(Debug)]
enum RelayCommand {
    On,
    Off,
}

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // Relay on IO10.
    // Change gpio10 if your board exposes a different pin.
    let mut relay = PinDriver::output(peripherals.pins.gpio10)?;

    // Most relay modules are active-low, so HIGH = OFF.
    relay.set_high()?;
    info!("Relay initialized OFF");

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

    let (tx, rx) = mpsc::channel::<RelayCommand>();

    let mut mqtt = EspMqttClient::new_cb(
        MQTT_BROKER,
        &MqttClientConfiguration {
            client_id: Some(CLIENT_ID),
            ..Default::default()
        },
        move |event| {
            if let EventPayload::Received { topic, data, .. } = event.payload() {
                let Some(topic) = topic else {
                    return;
                };

                let Ok(message) = core::str::from_utf8(data) else {
                    return;
                };

                info!("MQTT received topic={} payload={}", topic, message);

                if topic == TOPIC_CMD {
                    let msg = message.trim().to_ascii_uppercase();

                    if msg == "ON" || msg == "1" {
                        let _ = tx.send(RelayCommand::On);
                    } else if msg == "OFF" || msg == "0" {
                        let _ = tx.send(RelayCommand::Off);
                    } else {
                        warn!("Unknown relay command: {}", message);
                    }
                }
            }
        },
    )?;

    info!("MQTT client created, waiting...");
    FreeRtos::delay_ms(3000);

    mqtt.subscribe(TOPIC_CMD, QoS::AtLeastOnce)?;
    info!("Subscribed to {}", TOPIC_CMD);

    mqtt.publish(TOPIC_STATUS, QoS::AtLeastOnce, true, b"OFF")?;
    info!("Published initial relay status OFF");

    loop {
        match rx.try_recv() {
            Ok(RelayCommand::On) => {
                relay.set_low()?;
                mqtt.publish(TOPIC_STATUS, QoS::AtLeastOnce, true, b"ON")?;
                info!("Relay ON");
            }
            Ok(RelayCommand::Off) => {
                relay.set_high()?;
                mqtt.publish(TOPIC_STATUS, QoS::AtLeastOnce, true, b"OFF")?;
                info!("Relay OFF");
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                warn!("MQTT command channel disconnected");
            }
        }

        FreeRtos::delay_ms(100);
    }
}
