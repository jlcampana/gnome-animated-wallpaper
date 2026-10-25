#!/usr/bin/env bash
# Prepara los .tar.gz de la release en dist/:
#   fondos-animados-vX.Y.Z-fedoraNN-x86_64.tar.gz      (compilado aquí, si el equipo es Fedora)
#   fondos-animados-vX.Y.Z-ubuntuNN.NN-x86_64.tar.gz   (compilado en un contenedor con podman)
#
#   herramientas/empaquetar.sh [--solo-local] [--ubuntu VERSION]   (por defecto ubuntu 26.04)
#
# Cada paquete lleva bin/ (programas y scripts), extension/, data/, install.sh, README y LICENSE;
# su install.sh usa esos binarios sin compilar nada.
set -euo pipefail
cd "$(dirname "$0")/.."
RAIZ=$PWD
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
UBUNTU=26.04 SOLO_LOCAL=0
while (($#)); do
    case "$1" in
        --solo-local) SOLO_LOCAL=1 ;;
        --ubuntu) UBUNTU=$2; shift ;;
        *) echo "opción desconocida: $1" >&2; exit 1 ;;
    esac
    shift
done
mkdir -p dist

paquete() {  # $1 = etiqueta (fedora44…), $2 = carpeta con los binarios compilados
    local nombre="fondos-animados-v$VERSION-$1-x86_64" tmp
    tmp=$(mktemp -d)
    local d="$tmp/$nombre"
    mkdir -p "$d/bin" "$d/extension" "$d/data"
    cp "$2/fondos-animados" "$2/fondos-animados-renderer" "$d/bin/"
    cp scripts/fondo-animado* "$d/bin/"
    cp -r extension/*@jlcampana.github.io "$d/extension/"
    rm -f "$d"/extension/*/schemas/gschemas.compiled
    cp data/* "$d/data/"
    cp install.sh README.md LICENSE "$d/"
    tar -czf "dist/$nombre.tar.gz" -C "$tmp" "$nombre"
    rm -rf "$tmp"
    echo "dist/$nombre.tar.gz ($(du -h "dist/$nombre.tar.gz" | cut -f1))"
}

# Local (Fedora). os-release en una subshell: define VERSION y la pisaría.
cargo build --release --locked
paquete "$(. /etc/os-release && echo "$ID$VERSION_ID")" target/release

((SOLO_LOCAL)) && exit 0

# Ubuntu, en contenedor (el código se copia: el contenedor no toca el repo).
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/fondos-animados-build/ubuntu$UBUNTU"
mkdir -p "$CACHE/src" "$CACHE/target" "$CACHE/cargo"
rsync -a --delete --exclude target --exclude dist --exclude .git "$RAIZ/" "$CACHE/src/"
# Reutiliza lo ya compilado en el contenedor si no ha cambiado el código.
podman run --rm \
    -v "$CACHE/src:/src:Z" -v "$CACHE/target:/target:Z" -v "$CACHE/cargo:/cargo:Z" \
    "docker.io/library/ubuntu:$UBUNTU" bash -c '
set -e
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq curl ca-certificates build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libwebkitgtk-6.0-dev libsoup-3.0-dev >/dev/null
export CARGO_HOME=/cargo RUSTUP_HOME=/cargo/rustup
[ -x /cargo/bin/cargo ] || curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal --no-modify-path
cd /src && CARGO_TARGET_DIR=/target /cargo/bin/cargo build --release --locked'
paquete "ubuntu$UBUNTU" "$CACHE/target/release"
