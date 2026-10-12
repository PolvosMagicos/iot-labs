# Laboratorios IoT — ESP32-S3 en Rust

Laboratorios del curso de IoT implementados en Rust sobre ESP-IDF para la
placa ESP32-S3: lectura de sensores, publicación MQTT, cifrado de extremo a
extremo y persistencia en Google Sheets.

| Laboratorio | Tema | Componentes |
|---|---|---|
| [`lab-07`](lab-07) | Sensores y actuadores por MQTT | 4 actividades de firmware + stack Mosquitto / InfluxDB / Node-RED / Grafana |
| [`lab-08`](lab-08/README.md) | MQTT cifrado con ECIES y Google Sheets | Firmware (P-256 + AES-128-CTR), flujo Node-RED, Apps Script |
| [`lab-10`](lab-10/README.md) | Monitoreo ambiental seguro de extremo a extremo | Firmware (P-256 + AES-256-GCM), backend Rust con Google Sheets API, IFTTT y dashboard |

## Estructura

```text
.
├── lab-07/
│   ├── activity-01/   DHT22 → MQTT (temperatura, humedad, JSON)
│   ├── activity-02/   BME280 + MQ-2 → MQTT con alarma de gas
│   ├── activity-03/   Relé controlado por comandos MQTT
│   ├── activity-04/   DHT22 + BME280 → MQTT
│   ├── mosquitto/     compose.yml: Mosquitto, InfluxDB, Node-RED, Grafana
│   └── shell.nix
├── lab-08/
│   ├── activity-04/   Sensores + cifrado ECIES → MQTT
│   ├── node-red/      Descifrado y envío a Sheets
│   ├── google-sheets/ Endpoint Apps Script
│   ├── scripts/       Generación del par de claves P-256
│   └── shell.nix
└── lab-10/
    ├── esp32-node/    Firmware con cifrado AES-256-GCM
    ├── backend/       Suscriptor MQTT, descifrado, Sheets, IFTTT, dashboard
    └── flake.nix / shell.nix
```

## Requisitos

- ESP32-S3 y cable USB de datos.
- Red Wi-Fi de 2.4 GHz.
- Toolchain Rust para Xtensa (`espup`) y `espflash`.
- Nix (opcional): cada laboratorio incluye un `shell.nix` con las
  herramientas necesarias.

Instalación del toolchain (una sola vez):

```bash
cargo install espup espflash --locked
espup install --targets esp32s3 --std
source "$HOME/export-esp.sh"
```

La primera compilación de cada proyecto descarga ESP-IDF y tarda varios
minutos.

## Uso rápido

### Lab 07

Las credenciales se leen de `lab-07/.env` en tiempo de compilación (ver
`build.rs` de cada actividad):

```bash
cd lab-07
cp .env.example .env        # completar WIFI_SSID, WIFI_PASS, MQTT_BROKER
nix-shell

# Broker y servicios
(cd mosquitto && podman-compose up -d)

# Firmware
cd activity-01
cargo run --release         # compila, flashea y abre el monitor
```

Para observar los mensajes:

```bash
mosquitto_sub -h <broker> -t 'unmsm/iot2026/#' -v
```

### Lab 08 y Lab 10

Cada uno tiene su propio README con la configuración completa de claves,
Node-RED, Google Sheets e IFTTT:

- [`lab-08/README.md`](lab-08/README.md)
- [`lab-10/README.md`](lab-10/README.md)

## Secretos

Ningún secreto se versiona. Están excluidos por `.gitignore`:

- `.env` de cada laboratorio (Wi-Fi, broker, tokens). Usar `.env.example`
  como plantilla.
- Claves privadas (`keys/`, `*.pem`, `keys.txt`).
- Llaves de cuentas de servicio de Google (`credentials.json`, `*.json` del
  backend del lab 10).

También se excluyen las salidas de compilación (`target/`, `.embuild/`,
`sdkconfig`).
