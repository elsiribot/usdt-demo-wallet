{
  description = "USDT Demo Wallet — Leptos (CSR/wasm) Fedimint client";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-25.11";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      fenix,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        fx = fenix.packages.${system};

        # Stable Rust with the wasm32 target. edition 2024 (matches the pinned
        # fedimint workspace) needs a recent stable; fenix `stable` tracks it.
        toolchain = fx.combine [
          fx.stable.rustc
          fx.stable.cargo
          fx.stable.clippy
          fx.stable.rust-src
          fx.stable.rust-analyzer
          fx.targets.wasm32-unknown-unknown.stable.rust-std
        ];

        # clang is needed to compile the C in `secp256k1-sys` for the
        # wasm32-unknown-unknown target (the `cc` crate invokes it with
        # `--target=wasm32-unknown-unknown`). llvm/clang can emit wasm.
        llvm = pkgs.llvmPackages_20;
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            toolchain
            pkgs.trunk # fetches a matching wasm-bindgen-cli + wasm-opt itself
            pkgs.binaryen # wasm-opt (belt-and-suspenders; trunk can also fetch it)
            pkgs.wasm-bindgen-cli
            llvm.clang # C compiler (host + wasm target)
            llvm.llvm # llvm-ar for the wasm target
            pkgs.pkg-config
            pkgs.openssl
            pkgs.just
            pkgs.git
          ];

          # Compile secp256k1-sys's C to wasm with the UNWRAPPED clang so the nix
          # cc-wrapper doesn't force host glibc includes (gnu/stubs-32.h) or
          # inject wasm-invalid hardening flags. The unwrapped clang ships no
          # builtin headers, so point -resource-dir at the wrapper's
          # resource-root, which provides stddef.h/stdint.h.
          hardeningDisable = [ "all" ];
          CC_wasm32_unknown_unknown = "${llvm.clang-unwrapped}/bin/clang";
          CFLAGS_wasm32_unknown_unknown = "-resource-dir ${llvm.clang}/resource-root";
          AR_wasm32_unknown_unknown = "${llvm.llvm}/bin/llvm-ar";

          shellHook = ''
            echo "usdt-demo-wallet dev shell — trunk build / trunk serve"
          '';
        };
      }
    );
}
