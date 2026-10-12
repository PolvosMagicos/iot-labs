use std::{env, fs, net::SocketAddr, sync::Arc, time::Duration};

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, bail, Context, Result};
use axum::{extract::State, response::Html, routing::get, Router};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use p256::{ecdh::diffie_hellman, PublicKey, SecretKey};
use reqwest::Client as HttpClient;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tracing::{error, info, warn};

const GOOGLE_SCOPE: &str = "https://www.googleapis.com/auth/spreadsheets";

#[derive(Debug, Clone)]
struct Config {
    mqtt_host: String,
    mqtt_port: u16,
    mqtt_client_id: String,
    mqtt_topic: String,
    server_private_key_b64: String,
    google_credentials_path: String,
    google_spreadsheet_id: String,
    google_sheet_range: String,
    ifttt_event_name: String,
    ifttt_key: String,
    http_bind: SocketAddr,
}

impl Config {
    fn from_env() -> Result<Self> {
        Ok(Self {
            mqtt_host: required_env("MQTT_HOST")?,
            mqtt_port: env::var("MQTT_PORT")
                .unwrap_or_else(|_| "1883".into())
                .parse()
                .context("MQTT_PORT must be a valid u16")?,
            mqtt_client_id: required_env("MQTT_CLIENT_ID")?,
            mqtt_topic: required_env("MQTT_TOPIC")?,
            server_private_key_b64: required_env("SERVER_PRIVATE_KEY_B64")?,
            google_credentials_path: required_env("GOOGLE_CREDENTIALS_PATH")?,
            google_spreadsheet_id: required_env("GOOGLE_SPREADSHEET_ID")?,
            google_sheet_range: env::var("GOOGLE_SHEET_RANGE")
                .unwrap_or_else(|_| "Sheet1!A:E".into()),
            ifttt_event_name: required_env("IFTTT_EVENT_NAME")?,
            ifttt_key: required_env("IFTTT_KEY")?,
            http_bind: env::var("HTTP_BIND")
                .unwrap_or_else(|_| "0.0.0.0:8080".into())
                .parse()
                .context("HTTP_BIND must be host:port")?,
        })
    }
}

fn required_env(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("missing environment variable {name}"))
}

