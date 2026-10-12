use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use p256::{elliptic_curve::sec1::ToSec1Point, SecretKey};

fn generate_secret_key() -> Result<SecretKey> {
    loop {
        let mut secret_bytes = [0_u8; 32];

        getrandom::fill(&mut secret_bytes)
            .map_err(|error| anyhow!("failed to obtain random bytes: {error}"))?;

        // Not every arbitrary 256-bit value is a valid P-256 scalar.
        // Retry until the generated value is accepted.
        if let Ok(secret_key) = SecretKey::from_slice(&secret_bytes) {
            return Ok(secret_key);
        }
    }
}

fn main() -> Result<()> {
    let private_key = generate_secret_key()?;
    let public_key = private_key.public_key();

    let private_bytes = private_key.to_bytes();
    let public_bytes = public_key.to_sec1_point(false);

    println!(
        "SERVER_PRIVATE_KEY_B64={}",
        STANDARD.encode(private_bytes.as_slice())
    );

    println!(
        "SERVER_PUBLIC_KEY_B64={}",
        STANDARD.encode(public_bytes.as_bytes())
    );

    Ok(())
}
