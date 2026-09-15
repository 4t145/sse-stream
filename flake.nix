{
  description = "Development environment for sse-stream (Rust stable)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            name = "sse-stream-dev";

            # Needed to build dev-dependencies on Linux:
            # reqwest (default features) -> native-tls -> openssl-sys
            nativeBuildInputs = with pkgs; [
              pkg-config
            ];

            buildInputs =
              with pkgs;
              [ openssl ]
              ++ lib.optionals stdenv.hostPlatform.isDarwin [
                darwin.apple_sdk.frameworks.Security
                darwin.apple_sdk.frameworks.SystemConfiguration
              ];

            packages = with pkgs; [
              # Rust stable toolchain, pinned via flake.lock
              cargo
              rustc
              clippy
              rustfmt

              # Dev tooling
              rust-analyzer
              cargo-watch
              nil # Nix LSP, picked up by Zed's Nix extension via direnv PATH
            ];

            # Let rust-analyzer find the std library sources
            env.RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";

            shellHook = ''
              echo "🦀 sse-stream dev shell"
              echo "   $(rustc --version), $(cargo --version)"
            '';
          };
        }
      );

      formatter = forAllSystems (system: (import nixpkgs { inherit system; }).nixfmt);
    };
}
