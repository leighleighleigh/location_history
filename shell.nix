let
  rustOverlay = builtins.fetchTarball "https://github.com/oxalica/rust-overlay/archive/master.tar.gz";
  pkgs = import <nixpkgs> {
    overlays = [ (import rustOverlay) ];
  };

  rust = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

#  rust = pkgs.rust-bin.stable.latest.default.override {
#    extensions = [
#      "rust-src" # for rust-analyzer
#    ];
#  };
in
  pkgs.mkShell rec {
    buildInputs = [ rust ] ++ (with pkgs; [
      bacon 
      lunarvim
      gcc 
      pkg-config
      rust-analyzer
      stdenv.cc 
      cargo-binstall
      libxkbcommon
      libGL
      wayland
      xorg.libXrandr
      xorg.libXcursor
      xorg.libX11
      xorg.libXi
    ]);

    LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath buildInputs}";

    shellHook = ''
        export WINIT_UNIX_BACKEND=wayland
        export WGPU_BACKEND=gl
        export PS1="''${debian_chroot:+($debian_chroot)}\[\033[01;39m\]\u@\h\[\033[00m\]:\[\033[01;34m\]\W\[\033[00m\]\$ "
        export PS1="(nix-rs)$PS1"
        export LD_LIBRARY_PATH="''${LD_LIBRARY_PATH}:${LD_LIBRARY_PATH}"
    '';
  }
