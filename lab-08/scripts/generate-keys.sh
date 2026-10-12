#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
key_dir="${root_dir}/keys"

mkdir -p "${key_dir}"
umask 077

openssl ecparam -name prime256v1 -genkey -noout -out "${key_dir}/ecc_private.pem"
openssl ec -in "${key_dir}/ecc_private.pem" -pubout -out "${key_dir}/ecc_public.pem"

public_hex="$(
  openssl ec -in "${key_dir}/ecc_private.pem" -pubout -outform DER 2>/dev/null \
    | xxd -p -c 1000 \
    | sed 's/^3059301306072a8648ce3d020106082a8648ce3d03010703420004//'
)"

if [[ ${#public_hex} -ne 128 ]]; then
  echo "Could not extract a 64-byte P-256 public key" >&2
  exit 1
fi

echo "Keys created in ${key_dir}"
echo "ECC_PUBLIC_KEY_HEX=${public_hex}"
echo "Copy that line into ${root_dir}/.env"
echo "Copy ecc_private.pem only to the Node-RED host; never commit it."
