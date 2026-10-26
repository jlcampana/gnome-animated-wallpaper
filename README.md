# Fondos animados

Vídeos en bucle como fondo de escritorio de GNOME (Wayland), decodificados por la GPU. Incluye
una app para gestionar la biblioteca, buscar y descargar fondos de varias webs, convertir vídeos
y un botón en Ajustes rápidos.

- **Extensión de GNOME Shell** (GNOME 48–51): pone el vídeo en el fondo del escritorio y en
  Actividades (allí se queda en pausa para que la animación vaya fluida), lo pausa con ventanas
  a pantalla completa (o maximizadas, si quieres), con la sesión bloqueada y con la pantalla
  apagada, y añade un menú en Ajustes rápidos para cambiar de fondo o pausarlo.
- **Reproductor** (`fondos-animados-renderer`, Rust + GStreamer): decodifica con VA-API
  (AMD/Intel) o NVDEC (NVIDIA) y entrega los fotogramas como DMABuf, sin pasar por la CPU. Un
  solo decodificador para todos los monitores.
- **App «Fondos animados»** (Rust, GTK 4 + libadwaita): biblioteca, búsqueda en
  [MotionBGs](https://motionbgs.com), [MoeWalls](https://moewalls.com) y
  [WallpaperWaifu](https://wallpaperwaifu.com) con filtros y paginación, navegador integrado
  (para [VSThemes](https://vsthemes.org), que tiene captcha, o cualquier otra web), conversión y
  estilo claro/oscuro/automático.
- **Scripts** (`fondo-animado*`): convertir, quitar audio, miniaturas y control desde la terminal.
- **Media Controller** ya no va aquí: se ha movido a su propio repo,
  [gnome-extension-mediaplayer](https://github.com/jlcampana/gnome-extension-mediaplayer) (ver [abajo](#media-controller)).

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
| `~/Vídeos/Fondos animados/30 fps/` | versiones de 30 fps (mismo nombre que el original) |
| `~/.local/share/fondos-animados/biblioteca.db` | SQLite con fps, resolución, códec… de cada vídeo (se rehace sola si se borra) |
| `~/.local/share/fondos-animados/descargas.db` | SQLite con la cola de descargas y conversiones (sobrevive a cierres y caídas) |
| `~/.config/systemd/user/fondos-animados-descargas.service` | servicio de usuario del gestor de descargas |
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
    El desplegable junto al buscador filtra por fps (más de 30, a 30 o menos, con las dos
    versiones, sin versión de 30). Ver [Versiones de 30 fps](#versiones-de-30-fps).
  - *Explorar*: elige la web, busca (en inglés) o elige categoría y resolución, y pasa de página
    con las flechas o escribe el número de la página a la que saltar (cada página sustituye a la
    anterior para no acumular memoria). Clic en un resultado: vista previa y descarga.
  - *Navegador*: WebKit con sesión propia; cualquier descarga se importa sola a la biblioteca.
  - *Descargas y conversiones* (botón de la barra de título): ver [Cola de descargas y
    conversiones](#cola-de-descargas-y-conversiones).
  - *Ajustes*: estado de la extensión, pausas, descargas, hardware detectado y estilo.
- **Ajustes rápidos**: botón «Fondo animado» (activar/desactivar) con menú: pausar, elegir vídeo
  de la biblioteca, abrir la app.
- **Terminal**:

  ```bash
  fondo-animado                  # estado
  fondo-animado lista            # un fondo por línea: [original + 30 fps], * el actual
  fondo-animado poner frieren    # por nombre (parcial) o ruta, en la versión preferida
  fondo-animado poner frieren 30fps   # o en una concreta: original | 30fps
  fondo-animado version 30fps    # versión preferida (el fondo actual pasa a ella)
  fondo-animado siguiente | aleatorio
  fondo-animado on | off
  fondo-animado ajuste cover     # cover | contain | fill
  ```

### Cola de descargas y conversiones

Lo que se descarga desde *Explorar*, lo que se importa y las conversiones (versiones de 30 fps y
el diálogo *Convertir / recortar…*) entran en una cola guardada en SQLite.
El trabajo lo hace el **gestor de descargas**, un proceso aparte (`fondos-animados
--gestor-descargas`) que va como servicio de systemd de usuario
(`fondos-animados-descargas.service`, lo instala y activa `install.sh`):

- Si se cierra o se cae la app, el gestor sigue.
- Si se cae el gestor, systemd lo vuelve a arrancar y continúa las descargas a medias desde
  donde iban (y la app también lo arranca si lo echa en falta).
- Al iniciar sesión arranca solo y sigue con lo que quedara en la cola (por ejemplo, tras
  apagar el equipo). Cuando no queda nada que hacer, se cierra.
- Sin systemd de usuario, la app lo lanza ella misma.

- Las descargas van **de una en una**, en el orden de la lista; mientras se importa una, ya se
  está bajando la siguiente.
- Cada una se puede **pausar** (se reanuda donde se quedó), **subir o bajar** en la cola,
  **cancelar**, **reintentar** (si falló o se canceló) y **quitar** de la lista.
- Un vídeo que ya está en la cola no se añade otra vez; si estaba con error o cancelado, se
  vuelve a intentar.
- Las **conversiones** van aparte, también de una en una, para que una larga no deje esperando a
  las importaciones. Su fila lleva un icono de cámara (las descargas, el de descarga; los
  ficheros importados, el de abrir) y enseña el porcentaje y el codificador (GPU o CPU). Mientras
  esperan se pueden pausar, reordenar o cancelar; en marcha, solo cancelar. Si el gestor se cae a
  mitad, la conversión vuelve a empezar al arrancar otra vez. No hace falta dejar la app abierta.
- Las descargas del *Navegador* las hace WebKit dentro de la app (se pierden si se cierra); al
  terminar, entran en la cola para importarse.

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
5. Opcional (*Ajustes → Crear también la versión de 30 fps*): si el vídeo va a más de 30 fps,
   su versión de 30 entra en la cola de conversiones.

### Versiones de 30 fps

Un vídeo de 60 fps puede tener además una versión de 30, que hace trabajar menos a la GPU. Va en
la subcarpeta `30 fps` de la biblioteca con el mismo nombre que el original (en .mp4), y el
original se conserva. La app y el menú de Ajustes rápidos enseñan una sola entrada por fondo
(con la etiqueta «60 · 30 fps») y usan la versión elegida en *Ajustes → Versión que se usa de
fondo* (clave `preferred-version`: `original` o `30fps`); al cambiarla, el fondo actual pasa a
esa versión. El menú ⋮ de cada fondo permite usar una u otra, y crear o borrar la de 30 fps; el
menú principal tiene «Crear versiones de 30 fps de toda la biblioteca» (pide confirmación). Todas
van a la [cola](#cola-de-descargas-y-conversiones): se ve el avance de cada una, se pueden
cancelar y siguen aunque se cierre la app. Al terminar, si es el fondo actual y toca la de 30,
el fondo pasa a ella.

Con *Ajustes → Usar la de 30 fps con batería* (clave `battery-30fps`), al desenchufar el portátil
el fondo pasa a su versión de 30 fps (si la tiene) y al enchufarlo vuelve a la preferida. Lo
decide la extensión con UPower; la app y `fondo-animado` siguen el mismo criterio al elegir fondo.

Su calidad se elige en *Ajustes → Calidad de las versiones de 30 fps* (clave `quality-30fps`,
el `-q` de `fondo-animado-convertir`): Alta (24), Normal (28, por defecto) o Ligera (32). Para
un fondo no hace falta mucha: con x265, un 1440p de 60 fps a 20 Mbps queda a 30 fps en 2,2 Mbps
con la Alta (VMAF 91), 1,2 con la Normal (87) y 0,7 con la Ligera (81). La calidad está en la
escala de H.264/HEVC (0-51); para AV1 y VP9 por GPU, que cuantizan en 0-255, el script la pasa
a (calidad + 8) × 4. Cambiar la calidad no rehace las versiones que ya
existen: «Rehacer la versión de 30 fps» (menú ⋮) o «Rehacer todas las versiones de 30 fps» (menú
principal) las vuelven a crear con la calidad elegida. La tarjeta del fondo activo lleva una
etiqueta verde con la versión que se está reproduciendo («Activa: 30 fps»).

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
| Registro del gestor de descargas | `journalctl --user -u fondos-animados-descargas -f` (si lo lanzó la app sin systemd: `~/.cache/fondos-animados/gestor-descargas.log`) |
| Ver la cola de descargas | `sqlite3 ~/.local/share/fondos-animados/descargas.db 'SELECT id, nombre, estado, hecho, total, mensaje FROM trabajos ORDER BY orden'` |
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

Para publicar una versión: sube `version` en `Cargo.toml`, haz commit y ejecuta
`herramientas/publicar.sh NOTAS.md`. Crea la etiqueta anotada `vX.Y.Z` con el texto de `NOTAS.md`
(el primer párrafo es el resumen; el resto sale bajo «Novedades») y la sube. GitHub Actions compila
en contenedores de Fedora 44 y Ubuntu 26.04, empaqueta con `herramientas/empaquetar.sh` y publica la
release con la tabla de versiones de GTK, libadwaita y GStreamer de cada paquete. En cada push a
`main` el mismo workflow compila y pasa las pruebas en las dos distribuciones.

```
app/          la app (Rust): biblioteca, explorar, navegador, ajustes, fuentes web, importación
renderer/     el reproductor (Rust + GStreamer)
extension/    la extensión de GNOME Shell (JavaScript) y el esquema de ajustes
scripts/      fondo-animado, -convertir, -sin-audio, -miniatura
data/         lanzador .desktop e icono
herramientas/ empaquetado y publicación de releases
```

## Media Controller

La extensión de controles multimedia (fork de
[Advanced Media Controller](https://codeberg.org/sanjai-shaarugesh/Advanced-media-controller))
**se ha movido a su propio repo**, con todo su historial:
[jlcampana/gnome-extension-mediaplayer](https://github.com/jlcampana/gnome-extension-mediaplayer). Este instalador ya no la instala ni la toca;
si ya la tenías, sigue funcionando. Para instalarla o actualizarla:

```bash
curl -fsSL https://raw.githubusercontent.com/jlcampana/gnome-extension-mediaplayer/main/install.sh | bash
```

## Licencia

MIT. Los fondos que se descargan son de sus autores y de cada web; respeta sus condiciones.