#[derive(Debug, Deserialize)]
struct EncryptedPayload {
    version: u8,
    ephemeral_public_key: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct SensorReading {
    usuario: String,
    temperatura: f32,
    humedad: f32,
    calidad_aire: f32,
    #[serde(default)]
    device_timestamp_unix: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
struct StoredReading {
    timestamp: DateTime<Utc>,
    usuario: String,
    temperatura: f32,
    humedad: f32,
    calidad_aire: f32,
}

#[derive(Debug, Deserialize, Clone)]
struct ServiceAccount {
    client_email: String,
    private_key: String,
    token_uri: String,
}

#[derive(Debug, Serialize)]
struct JwtClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    exp: usize,
    iat: usize,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct SheetsValuesResponse {
    #[serde(default)]
    values: Vec<Vec<Value>>,
}

#[derive(Clone)]
struct GoogleSheetsClient {
    http: HttpClient,
    credentials: ServiceAccount,
    spreadsheet_id: String,
    range: String,
}

impl GoogleSheetsClient {
    fn new(config: &Config, http: HttpClient) -> Result<Self> {
        let raw = fs::read_to_string(&config.google_credentials_path)
            .with_context(|| format!("cannot read {}", config.google_credentials_path))?;
        let credentials: ServiceAccount =
            serde_json::from_str(&raw).context("invalid Google service-account JSON")?;

        Ok(Self {
            http,
            credentials,
            spreadsheet_id: config.google_spreadsheet_id.clone(),
            range: config.google_sheet_range.clone(),
        })
    }

    async fn access_token(&self) -> Result<String> {
        let now = Utc::now().timestamp() as usize;
        let claims = JwtClaims {
            iss: &self.credentials.client_email,
            scope: GOOGLE_SCOPE,
            aud: &self.credentials.token_uri,
            iat: now,
            exp: now + 3600,
        };

        let key = EncodingKey::from_rsa_pem(self.credentials.private_key.as_bytes())
            .context("invalid RSA private key in credentials.json")?;
        let jwt = encode(&Header::new(Algorithm::RS256), &claims, &key)
            .context("cannot sign Google OAuth JWT")?;

        let response = self
            .http
            .post(&self.credentials.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", jwt.as_str()),
            ])
            .send()
            .await
            .context("Google OAuth token request failed")?
            .error_for_status()
            .context("Google OAuth rejected the service account")?
            .json::<TokenResponse>()
            .await
            .context("invalid Google OAuth response")?;

        Ok(response.access_token)
    }

    async fn append(&self, reading: &StoredReading) -> Result<()> {
        let token = self.access_token().await?;
        let range = urlencoding::encode(&self.range);
        let url = format!(
            "https://sheets.googleapis.com/v4/spreadsheets/{}/values/{}:append",
            self.spreadsheet_id, range
        );

        let response = self
            .http
            .post(url)
            .bearer_auth(token)
            .query(&[
                ("valueInputOption", "USER_ENTERED"),
                ("insertDataOption", "INSERT_ROWS"),
            ])
            .json(&json!({
                "majorDimension": "ROWS",
                "values": [[
                    reading.timestamp.to_rfc3339(),
                    reading.usuario.clone(),
                    reading.temperatura,
                    reading.humedad,
                    reading.calidad_aire
                ]]
            }))
            .send()
            .await
            .context("Google Sheets append request failed")?;

        let status = response.status();
        let response_body = response
            .text()
            .await
            .context("could not read Google Sheets response")?;

        if !status.is_success() {
            anyhow::bail!("Google Sheets append failed with status {status}: {response_body}");
        }

        Ok(())
    }

    async fn read_rows(&self) -> Result<Vec<Vec<String>>> {
        let token = self.access_token().await?;
        let range = urlencoding::encode(&self.range);
        let url = format!(
            "https://sheets.googleapis.com/v4/spreadsheets/{}/values/{}",
            self.spreadsheet_id, range
        );

        let response = self
            .http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .context("Google Sheets read request failed")?
            .error_for_status()
            .context("Google Sheets rejected read request")?
            .json::<SheetsValuesResponse>()
            .await
            .context("invalid Google Sheets response")?;

        Ok(response
            .values
            .into_iter()
            .map(|row| row.into_iter().map(value_to_string).collect())
            .collect())
    }
}

fn value_to_string(value: Value) -> String {
    match value {
        Value::String(value) => value,
        other => other.to_string(),
    }
}

#[derive(Clone)]
struct AppState {
    sheets: GoogleSheetsClient,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::from_env()?;
    let http = HttpClient::builder()
        .use_rustls_tls()
        .no_proxy()
        .timeout(Duration::from_secs(20))
        .build()
        .context("cannot create HTTP client")?;

    let private_key = parse_private_key(&config.server_private_key_b64)?;
    let sheets = GoogleSheetsClient::new(&config, http.clone())?;

    let mqtt_config = config.clone();
    let mqtt_sheets = sheets.clone();
    let mqtt_http = http.clone();

    tokio::spawn(async move {
        if let Err(error) = run_mqtt(mqtt_config, private_key, mqtt_sheets, mqtt_http).await {
            error!(?error, "MQTT task stopped");
        }
    });

    let state = AppState { sheets };
    let app = Router::new()
        .route("/", get(dashboard))
        .route("/health", get(|| async { "ok" }))
        .with_state(Arc::new(state));

    let listener = TcpListener::bind(config.http_bind)
        .await
        .with_context(|| format!("cannot bind {}", config.http_bind))?;

    info!("Dashboard: http://{}", config.http_bind);
    axum::serve(listener, app).await?;

