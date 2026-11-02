{
  description = "Pure Wayland (niri) work time & window screenshot tracker with SCTK indicator";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    systems.url = "github:nix-systems/default";
  };

  outputs = { self, nixpkgs, systems, ... }:
    let
      eachSystem = nixpkgs.lib.genAttrs (import systems);
    in
    {
      packages = eachSystem (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.callPackage ./default.nix { };
          worktrack = self.packages.${system}.default;
        });

      apps = eachSystem (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/worktrack";
        };
        worktrack = self.apps.${system}.default;
      });

      devShells = eachSystem (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = import ./shell.nix { inherit pkgs; };
        });

      overlays.default = final: _prev: {
        worktrack = final.callPackage ./default.nix { };
      };

      nixosModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.programs.worktrack;
        in
        {
          options.programs.worktrack = {
            enable = lib.mkEnableOption "Worktrack screenshot time tracker for Wayland/niri";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              defaultText = lib.literalExpression "inputs.worktrack.packages.\${pkgs.system}.default";
              description = "The worktrack package to use.";
            };
          };

          config = lib.mkIf cfg.enable {
            environment.systemPackages = [ cfg.package ];
          };
        };

      nixosModules.worktrack = self.nixosModules.default;
    };
}
