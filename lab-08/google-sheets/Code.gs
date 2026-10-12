const SHEET_ID = PropertiesService.getScriptProperties().getProperty('SHEET_ID');
const SHEET_TAB = PropertiesService.getScriptProperties().getProperty('SHEET_TAB');
const API_TOKEN = PropertiesService.getScriptProperties().getProperty('API_TOKEN');

function jsonResponse(body) {
  return ContentService
    .createTextOutput(JSON.stringify(body))
    .setMimeType(ContentService.MimeType.JSON);
}

function doPost(e) {
  const lock = LockService.getScriptLock();

  try {
    if (!SHEET_ID || !SHEET_TAB || !API_TOKEN) {
      throw new Error('Configure SHEET_ID, SHEET_TAB and API_TOKEN in Script Properties');
    }

    const data = JSON.parse(e.postData.contents);
    if (data.api_token !== API_TOKEN) {
      return jsonResponse({ status: 'error', message: 'unauthorized' });
    }

    lock.waitLock(10000);
    const sheet = SpreadsheetApp.openById(SHEET_ID).getSheetByName(SHEET_TAB);
    if (!sheet) {
      throw new Error(`Sheet tab not found: ${SHEET_TAB}`);
    }

    const timestamp = new Date();
    sheet.appendRow([
      timestamp,
      String(data.dispositivo || 'unknown'),
      data.temperatura ?? '',
      data.humedad ?? '',
      data.presion_hpa ?? '',
      data.gas_raw ?? '',
      Boolean(data.alerta_gas),
      data.rssi ?? '',
    ]);

    const row = sheet.getLastRow();
    sheet.getRange(row, 1).setNumberFormat('yyyy-mm-dd hh:mm:ss');
    return jsonResponse({
      status: 'ok',
      row,
      timestamp: timestamp.toISOString(),
    });
  } catch (error) {
    return jsonResponse({ status: 'error', message: error.message });
  } finally {
    if (lock.hasLock()) {
      lock.releaseLock();
    }
  }
}

function testDoPost() {
  const response = doPost({
    postData: {
      contents: JSON.stringify({
        api_token: API_TOKEN,
        dispositivo: 'esp32-test',
        temperatura: 24.5,
        humedad: 62.3,
        presion_hpa: 1012.7,
        gas_raw: 342,
        alerta_gas: false,
        rssi: -65,
      }),
    },
  });
  Logger.log(response.getContent());
}
