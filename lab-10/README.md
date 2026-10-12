# Laboratorio 10 IoT — Implementación completa en Rust

Arquitectura:

```text
ESP32-S3 (Rust + ESP-IDF)
  └─ lectura/simulación de sensores
  └─ ECDH P-256 efímero + SHA-256 + AES-256-GCM
  └─ MQTT QoS 1
          ↓
Backend Rust
  └─ rumqttc subscriber
  └─ descifrado con la clave privada del servidor
  └─ Google Sheets API
  └─ Webhook IFTTT
  └─ dashboard HTTP con Axum
```

## 1. Requisitos

### ESP32

- ESP32-S3.
- Rust para ESP-IDF.
- `cargo-espflash`.
- Cable USB de datos.
- Red Wi-Fi de 2.4 GHz.

### Backend

- Rust estable reciente.
- Una cuenta/proyecto de Google Cloud.
- Google Sheets API habilitada.
- Una cuenta de servicio con una llave JSON.
- Una hoja de cálculo compartida con el correo de la cuenta de servicio.
- IFTTT Webhooks conectado.

## 2. Generar las claves ECDH del laboratorio

La clave privada permanece únicamente en el backend. El ESP32 recibe solo la clave pública.

```bash
cd backend
cargo run --bin keygen
```

La salida tendrá este formato:

```text
SERVER_PRIVATE_KEY_B64=...
SERVER_PUBLIC_KEY_B64=...
```

Copia `SERVER_PRIVATE_KEY_B64` al archivo `backend/.env`.

Antes de compilar el ESP32, exporta `SERVER_PUBLIC_KEY_B64` en la terminal.

No subas la clave privada, `credentials.json` ni `.env` a Git.

## 3. Google Cloud y Google Sheets

1. Crea un proyecto en Google Cloud.
2. Habilita **Google Sheets API**.
3. Crea una cuenta de servicio.
4. Genera una llave JSON y guárdala como:

```text
backend/credentials.json
```

5. Crea una hoja llamada `Monitoreo_Ambiental_IoT`.
6. En la primera fila crea estas columnas:

```text
timestamp | usuario | temperatura | humedad | calidad_aire
```

7. Abre `credentials.json`, copia el valor de `client_email` y comparte la hoja con ese correo como **Editor**.
8. Obtén el ID de la hoja desde su URL:

```text
https://docs.google.com/spreadsheets/d/SPREADSHEET_ID/edit
```

9. Coloca el ID en `backend/.env`.

## 4. Configurar IFTTT

1. Conecta el servicio **Webhooks**.
2. Crea un Applet.
3. Trigger: **Webhooks → Receive a web request**.
4. Event name:

```text
alerta_sensor_iot
```

5. Acción: **Gmail/Email → Send an email**.
6. Destinatario:

```text
gruposmartinnova@gmail.com
```

7. Asunto sugerido:

```text
Alerta urgente de monitoreo ambiental IoT
```

8. Cuerpo obligatorio:

```text
URGENCIA valor [{{Value1}} = {{Value2}}] enviado por [{{Value3}}]
```

9. En la página de documentación de Webhooks copia la key y colócala en `backend/.env`.

## 5. Ejecutar el backend

```bash
cd backend
cp .env.example .env
```

Completa `.env` y ejecuta:

```bash
cargo run --release
```

El backend:

- se suscribe al topic MQTT;
- descifra cada lectura;
- guarda una fila en Google Sheets;
- envía un Webhook cuando un límite es superado;
- publica un dashboard en `http://127.0.0.1:8080`.

Prueba de salud:

```bash
curl http://127.0.0.1:8080/health
```

## 6. Compilar y flashear el ESP32-S3

Exporta las variables antes de compilar. Los valores se incorporan al firmware con `env!`.

