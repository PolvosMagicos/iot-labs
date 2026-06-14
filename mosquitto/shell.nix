{ pkgs ? import <nixpkgs> {
    config.allowUnfree = true;
  }
}:

pkgs.mkShell {
  packages = with pkgs; [
    mosquitto
    mqtt-explorer
  ];

  shellHook = ''
    echo "Mosquitto shell loaded"
    echo "Available: mosquitto_sub, mosquitto_pub"
  '';
}
