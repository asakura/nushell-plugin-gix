{
  description = "nu_plugin_gix: a Nushell plugin exposing gix (gitoxide)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      rust-overlay,
      crane,
      git-hooks,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        craneLib = (crane.mkLib pkgs).overrideToolchain (_: rustToolchain);

        src = craneLib.cleanCargoSource ./.;

        commonArgs = {
          inherit src;
          strictDeps = true;
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        nu_plugin_gix = pkgs.callPackage ./package.nix { inherit crane; };

        # Env prefix that prevents ~/.cargo/config.toml from leaking into hook
        # invocations. CARGO_ENCODED_RUSTFLAGS (tier 1 in cargo's 3-tier
        # precedence) beats config-level [target.xxx].rustflags (tier 3),
        # neutralising nightly-only -Z flags.  The explicit linker overrides
        # the config-level [target.xxx].linker, avoiding pinned store paths or
        # absent system binaries.
        cargoHookEnv = "CARGO_ENCODED_RUSTFLAGS='' CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=${pkgs.stdenv.cc}/bin/cc";

        gitHooksCheck = git-hooks.lib.${system}.run {
          src = ./.;
          package = pkgs.prek;
          hooks = {
            cargo-check = {
              enable = true;
              entry = "env ${cargoHookEnv} ${rustToolchain}/bin/cargo check";
              pass_filenames = false;
            };
            clippy = {
              enable = true;
              entry = "env ${cargoHookEnv} ${rustToolchain}/bin/cargo clippy --";
              pass_filenames = false;
            };
            rustfmt.enable = true;
            taplo.enable = true;
            nixfmt.enable = true;
            shfmt.enable = true;
            shellcheck.enable = true;

            cargo-deny = {
              enable = true;
              entry = "${pkgs.cargo-deny}/bin/cargo-deny check";
              files = "(Cargo\\.lock|deny\\.toml)$";
              pass_filenames = false;
            };
          };
        };

        # Static, fully self-contained Linux binaries for release artifacts.
        # The *build* platform stays the normal glibc x86_64-linux toolchain;
        # only the final crate output is cross-compiled to a musl target,
        # statically linked, via the matching `pkgsCross` C toolchain as
        # the linker. This avoids `pkgsStatic`, which would also make rustc
        # itself a static-musl binary - that combination crashes proc-macro
        # build scripts.
        mkMuslPackage =
          crossPkgs: targetTriple:
          let
            targetEnvVar = pkgs.lib.toUpper (builtins.replaceStrings [ "-" ] [ "_" ] targetTriple);
            crossCC = "${crossPkgs.stdenv.cc.targetPrefix}cc";
            muslArgs = commonArgs // {
              pname = "nu_plugin_gix";
              doCheck = false;
              CARGO_BUILD_TARGET = targetTriple;
              "CARGO_TARGET_${targetEnvVar}_LINKER" = crossCC;
              "CC_${builtins.replaceStrings [ "-" ] [ "_" ] targetTriple}" = crossCC;
              RUSTFLAGS = "-C target-feature=+crt-static";
              depsBuildBuild = [ crossPkgs.stdenv.cc ];
            };
            muslCargoArtifacts = craneLib.buildDepsOnly muslArgs;
          in
          craneLib.buildPackage (muslArgs // { cargoArtifacts = muslCargoArtifacts; });
      in
      {
        packages = {
          inherit nu_plugin_gix;
          default = nu_plugin_gix;
        }
        // pkgs.lib.optionalAttrs (system == "x86_64-linux") {
          nu_plugin_gix-x86_64-linux-musl = mkMuslPackage pkgs.pkgsCross.musl64 "x86_64-unknown-linux-musl";
          nu_plugin_gix-aarch64-linux-musl = mkMuslPackage pkgs.pkgsCross.aarch64-multiplatform-musl "aarch64-unknown-linux-musl";
        };

        checks = {
          fmt = craneLib.cargoFmt { inherit src; };
          clippy = craneLib.cargoClippy (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--workspace --all-targets -- -D warnings";
            }
          );
          git-hooks = gitHooksCheck;
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ nu_plugin_gix ];
          RUSTFLAGS = "";
          shellHook = ''
            ${gitHooksCheck.shellHook}
          '';
          packages = with pkgs; [
            rustToolchain
            git
            cargo-deny
            cargo-llvm-cov
            cargo-nextest
            git-cliff
            nixfmt
            shfmt
            shellcheck
            rtk
          ];
        };

        formatter = pkgs.nixfmt-tree;
      }
    );
}