```bash
cd esp32-node

export WIFI_SSID='NOMBRE_WIFI'
export WIFI_PASS='PASSWORD_WIFI'
export MQTT_URL='mqtt://broker.hivemq.com:1883'
export MQTT_TOPIC='iot/ambiente/seguro/grupo4/sebastian'
export MQTT_CLIENT_ID='esp32-grupo4-sebastian'
export DEVICE_USER='ESP32_Nodo_Grupo4'
export SERVER_PUBLIC_KEY_B64='CLAVE_PUBLICA_GENERADA'

cargo espflash flash --release --monitor
```

El topic debe coincidir exactamente con `MQTT_TOPIC` en `backend/.env`.

Durante una demostración puedes usar:

```bash
export SAMPLE_INTERVAL_SECONDS='10'
```

Para la entrega final usa:

```bash
export SAMPLE_INTERVAL_SECONDS='300'
```

Como `SAMPLE_INTERVAL_SECONDS` también se incorpora al compilar, vuelve a flashear después de cambiarlo.

## 7. Probar MQTT antes de usar Google/IFTTT

Puedes observar los paquetes cifrados:

```bash
mosquitto_sub \
  -h broker.hivemq.com \
  -p 1883 \
  -t 'iot/ambiente/seguro/grupo4/sebastian' \
  -v
```

No deben aparecer los valores de temperatura, humedad o calidad del aire en texto plano. Debes ver campos Base64:

```json
{
  "version": 1,
  "ephemeral_public_key": "...",
  "nonce": "...",
  "ciphertext": "..."
}
```

## 8. Sustituir la simulación por sensores reales

El archivo `esp32-node/src/main.rs` contiene:

```rust
fn read_environment() -> SensorReading
```

Actualmente genera valores con el generador aleatorio de hardware del ESP32 para que el laboratorio se pueda demostrar sin sensores conectados.

Cuando tengas un DHT22/BME280/MQ, reemplaza únicamente esa función. No cambies la capa criptográfica ni MQTT.

## 9. Evidencias sugeridas para el informe

1. Diagrama de arquitectura.
2. Consola del ESP32 mostrando Wi-Fi, MQTT y publicación.
3. Captura de `mosquitto_sub` mostrando payload cifrado.
4. Consola del backend mostrando descifrado.
5. Google Sheets con filas insertadas.
6. Correo recibido desde IFTTT.
7. Dashboard web.
8. Explicación de:
   - ECDH P-256;
   - clave efímera;
   - derivación SHA-256;
   - AES-256-GCM;
   - nonce de 96 bits;
   - autenticidad/integridad del tag GCM.

## 10. Diferencia frente al código original del laboratorio

El código MicroPython proporcionado por la guía no cifra. Convierte el JSON a hexadecimal:

```python
raw_json.encode("utf-8").hex()
```

Eso es codificación reversible, no criptografía.

Esta implementación sí realiza:

```text
clave privada efímera ESP32 + clave pública servidor
                         ↓ ECDH
                   secreto compartido
                         ↓ SHA-256
                     clave AES
                         ↓ AES-GCM
                  ciphertext + tag
```

Cada lectura usa:

- una nueva clave efímera P-256;
- un nonce aleatorio de 12 bytes;
- AES-256-GCM autenticado.

## 11. Consideraciones

- HiveMQ público sirve para el laboratorio, no para producción.
- En producción usa MQTT sobre TLS, autenticación y ACL.
- La key de IFTTT debe permanecer privada.
- La llave JSON de Google es una credencial sensible.
- El dashboard consulta Google Sheets, no solo memoria local.
- El backend usa la hora de recepción UTC como timestamp autoritativo, evitando depender del reloj sin RTC del ESP32.

## 12. Entorno NixOS y prueba sin ESP32

Desde la raíz:

```bash
nix develop
```

Instala una sola vez las herramientas ESP-RS:

```bash
rustup default stable
cargo install espup --locked
cargo install espflash --locked
espup install --targets esp32s3 --std
source "$HOME/export-esp.sh"
```

Antes de flashear, prueba el flujo completo con el simulador Rust:

```bash
cd backend
cargo run --bin simulator -- 40 90 130
```

Los tres valores superan los límites y deberían crear una fila y disparar alertas IFTTT.
