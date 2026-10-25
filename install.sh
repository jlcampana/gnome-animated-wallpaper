#!/usr/bin/env bash
# Fondos animados — instalador para Fedora y Ubuntu (y derivadas).
#
#   curl -fsSL https://raw.githubusercontent.com/jlcampana/gnome-animated-wallpaper/main/install.sh | bash
#
# Opciones (con curl: «| bash -s -- --si»):
#   --si               no preguntar nada (acepta instalar dependencias)
#   --compilar         compilar desde el código aunque haya binarios para esta distro
#   --sin-dependencias no tocar los paquetes del sistema
#   --desinstalar      quitar la app, la extensión y los scripts (la biblioteca de vídeos se queda)
#
# Qué hace:
#   1. Instala las dependencias con dnf o apt (GTK 4, libadwaita, WebKitGTK 6, GStreamer con
#      VA-API y gtk4paintablesink, ffmpeg, 7z/unzip). Pide la contraseña con sudo.
#   2. Consigue los binarios: los de la release de GitHub para tu distro, los de esta carpeta
#      (si es una release descomprimida) o compilándolos (si es un clon o no hay binarios que
#      funcionen aquí).
#   3. Copia todo a tu usuario: ~/.local/bin, la extensión de GNOME Shell, el esquema de
#      ajustes, el lanzador y el icono. No instala nada fuera de tu home salvo los paquetes.
#
# Todo va dentro de main() para que bash lea el script entero antes de ejecutarlo: así
# funciona con «curl | bash» y las preguntas se leen de /dev/tty.
set -euo pipefail

REPO="${FONDOS_REPO:-jlcampana/gnome-animated-wallpaper}"
UUID="animated-wallpaper@jlcampana.github.io"
ID_APP="io.github.jlcampana.FondosAnimados"
ESQUEMA="org.gnome.shell.extensions.animated-wallpaper"
BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"
DATOS="${XDG_DATA_HOME:-$HOME/.local/share}"
EXT_DIR="$DATOS/gnome-shell/extensions/$UUID"
PROGRAMAS=(fondos-animados fondos-animados-renderer)
SCRIPTS=(fondo-animado fondo-animado-convertir fondo-animado-miniatura fondo-animado-sin-audio)

SI=0 COMPILAR=0 SIN_DEPS=0 DESINSTALAR=0
TMP=""

titulo() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
info() { printf '   %s\n' "$*"; }
aviso() { printf '\033[33m   ! %s\033[0m\n' "$*"; }
error() { printf '\033[31mERROR: %s\033[0m\n' "$*" >&2; exit 1; }

preguntar() {  # $1 = pregunta; devuelve 0 si sí
    ((SI)) && return 0
    local r=""
    if [[ -r /dev/tty ]]; then
        read -r -p "   $1 [S/n] " r </dev/tty || true
    fi
    [[ -z $r || $r =~ ^[sSyY] ]]
}

como_root() {
    if [[ $EUID -eq 0 ]]; then
        "$@"
    elif command -v sudo >/dev/null 2>&1; then
        sudo "$@"
    elif command -v pkexec >/dev/null 2>&1; then
        pkexec "$@"
    else
        error "hace falta sudo o pkexec para instalar paquetes"
    fi
}

# ── Distribución ────────────────────────────────────────────────────────────
DISTRO="" DISTRO_VER="" FAMILIA=""
detectar_distro() {
    [[ -f /etc/os-release ]] || error "no encuentro /etc/os-release"
    # shellcheck disable=SC1091
    . /etc/os-release
    DISTRO="${ID:-}" DISTRO_VER="${VERSION_ID:-}"
    case " ${ID:-} ${ID_LIKE:-} " in
        *" fedora "*|*" rhel "*) FAMILIA=fedora ;;
        *" ubuntu "*|*" debian "*) FAMILIA=ubuntu ;;
        *) FAMILIA="" ;;
    esac
    info "Sistema: ${PRETTY_NAME:-$DISTRO} ($(uname -m))"
    [[ -n $FAMILIA ]] || aviso "distribución no soportada oficialmente: no instalaré paquetes, solo los ficheros"
}

