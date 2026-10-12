// Node-RED Function node, placed after decrypt-ecies.
const data = msg.payload;
const sheetsUrl = env.get("GOOGLE_SHEETS_URL");
const apiToken = env.get("GOOGLE_SHEETS_TOKEN");

if (!sheetsUrl || !apiToken) {
  node.error("GOOGLE_SHEETS_URL or GOOGLE_SHEETS_TOKEN is not configured", msg);
  return null;
}

msg.payload = {
  api_token: apiToken,
  dispositivo: data.dispositivo || "esp32-desconocido",
  temperatura: Number(data.temperatura ?? 0),
  humedad: Number(data.humedad ?? 0),
  presion_hpa: Number(data.presion_hpa ?? 0),
  gas_raw: Number(data.gas_raw ?? 0),
  alerta_gas: Boolean(data.alerta_gas),
  rssi: Number(data.rssi ?? 0),
};
msg.url = sheetsUrl;
msg.method = "POST";
msg.headers = { "content-type": "application/json" };
return msg;