    Ok(())
}

async fn run_mqtt(
    config: Config,
    private_key: SecretKey,
    sheets: GoogleSheetsClient,
    http: HttpClient,
) -> Result<()> {
    let mut options = MqttOptions::new(
        config.mqtt_client_id.clone(),
        config.mqtt_host.clone(),
        config.mqtt_port,
    );
    options.set_keep_alive(Duration::from_secs(30));
    options.set_clean_session(true);

    let (client, mut event_loop) = AsyncClient::new(options, 20);
    client
        .subscribe(config.mqtt_topic.clone(), QoS::AtLeastOnce)
        .await
        .context("cannot subscribe to MQTT topic")?;

    info!(
        host = %config.mqtt_host,
        topic = %config.mqtt_topic,
        "MQTT subscriber started"
    );

    loop {
        match event_loop.poll().await {
            Ok(Event::Incoming(Packet::Publish(publication))) => {
                if publication.topic != config.mqtt_topic {
                    continue;
                }

                match process_message(&publication.payload, &private_key, &sheets, &http, &config)
                    .await
                {
                    Ok(()) => info!("MQTT reading processed"),
                    Err(error) => warn!(?error, "invalid MQTT reading ignored"),
                }
            }
            Ok(_) => {}
            Err(error) => {
                warn!(?error, "MQTT connection error; retrying");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

async fn process_message(
    bytes: &[u8],
    private_key: &SecretKey,
    sheets: &GoogleSheetsClient,
    http: &HttpClient,
    config: &Config,
) -> Result<()> {
    let encrypted: EncryptedPayload =
        serde_json::from_slice(bytes).context("payload is not valid encrypted JSON")?;

    if encrypted.version != 1 {
        bail!(
            "unsupported encrypted payload version {}",
            encrypted.version
        );
    }

    let reading = decrypt_reading(private_key, &encrypted)?;
    validate_reading(&reading)?;

    let stored = StoredReading {
        timestamp: Utc::now(),
        usuario: reading.usuario,
        temperatura: reading.temperatura,
        humedad: reading.humedad,
        calidad_aire: reading.calidad_aire,
    };

    info!(
        user = %stored.usuario,
        temperature = stored.temperatura,
        humidity = stored.humedad,
        air_quality = stored.calidad_aire,
        "decrypted environmental reading"
    );

    sheets.append(&stored).await?;

    let limits = [
        ("temperatura", stored.temperatura, 38.0_f32),
        ("humedad", stored.humedad, 85.0_f32),
        ("calidad_aire", stored.calidad_aire, 120.0_f32),
    ];

    for (variable, value, limit) in limits {
        if value > limit {
            trigger_ifttt(
                http,
                &config.ifttt_event_name,
                &config.ifttt_key,
                variable,
                value,
                &stored.usuario,
            )
            .await?;
        }
    }

    Ok(())
}

fn decrypt_reading(
    server_private_key: &SecretKey,
    encrypted: &EncryptedPayload,
) -> Result<SensorReading> {
    let ephemeral_bytes = STANDARD
        .decode(&encrypted.ephemeral_public_key)
        .context("invalid Base64 ephemeral public key")?;
    let ephemeral_public_key = PublicKey::from_sec1_bytes(&ephemeral_bytes)
        .context("invalid P-256 ephemeral public key")?;

    let shared_secret = diffie_hellman(
        server_private_key.to_nonzero_scalar(),
        ephemeral_public_key.as_affine(),
    );

    let aes_key = Sha256::digest(shared_secret.raw_secret_bytes());
    let cipher = Aes256Gcm::new_from_slice(&aes_key).map_err(|_| anyhow!("invalid AES key"))?;

    let nonce_bytes = STANDARD
        .decode(&encrypted.nonce)
        .context("invalid Base64 nonce")?;
    if nonce_bytes.len() != 12 {
        bail!("AES-GCM nonce must have 12 bytes");
    }

    let nonce = Nonce::try_from(nonce_bytes.as_slice())
        .map_err(|_| anyhow!("AES-GCM nonce must have exactly 12 bytes"))?;

    let ciphertext = STANDARD
        .decode(&encrypted.ciphertext)
        .context("invalid Base64 ciphertext")?;

    let plaintext = cipher
        .decrypt(&nonce, ciphertext.as_ref())
        .map_err(|_| anyhow!("AES-GCM authentication/decryption failed"))?;

    serde_json::from_slice(&plaintext).context("decrypted payload is not valid sensor JSON")
}

fn parse_private_key(encoded: &str) -> Result<SecretKey> {
    let bytes = STANDARD
        .decode(encoded)
        .context("SERVER_PRIVATE_KEY_B64 is not valid Base64")?;

    SecretKey::from_slice(&bytes).context("SERVER_PRIVATE_KEY_B64 is not a P-256 private key")
}

fn validate_reading(reading: &SensorReading) -> Result<()> {
    if reading.usuario.trim().is_empty() || reading.usuario.len() > 100 {
        bail!("invalid usuario");
    }

    for (name, value, min, max) in [
        ("temperatura", reading.temperatura, -40.0, 100.0),
        ("humedad", reading.humedad, 0.0, 100.0),
        ("calidad_aire", reading.calidad_aire, 0.0, 10_000.0),
    ] {
        if !value.is_finite() || value < min || value > max {
            bail!("{name} outside accepted range");
        }
    }

    Ok(())
}

async fn trigger_ifttt(
    http: &HttpClient,
    event_name: &str,
    key: &str,
    variable: &str,
    value: f32,
    usuario: &str,
) -> Result<()> {
    let url = format!(
        "https://maker.ifttt.com/trigger/{}/with/key/{}",
        event_name, key
    );

    http.post(url)
        .json(&json!({
            "value1": variable,
            "value2": format!("{value:.2}"),
            "value3": usuario,
        }))
        .send()
        .await
        .context("IFTTT request failed")?
        .error_for_status()
        .context("IFTTT rejected webhook")?;

    warn!(
        variable,
        value, usuario, "threshold exceeded; IFTTT webhook sent"
    );

    Ok(())
}

async fn dashboard(State(state): State<Arc<AppState>>) -> Html<String> {
    match state.sheets.read_rows().await {
        Ok(rows) => Html(render_dashboard(rows)),
        Err(error) => {
            error!(?error, "dashboard could not read Google Sheets");
            Html(format!(
                r#"<!doctype html>
<html lang="es">
<head><meta charset="utf-8"><meta http-equiv="refresh" content="10"><title>IoT</title></head>
<body><h1>Monitoreo Ambiental IoT</h1><p>No se pudo leer Google Sheets.</p><pre>{}</pre></body>
</html>"#,
                html_escape::encode_text(&error.to_string())
            ))
        }
    }
}

fn render_dashboard(rows: Vec<Vec<String>>) -> String {
    let mut body = String::new();

    for row in rows.into_iter().skip(1).rev().take(50) {
        let cell = |index: usize| {
            html_escape::encode_text(row.get(index).map(String::as_str).unwrap_or("")).to_string()
        };

        body.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            cell(0),
            cell(1),
            cell(2),
            cell(3),
            cell(4)
        ));
    }

    format!(
        r#"<!doctype html>
<html lang="es">
<head>
  <meta charset="utf-8">
  <meta http-equiv="refresh" content="10">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Monitoreo Ambiental IoT</title>
  <style>
    body {{ font-family: system-ui, sans-serif; max-width: 1100px; margin: 2rem auto; padding: 0 1rem; }}
    table {{ width: 100%; border-collapse: collapse; }}
    th, td {{ border: 1px solid #bbb; padding: .65rem; text-align: left; }}
    th {{ background: #eee; }}
    .note {{ color: #555; }}
  </style>
</head>
<body>
  <h1>Monitoreo Ambiental Seguro</h1>
  <p class="note">Datos persistidos en Google Sheets. Actualización automática cada 10 segundos.</p>
  <table>
    <thead>
      <tr>
        <th>Timestamp UTC</th>
        <th>Usuario</th>
        <th>Temperatura °C</th>
        <th>Humedad %</th>
        <th>Calidad del aire</th>
      </tr>
    </thead>
    <tbody>{body}</tbody>
  </table>
</body>
</html>"#
    )
}