# ── Dependencias ────────────────────────────────────────────────────────────
DEPS_FEDORA=(gtk4 libadwaita webkitgtk6.0 gstreamer1 gstreamer1-plugins-base gstreamer1-plugins-good
             gstreamer1-plugins-bad-free gstreamer1-plugin-gtk4 p7zip p7zip-plugins unzip glib2)
DEPS_FEDORA_COMPILAR=(cargo rust gcc pkgconf-pkg-config gtk4-devel libadwaita-devel gstreamer1-devel
                      gstreamer1-plugins-base-devel webkitgtk6.0-devel libsoup3-devel)
DEPS_UBUNTU=(libgtk-4-1 libadwaita-1-0 libwebkitgtk-6.0-4 gstreamer1.0-tools gstreamer1.0-plugins-base
             gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-gtk4 ffmpeg p7zip-full unzip
             mesa-va-drivers libglib2.0-bin)
DEPS_UBUNTU_COMPILAR=(build-essential pkg-config curl libgtk-4-dev libadwaita-1-dev libgstreamer1.0-dev
                      libgstreamer-plugins-base1.0-dev libwebkitgtk-6.0-dev libsoup-3.0-dev)

instalar_paquetes() {  # paquetes…
    case "$FAMILIA" in
        fedora)
            local faltan=()
            for p in "$@"; do rpm -q "$p" >/dev/null 2>&1 || faltan+=("$p"); done
            ((${#faltan[@]})) || { info "Dependencias ya instaladas"; return 0; }
            info "Faltan: ${faltan[*]}"
            preguntar "¿Instalarlas con dnf?" || { aviso "sin dependencias, puede que no funcione"; return 0; }
            como_root dnf install -y "${faltan[@]}"
            ;;
        ubuntu)
            local faltan=() hay=()
            for p in "$@"; do dpkg -s "$p" >/dev/null 2>&1 || faltan+=("$p"); done
            ((${#faltan[@]})) || { info "Dependencias ya instaladas"; return 0; }
            info "Faltan: ${faltan[*]}"
            preguntar "¿Instalarlas con apt?" || { aviso "sin dependencias, puede que no funcione"; return 0; }
            como_root apt-get update -qq
            # Solo las que existen en esta versión (los nombres cambian entre versiones).
            for p in "${faltan[@]}"; do
                if apt-cache show "$p" >/dev/null 2>&1; then hay+=("$p"); else aviso "no existe el paquete $p aquí"; fi
            done
            ((${#hay[@]})) && como_root env DEBIAN_FRONTEND=noninteractive apt-get install -y "${hay[@]}"
            ;;
    esac
}

dependencias() {
    titulo "Dependencias"
    ((SIN_DEPS)) && { info "Saltado (--sin-dependencias)"; return; }
    [[ -n $FAMILIA ]] || return 0
    if [[ $FAMILIA == fedora ]]; then
        instalar_paquetes "${DEPS_FEDORA[@]}"
        # ffmpeg: el de RPM Fusion si ya está; si no, ffmpeg-free de Fedora.
        command -v ffmpeg >/dev/null 2>&1 || instalar_paquetes ffmpeg-free
    else
        instalar_paquetes "${DEPS_UBUNTU[@]}"
    fi
    consejo_va_api
}

# La GPU solo decodifica H.264/HEVC si el driver VA-API los trae (en Fedora, los de RPM Fusion).
consejo_va_api() {
    command -v gst-inspect-1.0 >/dev/null 2>&1 || return 0
    local decod=()
    for el in vah264dec vah265dec vavp9dec vaav1dec nvh264dec nvh265dec; do
        gst-inspect-1.0 --exists "$el" 2>/dev/null && decod+=("$el")
    done
    if ((${#decod[@]})); then
        info "Decodificación por GPU: ${decod[*]}"
        return
    fi
    aviso "GStreamer no ve decodificadores de vídeo por GPU: el fondo se decodificará con la CPU."
    local gpu
    gpu=$(lspci 2>/dev/null | grep -iE 'vga|3d|display' | head -1 || true)
    if [[ $FAMILIA == fedora ]]; then
        case "$gpu" in
            *AMD*|*ATI*) aviso "AMD en Fedora: activa RPM Fusion y ejecuta: sudo dnf swap mesa-va-drivers mesa-va-drivers-freeworld" ;;
            *Intel*) aviso "Intel en Fedora: activa RPM Fusion (nonfree) e instala intel-media-driver" ;;
            *NVIDIA*) aviso "NVIDIA: hace falta el driver propietario y gstreamer1-plugins-bad-freeworld (RPM Fusion)" ;;
        esac
    else
        case "$gpu" in
            *Intel*) aviso "Intel en Ubuntu: sudo apt install intel-media-va-driver-non-free" ;;
            *NVIDIA*) aviso "NVIDIA: hace falta el driver propietario (gstreamer1.0-plugins-bad trae nvcodec)" ;;
        esac
    fi
}

# ── Binarios ────────────────────────────────────────────────────────────────
ORIGEN=""   # carpeta con bin/, extension/ y data/

binarios_funcionan() {  # $1 = carpeta bin
    local b
    for b in "${PROGRAMAS[@]}"; do
        [[ -x $1/$b ]] || return 1
        if ldd "$1/$b" 2>&1 | grep -q 'not found'; then
            aviso "$b necesita bibliotecas que este sistema no tiene:"
            ldd "$1/$b" 2>&1 | grep 'not found' | sed 's/^/     /' | head -5
            return 1
        fi
        "$1/$b" --version >/dev/null 2>&1 || return 1
    done
}

compilar() {  # $1 = carpeta del código
    titulo "Compilando (unos minutos la primera vez)"
    if ((!SIN_DEPS)) && [[ -n $FAMILIA ]]; then
        if [[ $FAMILIA == fedora ]]; then
            instalar_paquetes "${DEPS_FEDORA_COMPILAR[@]}"
        else
            instalar_paquetes "${DEPS_UBUNTU_COMPILAR[@]}"
        fi
    fi
    local cargo
    cargo=$(command -v cargo || true)
    # Hace falta Rust >= 1.85 (edición 2024); si el del sistema es viejo, rustup en ~/.cargo.
    if [[ -n $cargo ]]; then
        local v
        v=$("$cargo" --version | awk '{print $2}')
        if [[ $(printf '%s\n1.85.0\n' "$v" | sort -V | head -1) != 1.85.0 ]]; then
            aviso "Rust $v es demasiado viejo"
            cargo=""
        fi
    fi
    if [[ -z $cargo ]]; then
        [[ -x $HOME/.cargo/bin/cargo ]] || {
            info "Instalando Rust con rustup en ~/.cargo"
            curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal --no-modify-path
        }
        cargo=$HOME/.cargo/bin/cargo
    fi
    (cd "$1" && "$cargo" build --release --locked)
    mkdir -p "$1/bin"
    for b in "${PROGRAMAS[@]}"; do cp "$1/target/release/$b" "$1/bin/"; done
    for s in "${SCRIPTS[@]}"; do cp "$1/scripts/$s" "$1/bin/"; done
}

descargar_release() {
    command -v curl >/dev/null 2>&1 || error "hace falta curl"
    TMP=$(mktemp -d "${TMPDIR:-/tmp}/fondos-animados.XXXXXX")
    trap 'rm -rf "$TMP"' EXIT
    titulo "Descargando de GitHub"
    local api url="" etiqueta=""
    api=$(curl -fsSL --retry 3 "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null || true)
    etiqueta=$(grep -o '"tag_name": *"[^"]*"' <<<"$api" | head -1 | cut -d'"' -f4 || true)
    local urls
    urls=$(grep -o '"browser_download_url": *"[^"]*"' <<<"$api" | cut -d'"' -f4 || true)
    if ((!COMPILAR)) && [[ -n $FAMILIA && $(uname -m) == x86_64 ]]; then
        # Primero la de la misma versión de la distro; si no, cualquiera de la familia.
        url=$(grep -E -- "-${DISTRO}${DISTRO_VER}-x86_64\.tar\.gz$" <<<"$urls" | head -1 || true)
        [[ -n $url ]] || url=$(grep -E -- "-${FAMILIA}[0-9.]*-x86_64\.tar\.gz$" <<<"$urls" | sort -V | tail -1 || true)
    fi
    if [[ -n $url ]]; then
        info "Release $etiqueta: $(basename "$url")"
        curl -fsSL --retry 3 "$url" -o "$TMP/release.tar.gz"
        mkdir -p "$TMP/release"
        tar -xzf "$TMP/release.tar.gz" -C "$TMP/release" --strip-components=1
        if binarios_funcionan "$TMP/release/bin"; then
            ORIGEN="$TMP/release"
            return
        fi
        aviso "los binarios de la release no funcionan en este sistema; compilo desde el código"
    else
        info "No hay binarios para este sistema en la release${etiqueta:+ $etiqueta}; compilo desde el código"
    fi
    local fuente="https://codeload.github.com/$REPO/tar.gz/refs/heads/main"
    [[ -n $etiqueta ]] && fuente="https://codeload.github.com/$REPO/tar.gz/refs/tags/$etiqueta"
    curl -fsSL --retry 3 "$fuente" -o "$TMP/codigo.tar.gz"
    mkdir -p "$TMP/codigo"
    tar -xzf "$TMP/codigo.tar.gz" -C "$TMP/codigo" --strip-components=1
    compilar "$TMP/codigo"
    ORIGEN="$TMP/codigo"
}

conseguir_binarios() {
    local aqui=""
    if [[ -n ${BASH_SOURCE[0]:-} && -f ${BASH_SOURCE[0]} ]]; then
        aqui="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    fi
    if [[ -n $aqui && -d $aqui/bin && -d $aqui/extension ]] && ((!COMPILAR)) && binarios_funcionan "$aqui/bin"; then
        titulo "Usando los binarios de esta carpeta"
        ORIGEN="$aqui"
    elif [[ -n $aqui && -f $aqui/Cargo.toml ]]; then
        compilar "$aqui"
        ORIGEN="$aqui"
    else
        descargar_release
    fi
}

# ── Instalar / desinstalar ──────────────────────────────────────────────────
instalar_ficheros() {
    titulo "Instalando en tu usuario"
    mkdir -p "$BIN_DIR"
    for b in "${PROGRAMAS[@]}" "${SCRIPTS[@]}"; do
        local f="$ORIGEN/bin/$b"
        [[ -f $f ]] || f="$ORIGEN/scripts/$b"
        install -m 755 "$f" "$BIN_DIR/$b"
    done
    info "Programas y scripts → $BIN_DIR"

    rm -rf "$EXT_DIR"
    mkdir -p "$(dirname "$EXT_DIR")"
    cp -r "$ORIGEN/extension/$UUID" "$EXT_DIR"
    glib-compile-schemas "$EXT_DIR/schemas"
    info "Extensión → $EXT_DIR"

    mkdir -p "$DATOS/glib-2.0/schemas"
    cp "$EXT_DIR/schemas/$ESQUEMA.gschema.xml" "$DATOS/glib-2.0/schemas/"
    glib-compile-schemas "$DATOS/glib-2.0/schemas"

    mkdir -p "$DATOS/applications" "$DATOS/icons/hicolor/scalable/apps"
    sed "s|^Exec=.*|Exec=$BIN_DIR/fondos-animados|" "$ORIGEN/data/$ID_APP.desktop" >"$DATOS/applications/$ID_APP.desktop"
    cp "$ORIGEN/data/$ID_APP.svg" "$DATOS/icons/hicolor/scalable/apps/"
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$DATOS/applications" || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t -f "$DATOS/icons/hicolor" 2>/dev/null || true
    info "Lanzador «Fondos animados» en el menú de aplicaciones"

    local videos
    videos=$(xdg-user-dir VIDEOS 2>/dev/null || true)
    # Sin carpeta de vídeos configurada, xdg-user-dir devuelve la home.
    [[ -z $videos || $videos == "$HOME" ]] && videos="$HOME/Videos"
    mkdir -p "$videos/Fondos animados"
    info "Biblioteca de vídeos: $videos/Fondos animados"

    mkdir -p "$DATOS/fondos-animados"
    "$BIN_DIR/fondos-animados" --version >"$DATOS/fondos-animados/version" 2>/dev/null || true
}

activar_extension() {
    titulo "Extensión de GNOME Shell"
    if gnome-extensions enable "$UUID" 2>/dev/null; then
        info "Activada"
        return
    fi
    # GNOME Shell aún no la conoce (solo mira las extensiones al iniciar sesión): se deja
    # en la lista de activas para que arranque en el próximo inicio de sesión.
    local activas
    activas=$(gsettings get org.gnome.shell enabled-extensions 2>/dev/null || echo "@as []")
    if [[ $activas != *"'$UUID'"* ]]; then
        if [[ $activas == "@as []" || $activas == "[]" ]]; then
            activas="['$UUID']"
        else
            activas="${activas%]}, '$UUID']"
        fi
        gsettings set org.gnome.shell enabled-extensions "$activas" 2>/dev/null || true
    fi
    aviso "Cierra sesión y vuelve a entrar para que GNOME Shell cargue la extensión."
}

desinstalar() {
    titulo "Desinstalando Fondos animados"
    gnome-extensions disable "$UUID" 2>/dev/null || true
    for b in "${PROGRAMAS[@]}" "${SCRIPTS[@]}"; do rm -f "$BIN_DIR/$b"; done
    rm -rf "$EXT_DIR" "$DATOS/fondos-animados" "$HOME/.cache/fondos-animados"
    rm -f "$DATOS/applications/$ID_APP.desktop" "$DATOS/icons/hicolor/scalable/apps/$ID_APP.svg"
    rm -f "$DATOS/glib-2.0/schemas/$ESQUEMA.gschema.xml"
    [[ -d $DATOS/glib-2.0/schemas ]] && glib-compile-schemas "$DATOS/glib-2.0/schemas" 2>/dev/null || true
    info "Hecho. Tu biblioteca de vídeos y los ajustes (dconf) se quedan."
    info "Para borrar los ajustes: dconf reset -f /org/gnome/shell/extensions/animated-wallpaper/"
}

main() {
    while (($#)); do
        case "$1" in
            --si|-y) SI=1 ;;
            --compilar) COMPILAR=1 ;;
            --sin-dependencias) SIN_DEPS=1 ;;
            --desinstalar) DESINSTALAR=1 ;;
            -h|--help) sed -n '2,22p' "${BASH_SOURCE[0]:-$0}" 2>/dev/null | sed 's/^# \{0,1\}//'; return 0 ;;
            *) error "opción desconocida: $1" ;;
        esac
        shift
    done

    printf '\033[1mFondos animados\033[0m — vídeos en bucle como fondo de escritorio de GNOME\n'
    if ((DESINSTALAR)); then
        desinstalar
        return
    fi
    [[ $EUID -ne 0 ]] || error "ejecútalo con tu usuario, no como root (pedirá sudo cuando haga falta)"
    detectar_distro
    dependencias
    conseguir_binarios
    instalar_ficheros
    activar_extension

    titulo "Hardware de vídeo de este equipo"
    "$BIN_DIR/fondo-animado-convertir" --capacidades --forzar 2>/dev/null | sed 's/^/   /' || true

    titulo "Listo"
    info "Abre «Fondos animados» desde el menú de aplicaciones (o ejecuta fondos-animados)."
    info "Desde la terminal: fondo-animado ayuda"
}

main "$@"
