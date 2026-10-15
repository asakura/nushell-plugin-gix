{
  pkgs,
  crane,
  rustToolchain ? pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml,
  craneLib ? crane.mkLib pkgs,
}:
let
  libCrane =
    if rustToolchain != null then craneLib.overrideToolchain (_: rustToolchain) else craneLib;

  src = libCrane.cleanCargoSource ./.;

  commonArgs = {
    inherit src;
    strictDeps = true;
  };

  cargoArtifacts = libCrane.buildDepsOnly commonArgs;
in
libCrane.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;
    pname = "nu_plugin_gix";
    doCheck = false;
    meta.mainProgram = "nu_plugin_gix";
  }
)
