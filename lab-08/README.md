# Laboratorio 8 — MQTT con ECIES y Google Sheets (Rust)

Implementación para ESP32-S3 con Rust/ESP-IDF. El canal principal usa P-256
ECDH, SHA-256 y AES-128-CTR. Cada mensaje genera una clave efímera nueva y se
publica como Base64 con QoS 1.

## Estructura

- Las actividades 1–3 son la verificación base solicitada por la guía y ya
  están implementadas en `../lab-07/activity-01`, `activity-02` y
  `activity-03`.
- `activity-04`: lectura DHT22 + BME280 + MQ-2, cifrado ECIES y publicación
  MQTT.
- `node-red`: descifrado interoperable y preparación del POST.
- `google-sheets`: endpoint Apps Script para persistencia.
- `scripts/generate-keys.sh`: generación del par P-256.

## Cableado usado

| Componente | ESP32-S3 |
|---|---|
| DHT22 DATA | GPIO4, pull-up de 10 kΩ a 3.3 V |
| BME280 SDA/SCL | GPIO8/GPIO9 |
| MQ-2 AO | GPIO1 (ADC1), con salida limitada a 3.3 V |

El MQ-2 se alimenta con 5 V, pero su señal analógica nunca debe exceder 3.3 V
en el GPIO.

## 1. Generar las claves

```bash
chmod +x scripts/generate-keys.sh
./scripts/generate-keys.sh
cp .env.example .env
```

Copiar la línea `ECC_PUBLIC_KEY_HEX=...` mostrada por el script a `.env`.
La clave privada queda en `keys/ecc_private.pem`, ignorada por Git. Copiarla
al host de Node-RED con permisos `0600`.

## 2. Compilar y ejecutar el firmware

Configurar Wi-Fi, broker y clave pública en `.env`:

```dotenv
WIFI_SSID=...
WIFI_PASS=...
MQTT_BROKER=mqtt://192.168.1.100:1883
ECC_PUBLIC_KEY_HEX=<128 caracteres hex>
```

Desde el shell compartido:

```bash
nix-shell
cd activity-04
cargo build --release
espflash flash --monitor target/xtensa-esp32s3-espidf/release/activity-04-ecc
```

Verificación:

```bash
mosquitto_sub -h 192.168.1.100 \
  -t 'unmsm/iot2026/lab-08/g1/#' -v
```

`sensores/json_enc` debe contener Base64, no JSON legible.

## 3. Configurar Node-RED

Crear un flujo:

1. `mqtt in`: tópico
   `unmsm/iot2026/lab-08/g1/sensores/json_enc`, QoS 1.
2. `function`: pegar `node-red/decrypt-ecies.js`. En **Setup → Modules**,
   mapear `crypto` a `crypto` y `fs` a `fs`.
3. `debug`: conectar temporalmente para comprobar el JSON descifrado.
4. `function`: pegar `node-red/prepare-sheets.js`.
5. `http request`: método tomado de `msg.method`, URL de `msg.url`, retorno
   JSON y seguimiento de redirecciones habilitado.
6. `debug` y `catch` para observar respuestas y errores.

Configurar estas variables en el servicio Node-RED:

```dotenv
ECC_PRIVATE_KEY_PATH=/home/pi/.node-red/ecc_private.pem
GOOGLE_SHEETS_URL=https://script.google.com/macros/s/ID/exec
GOOGLE_SHEETS_TOKEN=<token aleatorio largo>
```

## 4. Configurar Google Sheets

Crear los encabezados:

`Timestamp | Dispositivo | Temperatura | Humedad | Presión_hPa | Gas_RAW |
Alerta_Gas | RSSI_dBm`

Pegar `google-sheets/Code.gs` en Apps Script. En **Project Settings → Script
Properties**, definir:

- `SHEET_ID`
- `SHEET_TAB`
- `API_TOKEN` (igual a `GOOGLE_SHEETS_TOKEN`)

Ejecutar `testDoPost`, autorizar el script y después desplegarlo como
aplicación web. La guía pide acceso público; el token añadido evita aceptar
escrituras anónimas sin autenticación de aplicación.

## Formato criptográfico

Antes de Base64:

```text
version 0x01 (1 B) | P-256 efímera X/Y (64 B) | nonce (16 B) | ciphertext
```

La clave AES es:

```text
SHA256(ECDH_shared_secret || ephemeral_x)[0..16]
```

Rust y Node.js validan la versión y usan exactamente este formato. AES-CTR
da confidencialidad, pero no autenticidad. Para producción se debe migrar a
AES-GCM o añadir HMAC; aquí se conserva AES-CTR porque es el algoritmo exigido
por la guía.
