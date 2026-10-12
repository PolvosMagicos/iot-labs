{
  description = "Rust environment for IoT Laboratory 10";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
    in {
      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          rustup
          gcc
          pkg-config
          cmake
          ninja
          python3
          git
          curl
          perl
          gnumake
          openssl
          libusb1
          udev
          zlib
          libxml2
          ncurses
          mosquitto
        ];

        shellHook = ''
          export LIBCLANG_PATH=${pkgs.llvmPackages.libclang.lib}/lib
          export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath [
            pkgs.openssl
            pkgs.libusb1
            pkgs.udev
            pkgs.zlib
            pkgs.libxml2
            pkgs.ncurses
          ]}:$LD_LIBRARY_PATH
          echo "IoT Lab 10 development shell"
          echo "Backend: cd backend && cargo run"
          echo "ESP32: source \$HOME/export-esp.sh, then cd esp32-node"
        '';
      };
    };
}
