{
  description = "RicoqOS reproducible seL4 boot environment";

  nixConfig = {
    extra-substituters = [ "https://coliasgroup.cachix.org" ];
    extra-trusted-public-keys = [ "coliasgroup.cachix.org-1:vYRVaHS5FCjsGmVVXlzF5LaIWjeEK17W+MHxK886zIE=" ];
  };

  inputs = {
    nixpkgs.url = "github:coliasgroup/nixpkgs/53c4f440db47a727388f489cf349175c26c1448d";
    rust-sel4 = {
      url = "github:seL4/rust-sel4/891b59622306e94ad428193838f0d6214bb5c7aa";
      flake = false;
    };
    sel4 = {
      url = "github:seL4/seL4/6e7c3b733d296cfd88d5fbf635c96e447a882374";
      flake = false;
    };
    fenix = {
      url = "github:nix-community/fenix/9ba6d89cd7cb4d2b94953d56bea1d46e08aa53bd";
      flake = false;
    };
  };

  outputs = inputs@{ nixpkgs, ... }:
    let
      systems = [ "aarch64-darwin" "x86_64-darwin" "aarch64-linux" "x86_64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      environments = forAllSystems (system: import ./nix/environment.nix {
        inherit inputs system;
      });
    in {
      packages = forAllSystems (system: environments.${system}.packages);
      apps = forAllSystems (system: environments.${system}.apps);
      checks = forAllSystems (system: environments.${system}.checks);
      devShells = forAllSystems (system: { default = environments.${system}.shell; });
    };
}
