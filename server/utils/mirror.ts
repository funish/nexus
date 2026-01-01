/**
 * Mirror registry configuration
 * Maps registry names to their upstream URLs
 *
 * Usage: /mirror/:registry/*
 * Example: /mirror/npm/jquery
 */
export const mirrorRegistries: Record<string, string> = {
  // JavaScript/TypeScript
  npm: "https://registry.npmjs.org",
  jsr: "https://npm.jsr.io",

  // Python
  pypi: "https://pypi.org",

  // Rust
  crates: "https://crates.io",

  // Go
  go: "https://proxy.golang.org",

  // Java
  maven: "https://repo1.maven.org/maven2",
  gradle: "https://plugins.gradle.org",

  // PHP
  composer: "https://repo.packagist.org",

  // Docker
  docker: "https://registry-1.docker.io",
  ghcr: "https://ghcr.io",
  quay: "https://quay.io",

  // Flutter/Dart
  pub: "https://pub.dev",

  // Haskell
  hackage: "https://hackage.haskell.org",

  // Julia
  julia: "https://pkg.julialang.org",

  // R
  cran: "https://cran.r-project.org",

  // Lua
  luarocks: "https://luarocks.org",

  // Nim
  nimble: "https://nimble.directory",

  // Elixir/Erlang
  hex: "https://hex.pm",

  // Clojure
  clojars: "https://clojars.org",

  // C++
  conan: "https://center.conan.io",

  // macOS
  homebrew: "https://formulae.brew.sh/api",

  // Windows
  chocolatey: "https://community.chocolatey.org/api/v2",

  // Debian/Ubuntu
  debian: "http://deb.debian.org/debian",
  ubuntu: "http://archive.ubuntu.com/ubuntu",

  // RHEL/Fedora
  fedora: "https://dl.fedoraproject.org/pub/fedora/linux",
  epel: "https://dl.fedoraproject.org/pub/epel",

  // Other Linux
  arch: "https://geo.mirror.pkgbuild.com",
  alpine: "https://dl-cdn.alpinelinux.org/alpine",
  gentoo: "https://distfiles.gentoo.org",
  openwrt: "https://downloads.openwrt.org",

  // Anaconda
  anaconda: "https://repo.anaconda.com",
  condaforge: "https://conda.anaconda.org/conda-forge",

  // Perl
  cpan: "https://www.cpan.org",

  // LaTeX
  ctan: "https://ctan.org",

  // Databases
  postgresql: "https://ftp.postgresql.org/pub",
  mysql: "https://downloads.mysql.com",

  // Package managers
  nix: "https://cache.nixos.org",
  guix: "https://guix.gnu.org",
};

export type MirrorRegistry = keyof typeof mirrorRegistries;
