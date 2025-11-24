#!/bin/sh

# Accept model parameter, default to 'default' if not provided
MODEL=${1:-default}

apt-get update && apt-get install -y curl unzip ca-certificates

echo "Downloading latest ixa-basic-transmission code..."
curl -L "https://github.com/CDCgov/ixa-basic-transmission/archive/refs/heads/main.zip" -o code.zip

echo "Unzipping..."
unzip -o code.zip
rm code.zip

mv ixa-basic-transmission-main ixa-basic-transmission
cd ixa-basic-transmission

curl https://raw.githubusercontent.com/CDCgov/ocio-certificates/refs/heads/main/data/min-cdc-bundle-ca.crt | tee /usr/local/share/ca-certificates/min-cdc-bundle-ca.crt >/dev/null
update-ca-certificates

cargo run -- --params params/${MODEL}.toml
