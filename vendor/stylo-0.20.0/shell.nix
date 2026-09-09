with import (builtins.fetchTarball {
  url = "https://github.com/NixOS/nixpkgs/archive/46ae0210ce163b3cba6c7da08840c1d63de9c701.tar.gz";
}) {};
stdenv.mkDerivation rec {
  name = "style-sync-shell";
}
