#!/usr/bin/env bash
# Publica una versión: crea la etiqueta anotada vX.Y.Z (versión de Cargo.toml) y la sube.
# GitHub Actions compila en Fedora y Ubuntu, empaqueta y publica la release.
#
#   herramientas/publicar.sh NOTAS.md
#
# NOTAS.md es el texto de la release en Markdown: el primer párrafo es el resumen y el resto
# (normalmente una lista) sale bajo «Novedades». Por ejemplo:
#
#   La app enseña su versión, y el Media Controller cambia de aspecto.
#
#   - **Versión en Ajustes**: al final de la página *Ajustes* aparece la versión de la app.
set -euo pipefail
cd "$(dirname "$0")/.."

NOTAS=${1:?uso: herramientas/publicar.sh NOTAS.md}
[[ -s $NOTAS ]] || { echo "No existe o está vacío: $NOTAS" >&2; exit 1; }
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
[[ -z "$(git status --porcelain)" ]] || { echo "Hay cambios sin confirmar" >&2; exit 1; }
git rev-parse -q --verify "refs/tags/v$VERSION" >/dev/null && { echo "La etiqueta v$VERSION ya existe" >&2; exit 1; }

# El asunto de la etiqueta no sale en la release; el cuerpo, sí.
git tag -a "v$VERSION" -F <(printf 'Versión %s\n\n' "$VERSION"; cat "$NOTAS")
git push origin main "v$VERSION"
echo "==> v$VERSION subida. La release sale en unos minutos:"
echo "    https://github.com/jlcampana/gnome-animated-wallpaper/actions"
