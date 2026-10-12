// Node-RED Function node.
// Configure external modules: crypto -> crypto, fs -> fs.
const privateKeyPath = env.get("ECC_PRIVATE_KEY_PATH");

if (!privateKeyPath) {
  node.error("ECC_PRIVATE_KEY_PATH is not configured", msg);
  return null;
}

try {
  const raw = Buffer.from(String(msg.payload), "base64");

  if (raw.length < 82) {
    throw new Error(`ciphertext is too short (${raw.length} bytes)`);
  }
  if (raw[0] !== 1) {
    throw new Error(`unsupported wire-format version ${raw[0]}`);
  }

  const ephemeralXy = raw.subarray(1, 65);
  const nonce = raw.subarray(65, 81);
  const ciphertext = raw.subarray(81);
  const ephemeralSec1 = Buffer.concat([Buffer.from([0x04]), ephemeralXy]);
  const spkiPrefix = Buffer.from(
    "3059301306072a8648ce3d020106082a8648ce3d030107034200",
    "hex",
  );

  const privateKey = crypto.createPrivateKey(
    fs.readFileSync(privateKeyPath, "utf8"),
  );
  const ephemeralPublicKey = crypto.createPublicKey({
    key: Buffer.concat([spkiPrefix, ephemeralSec1]),
    format: "der",
    type: "spki",
  });
  const shared = crypto.diffieHellman({
    privateKey,
    publicKey: ephemeralPublicKey,
  });

  const aesKey = crypto
    .createHash("sha256")
    .update(shared)
    .update(ephemeralXy.subarray(0, 32))
    .digest()
    .subarray(0, 16);

  const decipher = crypto.createDecipheriv("aes-128-ctr", aesKey, nonce);
  const plaintext = Buffer.concat([
    decipher.update(ciphertext),
    decipher.final(),
  ]);

  msg.encrypted_payload = msg.payload;
  msg.payload = JSON.parse(plaintext.toString("utf8"));
  return msg;
} catch (error) {
  node.error(`ECIES decryption failed: ${error.message}`, msg);
  return null;
}
