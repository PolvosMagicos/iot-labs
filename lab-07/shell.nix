{ pkgs ? import <nixpkgs> {
    config.allowUnfreePredicate = pkg:
      builtins.elem (pkgs.lib.getName pkg) [
        "mqtt-explorer"
      ];
  }
}:

pkgs.mkShell {
  packages = with pkgs; [
    rustup
    espflash

    # ESP-IDF build tools
    git
    cmake
    ninja
    python3
    pkg-config

    # Lab tools
    mosquitto
    podman
    podman-compose
    mqtt-explorer

    # Native libs often needed by esp-idf / bindgen / flashing
    openssl
    libusb1
    zlib
    libxml2
    ncurses
  ];

  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
    pkgs.stdenv.cc.cc.lib
    pkgs.zlib
    pkgs.libxml2
    pkgs.ncurses
    pkgs.openssl
    pkgs.libusb1
  ];

  shellHook = ''
    echo "ESP32 Rust lab shell loaded"
    echo "Available: cargo, espflash, mosquitto_pub, mosquitto_sub, mqtt-explorer, podman-compose"
  '';
}
