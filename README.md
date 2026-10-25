# Fondos animados

Vídeos en bucle como fondo de escritorio de GNOME (Wayland), decodificados por la GPU. Incluye
una app para gestionar la biblioteca, buscar y descargar fondos de varias webs, convertir vídeos
y un botón en Ajustes rápidos.

- **Extensión de GNOME Shell** (GNOME 48–51): pone el vídeo en el fondo del escritorio y en
  Actividades, lo pausa con ventanas a pantalla completa (o maximizadas, si quieres) y con la
  sesión bloqueada, y añade un menú en Ajustes rápidos para cambiar de fondo o pausarlo.
- **Reproductor** (`fondos-animados-renderer`, Rust + GStreamer): decodifica con VA-API
  (AMD/Intel) o NVDEC (NVIDIA) y entrega los fotogramas como DMABuf, sin pasar por la CPU. Un
  solo decodificador para todos los monitores.
- **App «Fondos animados»** (Rust, GTK 4 + libadwaita): biblioteca, búsqueda en
  [MotionBGs](https://motionbgs.com), [MoeWalls](https://moewalls.com) y
  [WallpaperWaifu](https://wallpaperwaifu.com) con filtros y paginación, navegador integrado
  (para [VSThemes](https://vsthemes.org), que tiene captcha, o cualquier otra web), conversión y
  estilo claro/oscuro/automático.
- **Scripts** (`fondo-animado*`): convertir, quitar audio, miniaturas y control desde la terminal.

## Instalar

```bash
curl -fsSL https://raw.githubusercontent.com/jlcampana/gnome-animated-wallpaper/main/install.sh | bash
```

Funciona en **Fedora** y **Ubuntu** (y derivadas). Instala las dependencias con dnf o apt (pide
la contraseña con sudo), baja los binarios de la última release para tu distribución (o compila
si no hay o no funcionan en tu sistema) y lo copia todo a tu usuario. Después, **cierra sesión y
vuelve a entrar** para que GNOME Shell cargue la extensión.

Opciones (`… | bash -s -- --si`):

| Opción | |
|---|---|
| `--si` | no preguntar nada |
| `--compilar` | compilar desde el código aunque haya binarios |
| `--sin-dependencias` | no tocar los paquetes del sistema |
| `--desinstalar` | quitarlo todo (la biblioteca de vídeos y los ajustes se quedan) |

Desde un clon del repositorio, `./install.sh` compila allí mismo (instala Rust y las cabeceras
de desarrollo si hace falta).

Dónde queda cada cosa:

| | |
|---|---|
| `~/.local/bin/` | `fondos-animados`, `fondos-animados-renderer` y los scripts `fondo-animado*` |
| `~/.local/share/gnome-shell/extensions/animated-wallpaper@jlcampana.github.io/` | la extensión |
| `~/Vídeos/Fondos animados/` | la biblioteca (se puede cambiar en dconf: `library-dir`) |
| `~/.cache/fondos-animados/` | miniaturas, vistas previas, descargas a medias y capacidades del hardware |
| `~/.local/share/fondos-animados/web/` | cookies y datos del navegador integrado |

### Decodificación por GPU

El reproductor usa los decodificadores VA-API de GStreamer (`vah264dec`, `vah265dec`,
`vavp9dec`, `vaav1dec`) o los de NVIDIA (`nvh264dec`…). Si no hay ninguno, funciona igual pero
decodificando con la CPU. El instalador avisa y explica qué instalar:

- **Fedora + AMD**: los drivers de Fedora no traen H.264/HEVC; con RPM Fusion activado,
  `sudo dnf swap mesa-va-drivers mesa-va-drivers-freeworld`.
- **Fedora + Intel**: `intel-media-driver` (RPM Fusion nonfree).
- **Ubuntu + Intel**: `sudo apt install intel-media-va-driver-non-free`.

Para comprobarlo: `fondo-animado-convertir --capacidades --forzar` (o la página Ajustes de la app).

## Uso

- **App**: «Fondos animados» en el menú de aplicaciones.
  - *Biblioteca*: clic en un vídeo para ponerlo de fondo; su menú (⋮) permite convertirlo,
    quitarle el audio, abrir su carpeta o borrarlo. Se pueden soltar vídeos o .zip en la ventana.
  - *Explorar*: elige la web, busca (en inglés) o elige categoría y resolución, y pasa de página
    con las flechas (cada página sustituye a la anterior para no acumular memoria). Clic en un
    resultado: vista previa y descarga.
  - *Navegador*: WebKit con sesión propia; cualquier descarga se importa sola a la biblioteca.
  - *Ajustes*: estado de la extensión, pausas, descargas, hardware detectado y estilo.
- **Ajustes rápidos**: botón «Fondo animado» (activar/desactivar) con menú: pausar, elegir vídeo
  de la biblioteca, abrir la app.
- **Terminal**:

  ```bash
  fondo-animado                  # estado
  fondo-animado lista
  fondo-animado poner frieren    # por nombre (parcial) o ruta
  fondo-animado siguiente | aleatorio
  fondo-animado on | off
  fondo-animado ajuste cover     # cover | contain | fill
  ```

### Al descargar o importar

1. Si el vídeo viene en .zip/.rar/.7z se descomprime. Los paquetes de **Wallpaper Engine** de tipo
   *vídeo* (`project.json` + vídeo) se importan con su título; los de tipo *escena* o *web* no se
   pueden reproducir y se avisa.
2. Se **quita el audio** sin recodificar (el fondo nunca suena: solo ocupa disco). Se puede
   desactivar en Ajustes, y el menú principal tiene «Quitar el audio de toda la biblioteca».
3. Si la GPU de **este equipo** no decodifica su códec (p. ej. AV1 en una Radeon RX 6400), se
   convierte al formato recomendado.
4. Opcional (*Ajustes → Adaptar a la pantalla*): recodificar a la resolución de tu pantalla y al
   formato recomendado. Por defecto solo si la GPU puede codificar (con CPU tarda).

### Hardware y formato recomendado

`fondo-animado-convertir --capacidades` detecta qué decodifica la GPU (decodificadores de
GStreamer) y qué codifica **de verdad** (hace una codificación de prueba de 0,2 s con cada
`*_vaapi` y `*_nvenc`: ffmpeg los lista aunque la GPU no los tenga, como pasa con la RX 6400, que
solo decodifica). El recomendado es el códec más eficiente (AV1 > HEVC > VP9 > H.264) que la GPU
decodifica y codifica; si no codifica nada, el mejor que decodifica, codificado con la CPU (x265
o x264). El resultado se guarda una semana por equipo, así que cada ordenador usa lo suyo.

### Convertir a mano

```bash
fondo-animado-convertir -r 2560x1080 -b 1 entrada.mp4 salida.mp4
```

`-r` resolución (`original` para no cambiarla), `-a` ajuste (`cubrir`, `encajar`, `estirar`),
`-f` fps, `-i`/`-d` inicio y duración en segundos, `-b` fundido del bucle en segundos (funde el
final con el principio para que no se note el salto), `-c` códec (`auto`, `av1`, `hevc`, `vp9`,
`h264`), `--cpu`, `-q` calidad. `fondo-animado-sin-audio vídeo…` y
`fondo-animado-miniatura vídeo salida.jpg` hacen lo que dicen.

## Depuración

| Qué | Cómo |
|---|---|
| Activar el registro de la extensión y del reproductor | `gsettings set org.gnome.shell.extensions.animated-wallpaper debug true` (o *Ajustes → Depuración* en la app). Reinicia el reproductor con `--debug`. |
| Ver ese registro | `journalctl --user -b -f \| grep -i 'fondos animados'` |
| Registro de la app (peticiones HTTP, scripts lanzados, capacidades) | `FONDOS_ANIMADOS_DEBUG=1 fondos-animados` (o la misma clave `debug`) |
| Abrir la app en una pestaña concreta | `FONDOS_ANIMADOS_PAGINA=explorar fondos-animados` (`biblioteca`, `explorar`, `navegador`, `ajustes`) |
| Comprobar que las webs siguen funcionando | `fondos-animados --probar-fuentes` (busca en cada web y enseña el enlace de descarga) |
| Probar el reproductor suelto, en una ventana | `fondos-animados-renderer --monitors 1280x540 --video vídeo.mp4 --debug` |
| Hardware de vídeo | `fondo-animado-convertir --capacidades --forzar` |
| Versiones | `fondos-animados --version`, `fondos-animados-renderer --version` |

Con `--debug`, el reproductor enseña la cadena de GStreamer que ha montado y las *caps* que llegan
al sink. Si aparece un decodificador `va…dec`/`nv…dec` y `memory:DMABuf`, la decodificación va
por la GPU sin copias. Por stdin acepta las órdenes `video <ruta>`, `fit cover|contain|fill`,
`pause`, `play` y `quit` (es como lo controla la extensión).

## Cómo funciona

Una aplicación no puede ponerse detrás del escritorio en Wayland, así que la extensión lanza el
reproductor como cliente Wayland propio (`Meta.WaylandClient`). Sus ventanas (una por monitor) se
marcan como de escritorio, se sacan de las listas de ventanas y se ocultan, y su contenido se
pinta con un `Clutter.Clone` dentro de cada `Meta.BackgroundActor`, interceptando
`BackgroundManager._createBackgroundActor`. Así aparece en el escritorio, en las miniaturas de
Actividades (con las esquinas redondeadas) y dondequiera que GNOME pinte el fondo. El bucle se
hace con *seeks* de segmento en GStreamer, sin vaciar el pipeline, para que no se note el salto.

No es posible añadir una sección propia en la app Configuración de GNOME (sus paneles van
compilados dentro). Los ajustes están en la app, en las preferencias de la extensión y en el menú
de Ajustes rápidos.

## Compilar

```bash
# Fedora
sudo dnf install cargo gtk4-devel libadwaita-devel gstreamer1-devel gstreamer1-plugins-base-devel webkitgtk6.0-devel libsoup3-devel
# Ubuntu
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libwebkitgtk-6.0-dev libsoup-3.0-dev

cargo build --release
cargo test                       # pruebas rápidas
cargo test -- --include-ignored  # además, descarga real de una web
herramientas/empaquetar.sh       # .tar.gz de la release (Fedora local + Ubuntu en podman)
```

```
app/          la app (Rust): biblioteca, explorar, navegador, ajustes, fuentes web, importación
renderer/     el reproductor (Rust + GStreamer)
extension/    la extensión de GNOME Shell (JavaScript) y el esquema de ajustes
scripts/      fondo-animado, -convertir, -sin-audio, -miniatura
data/         lanzador .desktop e icono
herramientas/ empaquetado de releases
```

## Licencia

MIT. Los fondos que se descargan son de sus autores y de cada web; respeta sus condiciones.
